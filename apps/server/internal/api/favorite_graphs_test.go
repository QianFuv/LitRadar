package api

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
)

func graphFixture(t *testing.T) (*authHandlers, *favoriteHandlers, *http.ServeMux, string) {
	t.Helper()
	auth, handlers, router, token := favoriteFixture(t)
	registerFixtureRoutes(t, router, handlers.graphRoutes())
	return auth, handlers, router, token
}

func graphRequestBody(title string, count int, graph string) string {
	return fmt.Sprintf(`{"title":%q,"db_name":"source","root_article_id":42,"node_count":%d,"graph":%s}`, title, count, graph)
}

func requireGraphRequest(t *testing.T, router http.Handler, token, method, path, body string, status int) *httptest.ResponseRecorder {
	t.Helper()
	response := authRequest(router, method, path, body, token)
	if response.Code != status {
		t.Fatalf("%s %s: status %d, want %d: %.500s", method, path, response.Code, status, response.Body.String())
	}
	return response
}

func decodeGraphResponse(t *testing.T, response *httptest.ResponseRecorder) favorites.Graph {
	t.Helper()
	var graph favorites.Graph
	if err := json.Unmarshal(response.Body.Bytes(), &graph); err != nil {
		t.Fatal(err)
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(response.Body.Bytes(), &fields); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"id", "title", "db_name", "root_article_id", "node_count", "created_at", "updated_at", "graph"} {
		if _, exists := fields[name]; !exists {
			t.Fatal("missing response field", name)
		}
	}
	if len(fields) != 8 || graph.ID <= 0 || graph.CreatedAt <= 0 || graph.UpdatedAt < graph.CreatedAt || fields["graph"][0] != '{' {
		t.Fatal("invalid graph shape", response.Body.String())
	}
	return graph
}

// TestRelationGraphFavoriteLifecycle covers every operation and the public response shapes.
func TestRelationGraphFavoriteLifecycle(t *testing.T) {
	_, _, router, token := graphFixture(t)
	const base = "/api/favorites/graphs"
	response := requireGraphRequest(t, router, token, "GET", base, "", 200)
	if response.Body.String() != `{"items":[],"next_cursor":null}` {
		t.Fatal(response.Body.String())
	}
	created := decodeGraphResponse(t, requireGraphRequest(t, router, token, "POST", base, graphRequestBody(" first ", 2, `{"nodes":[1,2],"large":9007199254740993}`), 200))
	if created.Title != "first" || created.DBName != "source" || created.RootArticleID != 42 || created.NodeCount != 2 {
		t.Fatal(created)
	}
	path := fmt.Sprintf("%s/%d", base, created.ID)
	loaded := decodeGraphResponse(t, requireGraphRequest(t, router, token, "GET", path, "", 200))
	if !reflect.DeepEqual(loaded, created) {
		t.Fatal(loaded, created)
	}
	renamed := decodeGraphResponse(t, requireGraphRequest(t, router, token, "PUT", path, `{"title":" renamed "}`, 200))
	if renamed.Title != "renamed" || renamed.NodeCount != created.NodeCount || string(renamed.Graph) != string(created.Graph) || renamed.CreatedAt != created.CreatedAt || renamed.UpdatedAt < created.UpdatedAt {
		t.Fatal(renamed)
	}
	updated := decodeGraphResponse(t, requireGraphRequest(t, router, token, "PUT", path, `{"node_count":1,"graph":{"nodes":[3],"custom":{"flag":true}}}`, 200))
	if updated.Title != "renamed" || updated.NodeCount != 1 || string(updated.Graph) != `{"nodes":[3],"custom":{"flag":true}}` {
		t.Fatal(updated)
	}
	decodeGraphResponse(t, requireGraphRequest(t, router, token, "PUT", path, `{"title":"combined","node_count":2,"graph":{"nodes":[3,4]}}`, 200))
	second := decodeGraphResponse(t, requireGraphRequest(t, router, token, "POST", base, graphRequestBody("second", 1, `{}`), 200))
	response = requireGraphRequest(t, router, token, "GET", base+"?limit=1", "", 200)
	var page favorites.GraphPage
	if err := json.Unmarshal(response.Body.Bytes(), &page); err != nil || len(page.Items) != 1 || page.Items[0].ID != second.ID || page.NextCursor == nil || strings.Contains(response.Body.String(), `"graph"`) || strings.Contains(response.Body.String(), `"page"`) {
		t.Fatal(response.Body.String(), err)
	}
	response = requireGraphRequest(t, router, token, "GET", base+"?limit=1&cursor="+*page.NextCursor, "", 200)
	if err := json.Unmarshal(response.Body.Bytes(), &page); err != nil || len(page.Items) != 1 || page.Items[0].ID != created.ID || page.NextCursor != nil {
		t.Fatal(response.Body.String(), err)
	}
	response = requireGraphRequest(t, router, token, "DELETE", path, "", 200)
	if response.Body.String() != `{"ok":true}` {
		t.Fatal(response.Body.String())
	}
	for _, operation := range []struct{ method, body string }{{"GET", ""}, {"PUT", `{"title":"missing"}`}, {"DELETE", ""}} {
		response = requireGraphRequest(t, router, token, operation.method, path, operation.body, 404)
		requireProblem(t, response, 404, "Graph not found")
	}
}

