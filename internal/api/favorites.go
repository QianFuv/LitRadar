package api

import (
	"context"
	"errors"
	"net/http"
	"strings"

	"github.com/QianFuv/LitRadar/internal/citation"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
)

type favoriteHandlers struct {
	repository    *favorites.Repository
	storage       config.Config
	authenticator *Authenticator
	pool          *executor.Pool
}

var favoriteBodies = map[string]string{"create_folder": "FolderCreate", "rename_folder": "FolderRename", "set_tracking": "TrackingSetRequest", "add_favorite": "FavoriteAdd", "bulk_add": "FavoriteBulkAdd", "bulk_remove": "FavoriteBulkRemove", "bulk_move": "FavoriteBulkMove", "check_favorites_batch": "FavoriteBatchCheckRequest"}

func (handlers *favoriteHandlers) routes() []route {
	routes := []route{}
	for _, operation := range []openapi.Operation{
		{Method: "GET", Path: "/api/favorites/folders", Id: "list_folders"},
		{Method: "POST", Path: "/api/favorites/folders", Id: "create_folder"},
		{Method: "PUT", Path: "/api/favorites/folders/{folder_id}", Id: "rename_folder"},
		{Method: "DELETE", Path: "/api/favorites/folders/{folder_id}", Id: "delete_folder"},
		{Method: "GET", Path: "/api/favorites/tracking", Id: "get_tracking"},
		{Method: "PUT", Path: "/api/favorites/tracking", Id: "set_tracking"},
		{Method: "GET", Path: "/api/favorites/folders/{folder_id}/articles", Id: "list_folder_articles"},
		{Method: "GET", Path: "/api/favorites/folders/{folder_id}/articles/page", Id: "list_folder_article_page"},
		{Method: "GET", Path: "/api/favorites/folders/{folder_id}/count", Id: "folder_count"},
		{Method: "GET", Path: "/api/favorites/folders/{folder_id}/export", Id: "export_folder"},
		{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles", Id: "add_favorite"},
		{Method: "DELETE", Path: "/api/favorites/folders/{folder_id}/articles/{article_id}", Id: "remove_favorite"},
		{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles/bulk", Id: "bulk_add"},
		{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles/bulk-remove", Id: "bulk_remove"},
		{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles/bulk-move", Id: "bulk_move"},
		{Method: "GET", Path: "/api/favorites/check", Id: "check_favorite"},
		{Method: "POST", Path: "/api/favorites/check/batch", Id: "check_favorites_batch"},
	} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) {
			handlers.handle(writer, request, operation.Id)
		}})
	}
	return routes
}

func mapBusinessError(err error) *apiError {
	var failure *apiError
	if errors.As(err, &failure) {
		return failure
	}
	var invalid favorites.InvalidInput
	switch {
	case errors.As(err, &invalid), errors.Is(err, favorites.ErrSameFolders), errors.Is(err, favorites.ErrCursor):
		return badRequest(err.Error())
	case errors.Is(err, favorites.ErrDuplicateFolder):
		return &apiError{status: 409, detail: err.Error()}
	case errors.Is(err, favorites.ErrFolderNotFound), errors.Is(err, favorites.ErrSourceFolderNotFound), errors.Is(err, favorites.ErrTargetFolderNotFound):
		return &apiError{status: 404, detail: err.Error()}
	default:
		return internalError()
	}
}

// handle retains syntax extraction, authentication, validation and executor admission order.
func (handlers *favoriteHandlers) handle(writer http.ResponseWriter, request *http.Request, name string) {
	input, failure := extractFavoriteRequest(request, name)
	if failure != nil {
		failure.write(writer)
		return
	}
	folder, article, query, body := input.folder, input.article, input.query, input.body
	current, failure := handlers.authenticator.requireUser(request)
	if failure != nil {
		failure.write(writer)
		return
	}
	if failure := validateFavoriteRequest(name, folder, article, query, body); failure != nil {
		failure.write(writer)
		return
	}
	owner := current.authorization.User.Id
	type result struct {
		payload any
		err     error
	}
	value, err := executor.Run(request.Context(), handlers.pool, func() (result, error) {
		payload, err := handlers.perform(context.WithoutCancel(request.Context()), name, owner, folder, article, query, body)
		return result{payload, err}, nil
	})
	if err != nil {
		mapExecutorError(err).write(writer)
		return
	}
	if value.err != nil {
		mapBusinessError(value.err).write(writer)
		return
	}
	if export, ok := value.payload.(favoriteExport); ok {
		writer.Header().Set("Content-Type", export.media)
		writer.Header().Set("Content-Disposition", "attachment; filename=\""+export.filename+"\"")
		writer.WriteHeader(200)
		_, _ = writer.Write([]byte(export.content))
		return
	}
	writeResponse(writer, value.payload)
}

