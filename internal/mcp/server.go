// Package mcp binds LitRadar's public tools to the migrated application services.
package mcp

import (
	"bytes"
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"time"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/platform/mcpcompat"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/query"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	sdk "github.com/modelcontextprotocol/go-sdk/mcp"
)

//go:embed tools.json
var declarations []byte

// Services shares initialized repositories and the API storage admission limit.
type Services struct {
	Storage   config.Config
	Favorites *favorites.Repository
	Pool      *executor.Pool
}

type toolDeclaration struct {
	Name        string
	Description string
	InputSchema json.RawMessage
}

// NewServer installs every declared tool with explicit schema and text-only results.
// The caller owns HTTP authorization, protocol compatibility and server lifetime.
func NewServer(services Services) (*sdk.Server, error) {
	var tools []toolDeclaration
	if err := json.Unmarshal(declarations, &tools); err != nil {
		return nil, err
	}
	server := sdk.NewServer(&sdk.Implementation{Name: "rmcp", Version: "2.1.0"}, nil)
	for _, tool := range tools {
		var schema toolSchema
		if err := json.Unmarshal(tool.InputSchema, &schema); err != nil {
			return nil, err
		}
		if !knownTool(tool.Name) {
			return nil, fmt.Errorf("unbound MCP tool %q", tool.Name)
		}
		server.AddTool(&sdk.Tool{Name: tool.Name, Description: tool.Description, InputSchema: tool.InputSchema}, func(ctx context.Context, request *sdk.CallToolRequest) (*sdk.CallToolResult, error) {
			input, err := decodeInput(request.Params.Arguments, schema)
			if err != nil {
				return toolError("failed to deserialize parameters: " + err.Error()), nil
			}
			return services.call(ctx, request, tool.Name, input)
		})
	}
	return server, nil
}

func knownTool(name string) bool {
	switch name {
	case "list_databases", "list_areas", "list_journal_ratings", "list_years", "list_journal_options", "list_journals", "get_journal", "search_articles", "get_article", "get_weekly_updates", "list_folders", "add_favorite", "remove_favorite":
		return true
	default:
		return false
	}
}

func (services Services) call(ctx context.Context, request *sdk.CallToolRequest, name string, input *toolInput) (*sdk.CallToolResult, error) {
	work, err := services.toolWork(ctx, request, name, input)
	if err != nil {
		return nil, err
	}
	if input.err != nil {
		return toolError(input.err.Error()), nil
	}
	type outcome struct {
		payload any
		err     error
	}
	result, err := executor.Run(ctx, services.Pool, func() (outcome, error) { payload, err := work(); return outcome{payload, err}, nil })
	if err != nil {
		return nil, &jsonrpc.Error{Code: -32603, Message: "LitRadar backend is temporarily unavailable"}
	}
	if result.err != nil {
		return toolError(publicError(result.err)), nil
	}
	text, err := encodeToolPayload(result.payload)
	if err != nil {
		return nil, &jsonrpc.Error{Code: -32603, Message: "Failed to serialize MCP tool response"}
	}
	return &sdk.CallToolResult{Content: []sdk.Content{&sdk.TextContent{Text: text}}}, nil
}

// toolWork resolves parameters before storage admission and selects the tool operation.
func (services Services) toolWork(ctx context.Context, request *sdk.CallToolRequest, name string, input *toolInput) (func() (any, error), error) {
	switch name {
	case "list_databases", "get_weekly_updates", "list_areas", "list_journal_ratings", "list_years", "list_journal_options":
		return services.metadataWork(ctx, name, input), nil
	case "list_journals", "search_articles":
		return services.searchWork(ctx, name, input), nil
	case "get_article", "get_journal":
		return services.recordWork(ctx, name, input), nil
	case "list_folders", "add_favorite", "remove_favorite":
		return services.favoriteWork(ctx, request, name, input)
	}
	return nil, nil
}

// metadataWork captures database validation while keeping weekly time inside the worker.
func (services Services) metadataWork(workContext context.Context, name string, input *toolInput) func() (any, error) {
	var work func() (any, error)
	switch name {
	case "list_databases":
		work = func() (any, error) {
			paths, err := services.Storage.ListIndexDatabases()
			if err != nil {
				return nil, err
			}
			for index, path := range paths {
				paths[index] = filepath.Base(path)
			}
			return paths, nil
		}
	case "get_weekly_updates":
		work = func() (any, error) {
			return query.WeeklyUpdates(workContext, services.Storage, weekly.FromTime(time.Now()))
		}
	case "list_areas", "list_journal_ratings", "list_years", "list_journal_options":
		database := input.text("db")
		work = func() (any, error) {
			switch name {
			case "list_areas":
				return query.ListAreas(workContext, services.Storage, database)
			case "list_journal_ratings":
				return query.ListJournalRatings(workContext, services.Storage, database)
			case "list_years":
				return query.ListYears(workContext, services.Storage, database)
			default:
				return query.ListJournalOptions(workContext, services.Storage, database)
			}
		}
	}
	return work
}