// TestRelationGraphFavoriteAuthenticationAndOwnership rejects unauthenticated and foreign access.
func TestRelationGraphFavoriteAuthenticationAndOwnership(t *testing.T) {
	auth, _, router, token := graphFixture(t)
	const base = "/api/favorites/graphs"
	created := decodeGraphResponse(t, requireGraphRequest(t, router, token, "POST", base, graphRequestBody("owned", 1, `{}`), 200))
	path := fmt.Sprintf("%s/%d", base, created.ID)
	for _, operation := range []struct{ method, path, body string }{
		{"GET", base, ""}, {"POST", base, graphRequestBody("new", 1, `{}`)},
		{"GET", path, ""}, {"PUT", path, `{"title":"new"}`}, {"DELETE", path, ""},
	} {
		requireGraphRequest(t, router, "", operation.method, operation.path, operation.body, 401)
	}
	ctx := context.Background()
	invite, err := auth.service.IssueInvite(ctx, 1, false, nil)
	if err != nil {
		t.Fatal(err)
	}
	foreign, err := auth.service.Register(ctx, "graph_other", "long enough password", &invite.Code, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	foreignToken, err := auth.service.CreateTrustedToken(ctx, foreign.ID, "foreign", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, operation := range []struct{ method, body string }{{"GET", ""}, {"PUT", `{"title":"intrusion"}`}, {"DELETE", ""}} {
		foreign := requireGraphRequest(t, router, foreignToken.Token, operation.method, path, operation.body, 404)
		missing := requireGraphRequest(t, router, foreignToken.Token, operation.method, base+"/9999999", operation.body, 404)
		if foreign.Body.String() != missing.Body.String() {
			t.Fatal("foreign graph existence disclosed")
		}
	}
	response := requireGraphRequest(t, router, foreignToken.Token, "GET", base, "", 200)
	if response.Body.String() != `{"items":[],"next_cursor":null}` {
		t.Fatal(response.Body.String())
	}
	loaded := decodeGraphResponse(t, requireGraphRequest(t, router, token, "GET", path, "", 200))
	if loaded.Title != "owned" {
		t.Fatal(loaded)
	}
}

// TestRelationGraphFavoriteValidation exercises business and typed-body rejection.
func TestRelationGraphFavoriteValidation(t *testing.T) {
	_, _, router, token := graphFixture(t)
	const base = "/api/favorites/graphs"
	for _, scenario := range []struct {
		name, method, path, body string
		status                   int
	}{
		{"empty title", "POST", base, graphRequestBody(" ", 1, `{}`), 400},
		{"long title", "POST", base, graphRequestBody(strings.Repeat("文", 201), 1, `{}`), 422},
		{"array graph", "POST", base, graphRequestBody("valid", 1, `[]`), 422},
		{"null graph", "POST", base, graphRequestBody("valid", 1, `null`), 422},
		{"scalar graph", "POST", base, graphRequestBody("valid", 1, `42`), 422},
		{"string graph", "POST", base, graphRequestBody("valid", 1, `"graph"`), 422},
		{"zero nodes", "POST", base, graphRequestBody("valid", 0, `{}`), 422},
		{"too many nodes", "POST", base, graphRequestBody("valid", 501, `{}`), 422},
		{"invalid root", "POST", base, `{"title":"title","db_name":"","root_article_id":0,"node_count":1,"graph":{}}`, 422},
		{"long database", "POST", base, strings.Replace(graphRequestBody("valid", 1, `{}`), `"source"`, fmt.Sprintf("%q", strings.Repeat("文", 256)), 1), 422},
		{"empty update", "PUT", base + "/1", `{}`, 400},
		{"count without graph", "PUT", base + "/1", `{"node_count":1}`, 400},
		{"title and count without graph", "PUT", base + "/1", `{"title":"valid","node_count":1}`, 400},
		{"graph without count", "PUT", base + "/1", `{"graph":{}}`, 400},
		{"invalid replacement", "PUT", base + "/1", `{"graph":null,"node_count":1}`, 400},
		{"empty updated title", "PUT", base + "/1", `{"title":" "}`, 400},
		{"missing create fields", "POST", base, `{}`, 422},
		{"invalid field type", "PUT", base + "/1", `{"title":1}`, 422},
		{"invalid syntax", "POST", base, `{`, 400},
		{"small limit", "GET", base + "?limit=0", "", 422},
		{"large limit", "GET", base + "?limit=101", "", 422},
		{"bad cursor", "GET", base + "?cursor=bad", "", 400},
		{"empty cursor", "GET", base + "?cursor=", "", 200},
		{"zero graph id", "GET", base + "/0", "", 422},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			response := requireGraphRequest(t, router, token, scenario.method, scenario.path, scenario.body, scenario.status)
			if scenario.status >= 400 {
				requireProblem(t, response, scenario.status, "")
			}
		})
	}
	response := requireGraphRequest(t, router, token, "GET", base+"?cursor=bad", "", 400)
	requireProblem(t, response, 400, "Invalid favorite pagination cursor")
}