type favoriteRequest struct {
	folder, article int64
	query           typedQuery
	body            map[string]any
}

// extractFavoriteRequest preserves path, query, required-field and body rejection before authentication.
func extractFavoriteRequest(request *http.Request, name string) (favoriteRequest, *apiError) {
	var folder, article int64
	var failure *apiError
	folder, article, failure = extractFavoritePaths(request, name)
	if failure != nil {
		return favoriteRequest{}, failure
	}
	fields := favoriteQueryFields(name)
	query, failure := extractQuery(request.URL.RawQuery, fields)
	if failure != nil {
		return favoriteRequest{}, failure
	}
	if name == "check_favorite" && query.integer("article_id") == nil {
		return favoriteRequest{}, queryRejection("missing field `article_id`")
	}
	var body map[string]any
	if kind, exists := favoriteBodies[name]; exists {
		decoded, failure := extractBody(request, requestBodies[kind], false)
		if failure != nil {
			return favoriteRequest{}, failure
		}
		body = decoded.(map[string]any)
	}
	return favoriteRequest{folder, article, query, body}, nil
}

// extractFavoritePaths retains tuple errors for the two remove-favorite captures.
func extractFavoritePaths(request *http.Request, name string) (int64, int64, *apiError) {
	var folder, article int64
	var failure *apiError
	if request.PathValue("folder_id") != "" {
		folder, failure = extractPathInteger(request, "folder_id")
		if name == "remove_favorite" {
			folder, failure = extractTuplePathInteger(request, "folder_id", 0)
		}
		if failure != nil {
			return folder, article, failure
		}
	}
	if name == "remove_favorite" {
		article, failure = extractTuplePathInteger(request, "article_id", 1)
		if failure != nil {
			return folder, article, failure
		}
	}
	return folder, article, nil
}

// favoriteQueryFields selects the existing endpoint-specific scalar query schema.
func favoriteQueryFields(name string) map[string]queryKind {
	fields := map[string]queryKind{}
	switch name {
	case "list_folder_articles":
		fields = map[string]queryKind{"limit": queryInteger, "offset": queryInteger}
	case "list_folder_article_page":
		fields = map[string]queryKind{"limit": queryInteger, "cursor": queryText}
	case "remove_favorite":
		fields = map[string]queryKind{"db_name": queryText}
	case "check_favorite":
		fields = map[string]queryKind{"article_id": queryInteger, "db_name": queryText}
	case "export_folder":
		fields = map[string]queryKind{"format": queryText}
	}
	return fields
}

// perform separates folder operations from article operations without changing ownership.
func (handlers *favoriteHandlers) perform(ctx context.Context, name string, owner identity.Id, folder, article int64, query typedQuery, body map[string]any) (any, error) {
	repository := handlers.repository
	switch name {
	case "list_folders":
		return repository.ListFolders(ctx, owner)
	case "create_folder":
		return repository.CreateFolder(ctx, owner, body["name"].(string), body["is_tracking"].(bool))
	case "rename_folder":
		changed, err := repository.RenameFolder(ctx, owner, folder, body["name"].(string))
		return favoriteChanged(changed, err, "Folder not found")
	case "delete_folder":
		changed, err := repository.DeleteFolder(ctx, owner, folder)
		return favoriteChanged(changed, err, "Folder not found")
	case "get_tracking":
		return handlers.trackingFolder(ctx, owner)
	case "set_tracking":
		changed, err := repository.SetTrackingFolder(ctx, owner, body["folder_id"].(int64))
		return favoriteChanged(changed, err, "Folder not found")
	case "folder_count":
		count, err := repository.CountFavorites(ctx, owner, &folder)
		return map[string]int64{"count": count}, err
	default:
		return handlers.performFavoriteArticles(ctx, name, owner, folder, article, query, body)

	}
}