// searchWork captures journal or article search parameters before execution.
func (services Services) searchWork(workContext context.Context, name string, input *toolInput) func() (any, error) {
	var work func() (any, error)
	switch name {
	case "list_journals":
		database, params := input.journals()
		work = func() (any, error) { return query.ListJournals(workContext, services.Storage, database, params) }
	case "search_articles":
		database, params := input.articles()
		work = func() (any, error) { return query.ListArticles(workContext, services.Storage, database, params) }
	}
	return work
}

// recordWork validates the database before the requested record identifier.
func (services Services) recordWork(workContext context.Context, name string, input *toolInput) func() (any, error) {
	var work func() (any, error)
	database := input.text("db")
	field := "article_id"
	if name == "get_journal" {
		field = "journal_id"
	}
	id := input.positiveId(field, input.fields[field].(string))
	work = func() (any, error) {
		if name == "get_article" {
			return query.GetArticle(workContext, services.Storage, database, id)
		}
		return query.GetJournal(workContext, services.Storage, database, id)
	}
	return work
}

// favoriteWork uses the current authenticated principal and detaches only mutations.
func (services Services) favoriteWork(ctx context.Context, request *sdk.CallToolRequest, name string, input *toolInput) (func() (any, error), error) {
	var work func() (any, error)
	workContext := ctx
	principal, ok := mcpcompat.PrincipalFor(request)
	if !ok {
		return nil, &jsonrpc.Error{Code: -32603, Message: "Authenticated MCP user is missing"}
	}
	if name == "list_folders" {
		work = func() (any, error) { return services.Favorites.ListFolders(workContext, principal.UserId) }
		return work, nil
	}
	workContext = context.WithoutCancel(ctx)
	folder := *input.integer("folder_id")
	if folder <= 0 {
		input.fail("folder_id must be a positive integer")
	}
	article := input.positiveId("article_id", input.fields["article_id"].(string))
	database := ""
	if value := input.text("db_name"); value != nil {
		database = *value
	}
	reference := favorites.Reference{ArticleId: identity.Id(article), DbName: database}
	work = func() (any, error) {
		if name == "add_favorite" {
			return services.Favorites.AddFavorite(workContext, principal.UserId, folder, favorites.Add{Reference: reference})
		}
		didRemove, err := services.Favorites.RemoveFavorite(workContext, principal.UserId, folder, reference)
		if err != nil {
			return nil, err
		}
		if !didRemove {
			return nil, errFavoriteNotFound
		}
		return map[string]bool{"ok": true}, nil
	}
	return work, nil
}

func encodeToolPayload(payload any) (string, error) {
	type folderWire struct {
		favorites.Folder
		CreatedAt json.RawMessage `json:"created_at"`
	}
	type favoriteWire struct {
		favorites.Favorite
		CreatedAt json.RawMessage `json:"created_at"`
	}
	switch value := payload.(type) {
	case []favorites.Folder:
		rows := make([]folderWire, len(value))
		for index, folder := range value {
			encoded, err := sources.Json(folder.CreatedAt)
			if err != nil {
				return "", err
			}
			rows[index] = folderWire{folder, encoded}
		}
		payload = rows
	case favorites.Favorite:
		encoded, err := sources.Json(value.CreatedAt)
		if err != nil {
			return "", err
		}
		payload = favoriteWire{value, encoded}
	}
	encoded, err := jsonvalue.EncodeJson(payload)
	var pretty bytes.Buffer
	if err == nil {
		err = json.Indent(&pretty, []byte(encoded), "", "  ")
	}
	return pretty.String(), err
}

var errFavoriteNotFound = errors.New("Favorite not found")

func toolError(message string) *sdk.CallToolResult {
	return &sdk.CallToolResult{IsError: true, Content: []sdk.Content{&sdk.TextContent{Text: message}}}
}

func publicError(err error) string {
	var invalid query.InvalidInput
	var missing query.NotFound
	var sort query.UnsupportedSortField
	var business favorites.InvalidInput
	if errors.As(err, &invalid) || errors.As(err, &missing) || errors.As(err, &sort) || errors.As(err, &business) {
		return err.Error()
	}
	for _, known := range []error{config.ErrNoDatabases, config.ErrNotFound, config.ErrMultipleDatabases, config.ErrInvalidName, query.ErrInvalidCursor, query.ErrInvalidSearchExpression, query.ErrUnsupportedArticleSort, query.ErrLegacyWeeklyLimit, favorites.ErrDuplicateFolder, favorites.ErrFolderNotFound, favorites.ErrSourceFolderNotFound, favorites.ErrTargetFolderNotFound, favorites.ErrSameFolders, favorites.ErrCursor, errFavoriteNotFound} {
		if errors.Is(err, known) {
			return known.Error()
		}
	}
	return "Internal Server Error"
}
