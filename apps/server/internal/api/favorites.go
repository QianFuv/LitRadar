package api

import (
	"context"
	"errors"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/citation"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
)

type favoriteHandlers struct {
	repository    *favorites.Repository
	storage       config.Config
	authenticator *Authenticator
	pool          *executor.Pool
}
type okResponse struct {
	// OK reports whether the operation succeeded.
	OK bool `json:"ok" nullable:"false"`
}
type countResponse struct {
	// Count describes number of affected favorite rows.
	Count int64 `json:"count" nullable:"false" format:"int64"`
}
type addedResponse struct {
	// Added describes number of inserted favorite rows.
	Added int64 `json:"added" nullable:"false" format:"int64"`
}
type trackingFolderResponse struct {
	// FolderID describes folder row identifier.
	FolderID *int64 `json:"folder_id" required:"false" nullable:"true" format:"int64"`
	// FolderName describes folder display name.
	FolderName *string `json:"folder_name" required:"false" nullable:"true"`
}
type exportOutput struct {
	// Body contains the decoded operation payload.
	Body []byte `format:"binary" encoding:"binary"`
	// ContentType sets the Content-Type header.
	ContentType string `header:"Content-Type"`
	// ContentDisposition sets the Content-Disposition header.
	ContentDisposition string `header:"Content-Disposition"`
}

