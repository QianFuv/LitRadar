package api

import (
	"context"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
)

func (handlers *favoriteHandlers) graphRoutes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/graphs", ID: "list_relation_graph_favorites"}, func(ctx context.Context, input *ListRelationGraphFavoritesInput) (*output[favorites.GraphPage], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.ID
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (favorites.GraphPage, error) {
				return handlers.repository.ListGraphPage(workContext, owner, input.Limit, cleanText(input.Cursor.Pointer()))
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/favorites/graphs", ID: "create_relation_graph_favorite"}, func(ctx context.Context, input *CreateRelationGraphFavoriteInput) (*output[*favorites.Graph], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			if len(input.Body.Graph) > favorites.MaxGraphBytes {
				return nil, (&apiError{status: 413, detail: "graph must be at most 2 MiB"}).problem()
			}
			owner := current.authorization.User.ID
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (*favorites.Graph, error) {
				return handlers.repository.CreateGraph(workContext, owner, favorites.GraphCreate{Title: input.Body.Title, DBName: input.Body.DBName, RootArticleID: int64(input.Body.RootArticleID), NodeCount: input.Body.NodeCount, Graph: input.Body.Graph})
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/favorites/graphs/{graph_id}", ID: "get_relation_graph_favorite"}, func(ctx context.Context, input *GetRelationGraphFavoriteInput) (*output[*favorites.Graph], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.ID
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (*favorites.Graph, error) {
				graph, err := handlers.repository.GetGraph(workContext, owner, input.GraphID)
				if err != nil {
					return nil, err
				}
				if graph == nil {
					return nil, &apiError{status: 404, detail: "Graph not found"}
				}
				return graph, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/favorites/graphs/{graph_id}", ID: "update_relation_graph_favorite"}, func(ctx context.Context, input *UpdateRelationGraphFavoriteInput) (*output[*favorites.Graph], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			if len(input.Body.Graph) > favorites.MaxGraphBytes {
				return nil, (&apiError{status: 413, detail: "graph must be at most 2 MiB"}).problem()
			}
			owner := current.authorization.User.ID
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (*favorites.Graph, error) {
				graph, err := handlers.repository.UpdateGraph(workContext, owner, input.GraphID, favorites.GraphUpdate{Title: input.Body.Title, NodeCount: input.Body.NodeCount, Graph: input.Body.Graph})
				if err != nil {
					return nil, err
				}
				if graph == nil {
					return nil, &apiError{status: 404, detail: "Graph not found"}
				}
				return graph, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/favorites/graphs/{graph_id}", ID: "delete_relation_graph_favorite"}, func(ctx context.Context, input *DeleteRelationGraphFavoriteInput) (*output[okResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.ID
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (okResponse, error) {
				changed, err := handlers.repository.DeleteGraph(workContext, owner, input.GraphID)
				return changedResponse(changed, err, "Graph not found")
			}, mapBusinessError)
		}),
	}
}
