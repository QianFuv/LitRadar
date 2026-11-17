package api

import (
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/weekly"
)

// TestArticleSearchModeRejectsSimple checks the public enum before storage access.
func TestArticleSearchModeRejectsSimple(t *testing.T) {
	handlers, auth, router, token := articleFixture(t)
	index := indexHandlers{storage: handlers.storage, authenticator: auth.authenticator, pool: auth.pool, cache: &weekly.Cache{}}
	registerFixtureRoutes(t, router, index.routes())
	for _, mode := range []string{"simple", "semantic"} {
		response := authRequest(router, "GET", "/api/articles?search_mode="+mode, "", token)
		problem := requireProblem(t, response, 422, "validation failed")
		if len(problem.Errors) != 1 || problem.Errors[0].Location != "query.search_mode" {
			t.Fatal("missing mode validation detail", response.Body.String())
		}
	}
}
