package api

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
)

var graphCreateBody = structBody("RelationGraphFavoriteCreate",
	bodyFieldOf("title", stringBody), bodyFieldOf("db_name", stringBody),
	bodyFieldOf("root_article_id", integerBody), bodyFieldOf("node_count", integerBody),
	bodyFieldOf("graph", bodyType{kind: "raw"}))

var graphUpdateBody = structBody("RelationGraphFavoriteUpdate",
	defaultBodyField("title", stringBody, nil), defaultBodyField("node_count", integerBody, nil),
	defaultBodyField("graph", bodyType{kind: "raw"}, nil))

func (handlers *favoriteHandlers) graphRoutes() []route {
	routes := []route{}
	for _, operation := range []openapi.Operation{
		{Method: "GET", Path: "/api/favorites/graphs", Id: "list_relation_graph_favorites"},
		{Method: "POST", Path: "/api/favorites/graphs", Id: "create_relation_graph_favorite"},
		{Method: "GET", Path: "/api/favorites/graphs/{graph_id}", Id: "get_relation_graph_favorite"},
		{Method: "PUT", Path: "/api/favorites/graphs/{graph_id}", Id: "update_relation_graph_favorite"},
		{Method: "DELETE", Path: "/api/favorites/graphs/{graph_id}", Id: "delete_relation_graph_favorite"},
	} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) {
			handlers.handleGraph(writer, request, operation.Id)
		}})
	}
	return routes
}

type graphRequest struct {
	id     int64
	query  typedQuery
	create favorites.GraphCreate
	update favorites.GraphUpdate
}

func extractGraphRequest(request *http.Request, name string) (graphRequest, *apiError) {
	var input graphRequest
	if request.PathValue("graph_id") != "" {
		id, failure := extractPathInteger(request, "graph_id")
		if failure != nil {
			return input, failure
		}
		input.id = id
	}
	if name == "list_relation_graph_favorites" {
		query, failure := extractQuery(request.URL.RawQuery, map[string]queryKind{"limit": queryInteger, "cursor": queryText})
		input.query = query
		return input, failure
	}
	if name != "create_relation_graph_favorite" && name != "update_relation_graph_favorite" {
		return input, nil
	}
	kind := graphCreateBody
	if name == "update_relation_graph_favorite" {
		kind = graphUpdateBody
	}
	decoded, failure := extractBodyWithLimit(request, kind, false, 3*1024*1024)
	if failure != nil {
		return input, failure
	}
	body := decoded.(map[string]any)
	if name == "create_relation_graph_favorite" {
		input.create = favorites.GraphCreate{Title: body["title"].(string), DbName: body["db_name"].(string), RootArticleId: body["root_article_id"].(int64), NodeCount: body["node_count"].(int64), Graph: body["graph"].(json.RawMessage)}
	} else {
		if title, ok := body["title"].(string); ok {
			input.update.Title = &title
		}
		if count, ok := body["node_count"].(int64); ok {
			input.update.NodeCount = &count
		}
		input.update.Graph, _ = body["graph"].(json.RawMessage)
	}
	return input, nil
}

func validateGraphRequest(name string, input graphRequest) *apiError {
	var err error
	switch name {
	case "list_relation_graph_favorites":
		limit := input.query.integerDefault("limit", 50)
		if limit < 1 || limit > 100 {
			return badRequest("limit must be between 1 and 100")
		}
	case "create_relation_graph_favorite":
		err = input.create.Validate()
	case "update_relation_graph_favorite":
		err = input.update.Validate()
	}
	graph := input.create.Graph
	if name == "update_relation_graph_favorite" {
		graph = input.update.Graph
	}
	if len(graph) > favorites.MaxGraphBytes {
		return &apiError{status: 413, detail: "graph must be at most 2 MiB"}
	}
	if err != nil {
		return mapBusinessError(err)
	}
	switch name {
	case "get_relation_graph_favorite", "update_relation_graph_favorite", "delete_relation_graph_favorite":
		return positiveFavoriteId("graph_id", input.id)
	}
	return nil
}

func (handlers *favoriteHandlers) handleGraph(writer http.ResponseWriter, request *http.Request, name string) {
	input, failure := extractGraphRequest(request, name)
	if failure != nil {
		failure.write(writer)
		return
	}
	current, failure := handlers.authenticator.requireUser(request)
	if failure != nil {
		failure.write(writer)
		return
	}
	if failure := validateGraphRequest(name, input); failure != nil {
		failure.write(writer)
		return
	}
	type result struct {
		payload any
		err     error
	}
	value, err := executor.Run(request.Context(), handlers.pool, func() (result, error) {
		payload, err := handlers.performGraph(context.WithoutCancel(request.Context()), name, current.authorization.User.Id, input)
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
	writeResponse(writer, value.payload)
}

func (handlers *favoriteHandlers) performGraph(ctx context.Context, name string, owner identity.Id, input graphRequest) (any, error) {
	var graph *favorites.Graph
	var err error
	switch name {
	case "list_relation_graph_favorites":
		return handlers.repository.ListGraphPage(ctx, owner, input.query.integerDefault("limit", 50), input.query.text("cursor"))
	case "create_relation_graph_favorite":
		return handlers.repository.CreateGraph(ctx, owner, input.create)
	case "get_relation_graph_favorite":
		graph, err = handlers.repository.GetGraph(ctx, owner, input.id)
	case "update_relation_graph_favorite":
		graph, err = handlers.repository.UpdateGraph(ctx, owner, input.id, input.update)
	case "delete_relation_graph_favorite":
		changed, err := handlers.repository.DeleteGraph(ctx, owner, input.id)
		return favoriteChanged(changed, err, "Graph not found")
	default:
		return nil, errors.New("unknown graph favorite operation")
	}
	if err != nil {
		return nil, err
	}
	if graph == nil {
		return nil, &apiError{status: 404, detail: "Graph not found"}
	}
	return graph, nil
}