func changedResponse(changed bool, err error, detail string) (okResponse, error) {
	if err != nil {
		return okResponse{}, err
	}
	if !changed {
		return okResponse{}, &apiError{status: 404, detail: detail}
	}
	return okResponse{OK: true}, nil
}
func (handlers *favoriteHandlers) exportFolder(ctx context.Context, input *ExportFolderInput) (*exportOutput, error) {
	current, failure := handlers.authenticator.requireUser(input.request)
	if failure != nil {
		return nil, failure.problem()
	}
	value, err := admitted(ctx, handlers.pool, func() (favoriteExport, error) {
		return handlers.export(context.WithoutCancel(ctx), current.authorization.User.Id, input.FolderID, input.Format)
	}, mapBusinessError)
	if err != nil {
		return nil, err
	}
	return &exportOutput{Body: []byte(value.Body.content), ContentType: value.Body.media, ContentDisposition: "attachment; filename=\"" + value.Body.filename + "\""}, nil
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

func (handlers *favoriteHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/folders", ID: "list_folders"}, func(ctx context.Context, input *ListFoldersInput) (*output[[]favorites.Folder], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() ([]favorites.Folder, error) { return handlers.repository.ListFolders(workContext, owner) }, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/favorites/folders", ID: "create_folder"}, func(ctx context.Context, input *CreateFolderInput) (*output[favorites.Folder], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			if failure := validateFolderName(input.Body.Name); failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (favorites.Folder, error) {
				return handlers.repository.CreateFolder(workContext, owner, input.Body.Name, input.Body.IsTracking)
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/favorites/folders/{folder_id}", ID: "rename_folder"}, func(ctx context.Context, input *RenameFolderInput) (*output[okResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			if failure := validateFolderName(input.Body.Name); failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (okResponse, error) {
				changed, err := handlers.repository.RenameFolder(workContext, owner, input.FolderID, input.Body.Name)
				return changedResponse(changed, err, "Folder not found")
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/favorites/folders/{folder_id}", ID: "delete_folder"}, func(ctx context.Context, input *DeleteFolderInput) (*output[okResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (okResponse, error) {
				changed, err := handlers.repository.DeleteFolder(workContext, owner, input.FolderID)
				return changedResponse(changed, err, "Folder not found")
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/tracking", ID: "get_tracking"}, func(ctx context.Context, input *GetTrackingInput) (*output[trackingFolderResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (trackingFolderResponse, error) {
				selected, err := handlers.repository.TrackingFolder(workContext, owner)
				if err != nil {
					return trackingFolderResponse{}, err
				}
				result := trackingFolderResponse{}
				if selected != nil {
					result.FolderID = &selected.Id
					result.FolderName = &selected.Name
				}
				return result, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/favorites/tracking", ID: "set_tracking"}, func(ctx context.Context, input *SetTrackingInput) (*output[okResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (okResponse, error) {
				changed, err := handlers.repository.SetTrackingFolder(workContext, owner, input.Body.FolderID)
				return changedResponse(changed, err, "Folder not found")
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/folders/{folder_id}/count", ID: "folder_count"}, func(ctx context.Context, input *FolderCountInput) (*output[countResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (countResponse, error) {
				count, err := handlers.repository.CountFavorites(workContext, owner, &input.FolderID)
				return countResponse{Count: count}, err
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/folders/{folder_id}/articles", ID: "list_folder_articles"}, func(ctx context.Context, input *ListFolderArticlesInput) (*output[[]favorites.Article], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() ([]favorites.Article, error) {
				rows, err := handlers.repository.ListFavorites(workContext, owner, &input.FolderID, input.Limit, input.Offset)
				if err != nil {
					return nil, err
				}
				return favorites.Enrich(workContext, handlers.storage, rows), nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/folders/{folder_id}/articles/page", ID: "list_folder_article_page"}, func(ctx context.Context, input *ListFolderArticlePageInput) (*output[domain.Page[favorites.Article]], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (domain.Page[favorites.Article], error) {
				return handlers.repository.ArticlePage(workContext, handlers.storage, owner, input.FolderID, input.Limit, cleanText(input.Cursor.Pointer()))
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles", ID: "add_favorite"}, func(ctx context.Context, input *AddFavoriteInput) (*output[favorites.Favorite], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (favorites.Favorite, error) {
				return handlers.repository.AddFavorite(workContext, owner, input.FolderID, favorites.Add{Reference: favorites.Reference{ArticleId: input.Body.ArticleID, DbName: input.Body.DBName}, Note: input.Body.Note})
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/favorites/folders/{folder_id}/articles/{article_id}", ID: "remove_favorite"}, func(ctx context.Context, input *RemoveFavoriteInput) (*output[okResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (okResponse, error) {
				changed, err := handlers.repository.RemoveFavorite(workContext, owner, input.FolderID, favorites.Reference{ArticleId: identity.Id(input.ArticleID), DbName: textOr(input.DBName.Pointer(), "")})
				return changedResponse(changed, err, "Favorite not found")
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/check", ID: "check_favorite"}, func(ctx context.Context, input *CheckFavoriteInput) (*output[[]favorites.Membership], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() ([]favorites.Membership, error) {
				return handlers.repository.IsFavorited(workContext, owner, favorites.Reference{ArticleId: identity.Id(input.ArticleID), DbName: textOr(input.DBName.Pointer(), "")})
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/favorites/check/batch", ID: "check_favorites_batch"}, func(ctx context.Context, input *CheckFavoritesBatchInput) (*output[[]favorites.BatchMembership], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() ([]favorites.BatchMembership, error) {
				ids := make([]int64, len(input.Body.ArticleIDs))
				for index, id := range input.Body.ArticleIDs {
					ids[index] = int64(id)
				}
				return handlers.repository.BatchIsFavorited(workContext, owner, ids, input.Body.DBName)
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles/bulk", ID: "bulk_add"}, func(ctx context.Context, input *BulkAddInput) (*output[addedResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (addedResponse, error) {
				items := make([]favorites.Add, len(input.Body.Articles))
				for index, item := range input.Body.Articles {
					items[index] = favorites.Add{Reference: favorites.Reference{ArticleId: item.ArticleID, DbName: item.DBName}, Note: item.Note}
				}
				count, err := handlers.repository.BulkAdd(workContext, owner, input.FolderID, items)
				return addedResponse{Added: count}, err
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles/bulk-remove", ID: "bulk_remove"}, func(ctx context.Context, input *BulkRemoveInput) (*output[countResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (countResponse, error) {
				items := make([]favorites.Reference, len(input.Body.Articles))
				for index, item := range input.Body.Articles {
					items[index] = favorites.Reference{ArticleId: item.ArticleID, DbName: item.DBName}
				}
				count, err := handlers.repository.BulkRemove(workContext, owner, input.FolderID, items)
				return countResponse{Count: count}, err
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/favorites/folders/{folder_id}/articles/bulk-move", ID: "bulk_move"}, func(ctx context.Context, input *BulkMoveInput) (*output[countResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (countResponse, error) {
				items := make([]favorites.Reference, len(input.Body.Articles))
				for index, item := range input.Body.Articles {
					items[index] = favorites.Reference{ArticleId: item.ArticleID, DbName: item.DBName}
				}
				count, err := handlers.repository.BulkMove(workContext, owner, input.FolderID, input.Body.TargetFolderID, items)
				return countResponse{Count: count}, err
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/folders/{folder_id}/export", ID: "export_folder"}, handlers.exportFolder),
	}
}

type favoriteExport struct{ content, filename, media string }

// export preserves snapshot limits, metadata batches and output-limit projection.
func (handlers *favoriteHandlers) export(ctx context.Context, owner identity.Id, folder int64, format string) (favoriteExport, error) {
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

// validateFolderName rejects whitespace-only labels after Huma validates raw length.
func validateFolderName(value string) *apiError {
	value = strings.TrimSpace(value)
	if value == "" {
		return badRequest("Folder name must be 1-100 characters")
	}
	return nil
}