// TestRelationGraphFavoriteSizeBoundary accepts full snapshots and rejects byte oversize.
func TestRelationGraphFavoriteSizeBoundary(t *testing.T) {
	_, _, router, token := graphFixture(t)
	const base = "/api/favorites/graphs"
	graph := `{"data":"` + strings.Repeat("x", favorites.MaxGraphBytes-len(`{"data":""}`)) + `"}`
	created := decodeGraphResponse(t, requireGraphRequest(t, router, token, "POST", base, graphRequestBody(strings.Repeat("文", 200), 500, graph), 200))
	if len(created.Graph) != favorites.MaxGraphBytes {
		t.Fatal("snapshot size", len(created.Graph))
	}
	path := fmt.Sprintf("%s/%d", base, created.ID)
	requireGraphRequest(t, router, token, "PUT", path, `{"node_count":1,"graph":`+graph+`}`, 200)
	oversize := graph[:len(graph)-2] + `x"}`
	for _, operation := range []struct{ method, path, body string }{
		{"POST", base, graphRequestBody("large", 1, oversize)},
		{"PUT", path, `{"node_count":1,"graph":` + oversize + `}`},
	} {
		response := requireGraphRequest(t, router, token, operation.method, operation.path, operation.body, 413)
		requireProblem(t, response, 413, "graph must be at most 2 MiB")
	}
	response := requireGraphRequest(t, router, token, "POST", base, strings.Repeat(" ", 3*1024*1024+1), 413)
	requireProblem(t, response, 413, "request body is too large limit=3145728 bytes")
}

// TestRelationGraphFavoriteRegisteredRoutesAndContract checks production routing and offline generation.
func TestRelationGraphFavoriteRegisteredRoutesAndContract(t *testing.T) {
	router, _, token := completeRouter(t)
	const base = "/api/favorites/graphs"
	requireGraphRequest(t, router, token, "GET", "/api/favorites/folders", "", 200)
	created := decodeGraphResponse(t, requireGraphRequest(t, router, token, "POST", base, graphRequestBody("production", 1, `{"large":9007199254740993}`), 200))
	if string(created.Graph) != `{"large":9007199254740993}` {
		t.Fatal("validated request lost snapshot precision", string(created.Graph))
	}
	path := fmt.Sprintf("%s/%d", base, created.ID)
	requireGraphRequest(t, router, token, "GET", base, "", 200)
	requireGraphRequest(t, router, token, "GET", path, "", 200)
	requireGraphRequest(t, router, token, "PUT", path, `{"title":"updated"}`, 200)
	requireGraphRequest(t, router, token, "DELETE", path, "", 200)
	document, err := GenerateOpenAPI()
	if err != nil {
		t.Fatal(err)
	}
	served := requireGraphRequest(t, router, token, "GET", "/openapi.json", "", 200)
	if served.Body.String() != string(document) {
		t.Fatal("offline and live OpenAPI documents differ")
	}
	var contract struct {
		Paths map[string]map[string]struct {
			OperationID string `json:"operationId"`
		} `json:"paths"`
		Components struct {
			Schemas map[string]json.RawMessage `json:"schemas"`
		} `json:"components"`
	}
	if err := json.Unmarshal(document, &contract); err != nil {
		t.Fatal(err)
	}
	for _, route := range (&favoriteHandlers{}).graphRoutes() {
		operation := route.operation
		if contract.Paths[operation.Path][strings.ToLower(operation.Method)].OperationID != operation.ID {
			t.Fatal("missing graph operation", operation)
		}
	}
	for _, name := range []string{"FavoriteGraphSummary", "FavoriteGraph", "RelationGraphFavoriteCreate", "RelationGraphFavoriteUpdate", "FavoriteGraphPage"} {
		if _, exists := contract.Components.Schemas[name]; !exists {
			t.Fatal("missing graph schema", name)
		}
	}
}