// favoriteChanged maps unchanged repository mutations to the original missing-target error.
func favoriteChanged(changed bool, err error, detail string) (any, error) {
	if err != nil {
		return nil, err
	}
	if !changed {
		return nil, &apiError{status: 404, detail: detail}
	}
	return map[string]bool{"ok": true}, nil
}

// trackingFolder projects the current optional tracking folder without changing repository ownership.
func (handlers *favoriteHandlers) trackingFolder(ctx context.Context, owner identity.Id) (any, error) {
	repository := handlers.repository
	selected, err := repository.TrackingFolder(ctx, owner)
	if err != nil {
		return nil, err
	}
	result := struct {
		FolderId   *int64  `json:"folder_id"`
		FolderName *string `json:"folder_name"`
	}{nil, nil}
	if selected != nil {
		result.FolderId, result.FolderName = &selected.Id, &selected.Name
	}
	return result, nil
}

// performFavoriteArticles executes single-article, page, bulk and export operations.
func (handlers *favoriteHandlers) performFavoriteArticles(ctx context.Context, name string, owner identity.Id, folder, article int64, query typedQuery, body map[string]any) (any, error) {
	repository := handlers.repository
	db := ""
	if value := query.text("db_name"); value != nil {
		db = *value
	}
	switch name {
	case "list_folder_articles":
		return handlers.listFavoriteArticles(ctx, owner, folder, query)
	case "list_folder_article_page":
		limit := query.integerDefault("limit", 50)
		return repository.ArticlePage(ctx, handlers.storage, owner, folder, limit, query.text("cursor"))
	case "add_favorite":
		return repository.AddFavorite(ctx, owner, folder, favoriteAdd(body))
	case "remove_favorite":
		changed, err := repository.RemoveFavorite(ctx, owner, folder, favorites.Reference{ArticleId: identity.Id(article), DbName: db})
		return favoriteChanged(changed, err, "Favorite not found")
	case "check_favorite":
		return repository.IsFavorited(ctx, owner, favorites.Reference{ArticleId: identity.Id(*query.integer("article_id")), DbName: db})
	case "check_favorites_batch":
		return handlers.checkFavoriteBatch(ctx, owner, body)
	case "bulk_add", "bulk_remove", "bulk_move":
		return handlers.performFavoriteBulk(ctx, name, owner, folder, body)
	case "export_folder":
		return handlers.export(ctx, owner, folder, query.text("format"))
	}
	return nil, errors.New("unknown favorite operation")
}

// listFavoriteArticles retains the list limit and offset defaults before enrichment.
func (handlers *favoriteHandlers) listFavoriteArticles(ctx context.Context, owner identity.Id, folder int64, query typedQuery) (any, error) {
	repository := handlers.repository
	limit, offset := query.integerDefault("limit", 100), query.integerDefault("offset", 0)
	rows, err := repository.ListFavorites(ctx, owner, &folder, limit, offset)
	if err != nil {
		return nil, err
	}
	return favorites.Enrich(ctx, handlers.storage, rows), nil
}

// checkFavoriteBatch converts every decoded identity before business-layer batch validation.
func (handlers *favoriteHandlers) checkFavoriteBatch(ctx context.Context, owner identity.Id, body map[string]any) (any, error) {
	repository := handlers.repository
	values := body["article_ids"].([]any)
	ids := make([]int64, len(values))
	for index, value := range values {
		ids[index] = value.(int64)
	}
	return repository.BatchIsFavorited(ctx, owner, ids, body["db_name"].(string))
}

// performFavoriteBulk retains ordered item conversion and the distinct bulk result envelopes.
func (handlers *favoriteHandlers) performFavoriteBulk(ctx context.Context, name string, owner identity.Id, folder int64, body map[string]any) (any, error) {
	repository := handlers.repository
	switch name {
	case "bulk_add":
		values := body["articles"].([]any)
		items := make([]favorites.Add, len(values))
		for index, value := range values {
			items[index] = favoriteAdd(value.(map[string]any))
		}
		count, err := repository.BulkAdd(ctx, owner, folder, items)
		return map[string]int64{"added": count}, err
	case "bulk_remove", "bulk_move":
		values := body["articles"].([]any)
		items := make([]favorites.Reference, len(values))
		for index, value := range values {
			record := value.(map[string]any)
			items[index] = favorites.Reference{ArticleId: identity.Id(record["article_id"].(int64)), DbName: record["db_name"].(string)}
		}
		var count int64
		var err error
		if name == "bulk_remove" {
			count, err = repository.BulkRemove(ctx, owner, folder, items)
		} else {
			count, err = repository.BulkMove(ctx, owner, folder, body["target_folder_id"].(int64), items)
		}
		return map[string]int64{"count": count}, err
	}
	return nil, errors.New("unknown favorite operation")
}

func favoriteAdd(body map[string]any) favorites.Add {
	return favorites.Add{Reference: favorites.Reference{ArticleId: identity.Id(body["article_id"].(int64)), DbName: body["db_name"].(string)}, Note: body["note"].(string)}
}

type favoriteExport struct{ content, filename, media string }

// export preserves snapshot limits, metadata batches and output-limit projection.
func (handlers *favoriteHandlers) export(ctx context.Context, owner identity.Id, folder int64, requested *string) (favoriteExport, error) {
	format := "bibtex"
	if requested != nil {
		format = *requested
	}
	selected, failure := selectFavoriteCitationFormat(format)
	if failure != nil {
		return favoriteExport{}, failure
	}
	snapshot, err := handlers.repository.LoadCitationSnapshot(ctx, owner, folder, 10000)
	if err != nil {
		return favoriteExport{}, err
	}
	if snapshot.HasMore {
		return favoriteExport{}, &apiError{status: 413, detail: "Favorite export supports at most 10000 items"}
	}
	records := make([]domain.FavoriteCitation, 0, len(snapshot.References))
	for start := 0; start < len(snapshot.References); start += 250 {
		batch, err := favorites.LoadCitationRecords(ctx, handlers.storage, snapshot.References[start:min(start+250, len(snapshot.References))])
		if err != nil {
			return favoriteExport{}, err
		}
		records = append(records, batch...)
	}
	content, err := selected.serialize(records, 8*1024*1024)
	if err != nil {
		if errors.Is(err, citation.ErrOutputLimit) {
			return favoriteExport{}, &apiError{status: 413, detail: "Favorite export exceeds the 8 MiB output limit"}
		}
		return favoriteExport{}, err
	}
	return favoriteExport{content, exportFilename(snapshot.FolderName, selected.extension), selected.media}, nil
}
func exportFilename(name, extension string) string {
	sanitized := strings.Map(func(character rune) rune {
		if character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("._-", character) {
			return character
		}
		return '_'
	}, name)
	sanitized = strings.Trim(sanitized, "._")
	if sanitized == "" {
		sanitized = "favorites"
	}
	return sanitized + "." + extension
}

type favoriteCitationFormat struct {
	serialize        func([]domain.FavoriteCitation, int) (string, error)
	extension, media string
}

// selectFavoriteCitationFormat rejects unsupported formats before any metadata or snapshot access.
func selectFavoriteCitationFormat(format string) (favoriteCitationFormat, *apiError) {
	var serialize func([]domain.FavoriteCitation, int) (string, error)
	extension, media := "", ""
	switch format {
	case "bibtex":
		serialize, extension, media = citation.Bibtex, "bib", "application/x-bibtex"
	case "ris":
		serialize, extension, media = citation.Ris, "ris", "application/x-research-info-systems"
	case "endnote":
		serialize, extension, media = citation.EndnoteXml, "xml", "application/xml"
	default:
		return favoriteCitationFormat{}, badRequest("Invalid export format")
	}
	return favoriteCitationFormat{serialize, extension, media}, nil
}
