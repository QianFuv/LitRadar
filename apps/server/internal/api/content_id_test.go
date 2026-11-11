package api

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/weekly"
	"github.com/QianFuv/LitRadar/apps/server/internal/testkit/content"
	"github.com/danielgtaylor/huma/v2"
)

const precisionArticleID int64 = 9223361829786370983
const precisionJournalID int64 = 9223361829786370981
const precisionIssueID int64 = 9223361829786370982

var contentIDParameters = []struct {
	method, path, location, name string
	isArray                      bool
}{
	{"GET", "/api/articles/{article_id}", "path", "article_id", false},
	{"GET", "/api/articles/{article_id}/abstract", "path", "article_id", false},
	{"GET", "/api/articles/{article_id}/access", "path", "article_id", false},
	{"GET", "/api/articles/{article_id}/fulltext", "path", "article_id", false},
	{"GET", "/api/articles/{article_id}/related", "path", "article_id", false},
	{"GET", "/api/favorites/check", "query", "article_id", false},
	{"DELETE", "/api/favorites/folders/{folder_id}/articles/{article_id}", "path", "article_id", false},
	{"GET", "/api/articles", "query", "journal_id", true},
	{"GET", "/api/issues", "query", "journal_id", false},
	{"GET", "/api/journals/{journal_id}", "path", "journal_id", false},
	{"GET", "/api/weekly-updates/articles", "query", "journal_id", false},
	{"GET", "/api/articles", "query", "issue_id", false},
	{"GET", "/api/issues/{issue_id}", "path", "issue_id", false},
}

func TestContentIDParametersAreDecimalStrings(t *testing.T) {
	api, err := newDocumentationAPI()
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range contentIDParameters {
		t.Run(scenario.method+" "+scenario.path+" "+scenario.name, func(t *testing.T) {
			path := api.OpenAPI().Paths[scenario.path]
			operation := path.Get
			if scenario.method == "DELETE" {
				operation = path.Delete
			}
			for _, parameter := range operation.Parameters {
				if parameter.In != scenario.location || parameter.Name != scenario.name {
					continue
				}
				schema := parameter.Schema
				if scenario.isArray {
					if schema.Type != "array" {
						t.Fatal("journal filters lost array encoding", schema)
					}
					schema = schema.Items
				}
				if schema.Type != "string" || schema.Format != "" || schema.Pattern != "^[1-9][0-9]*$" {
					t.Fatalf("imprecise ID contract: %+v", schema)
				}
				return
			}
			t.Fatal("ID parameter is missing")
		})
	}
	for _, name := range []string{"FavoriteGraph", "FavoriteGraphSummary", "RelationGraphFavoriteCreate"} {
		schema := api.OpenAPI().Components.Schemas.Map()[name].Properties["root_article_id"]
		if schema.Type != "string" || schema.Format != "" || schema.Pattern != "^[1-9][0-9]*$" {
			t.Fatalf("%s has an imprecise root ID: %+v", name, schema)
		}
	}
	for _, path := range api.OpenAPI().Paths {
		for _, operation := range []*huma.Operation{path.Get, path.Post, path.Put, path.Delete} {
			if operation == nil {
				continue
			}
			for _, parameter := range operation.Parameters {
				switch parameter.Name {
				case "user_id", "folder_id", "token_id", "graph_id", "announcement_id", "task_id", "code_id":
					if parameter.Schema.Type != "integer" {
						t.Fatal("autoincrement ID contract changed", operation.OperationID, parameter.Name)
					}
				}
			}
		}
	}
}

func seedPrecisionIndex(t *testing.T, configuration config.Config) {
	t.Helper()
	filename := filepath.Join(configuration.IndexDir, "fixture.sqlite")
	if err := content.Create(filename, 10); err != nil {
		t.Fatal(err)
	}
	database, err := sqlite.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := database.Close(); err != nil {
			t.Error(err)
		}
	})
	if _, err := database.Exec(`INSERT INTO journals(journal_id,catalog_id,title,title_aliases_json,issns_json) VALUES(?, 'precision-journal','Precision Journal','[]','[]')`, precisionJournalID); err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec(`INSERT INTO issues(issue_id,journal_id,title,date) VALUES(?,?,'Precision Issue','2026-01-01')`, precisionIssueID, precisionJournalID); err != nil {
		t.Fatal(err)
	}
	for index, title := range []string{"Precision evidence networks", "Precision evidence mapping"} {
		if _, err := database.Exec(`INSERT INTO articles(article_id,journal_id,issue_id,title,authors_json,date) VALUES(?,?,?,?,'[]','2026-01-01')`, precisionArticleID-int64(index), precisionJournalID, precisionIssueID, title); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := database.Exec(`INSERT INTO article_listing(article_id,journal_id,issue_id,date) SELECT article_id,journal_id,issue_id,date FROM articles WHERE journal_id=?`, precisionJournalID); err != nil {
		t.Fatal(err)
	}
	connection, err := database.Conn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if err := search.Rebuild(t.Context(), connection); err != nil {
		t.Fatal(err)
	}
	if err := connection.Close(); err != nil {
		t.Fatal(err)
	}
	directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	manifest := fmt.Sprintf(`{"db_name":"fixture.sqlite","generated_at":%q,"notifiable_article_ids":[%d]}`, "2026-11-11T12:00:00Z", precisionArticleID)
	if err := os.WriteFile(filepath.Join(directory, "precision.changes.json"), []byte(manifest), 0600); err != nil {
		t.Fatal(err)
	}
}

func TestContentIDRoutesPreserve63BitPrecision(t *testing.T) {
	handlers, auth, router, token := articleFixture(t)
	seedPrecisionIndex(t, handlers.storage)
	index := indexHandlers{storage: handlers.storage, authenticator: auth.authenticator, pool: auth.pool, cache: &weekly.Cache{}}
	registerFixtureRoutes(t, router, index.routes())
	folders := &favoriteHandlers{favorites.New(auth.repository), handlers.storage, auth.authenticator, auth.pool}
	registerFixtureRoutes(t, router, folders.routes())
	registerArticleFixture(t, handlers, "fixture", articleFixtureProvider{hasSupport: true, resolve: func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
		return domain.ArticleFullTextResolution{Redirect: &domain.ArticleRedirect{Location: "https://example.org/" + strconv.FormatInt(precisionArticleID, 10)}}, nil
	}})
	setArticleOrder(t, handlers, `{"default":["fixture"],"catalogs":{}}`)
	article := strconv.FormatInt(precisionArticleID, 10)
	journal := strconv.FormatInt(precisionJournalID, 10)
	issue := strconv.FormatInt(precisionIssueID, 10)
	window := url.QueryEscape("2026-11-12T00:00:00Z")
	for _, scenario := range []struct{ path, fragment string }{
		{"/api/articles/" + article, `"article_id":"` + article + `"`},
		{"/api/articles/" + article + "/related", `"article_id":"` + strconv.FormatInt(precisionArticleID-1, 10) + `"`},
		{"/api/journals/" + journal, `"journal_id":"` + journal + `"`},
		{"/api/issues/" + issue, `"issue_id":"` + issue + `"`},
		{"/api/articles?journal_id=" + journal + "&journal_id=2", `"article_id":"` + article + `"`},
		{"/api/articles?issue_id=" + issue, `"article_id":"` + article + `"`},
		{"/api/issues?journal_id=" + journal, `"issue_id":"` + issue + `"`},
		{"/api/weekly-updates/articles?db=fixture.sqlite&journal_id=" + journal + "&window_end=" + window, `"article_id":"` + article + `"`},
		{"/api/articles/" + article + "/access", `"available":true`},
	} {
		t.Run(scenario.path, func(t *testing.T) {
			response := requireSuccess(t, router, "GET", scenario.path, "", token)
			if response.Code != http.StatusOK || !strings.Contains(response.Body.String(), scenario.fragment) {
				t.Fatal(response.Code, response.Body.String())
			}
		})
	}
	for _, suffix := range []string{"abstract", "fulltext"} {
		response := requireSuccess(t, router, "GET", "/api/articles/"+article+"/"+suffix, "", token)
		if response.Code != http.StatusTemporaryRedirect || response.Header().Get("Location") != "https://example.org/"+article {
			t.Fatal(response.Code, response.Header())
		}
	}
	folder, err := folders.repository.CreateFolder(context.Background(), 1, "Precision", false)
	if err != nil {
		t.Fatal(err)
	}
	path := fmt.Sprintf("/api/favorites/folders/%d/articles", folder.ID)
	requireSuccess(t, router, "POST", path, `{"article_id":"`+article+`","db_name":"fixture"}`, token)
	response := requireSuccess(t, router, "GET", "/api/favorites/check?article_id="+article+"&db_name=fixture", "", token)
	if response.Body.String() == "[]" {
		t.Fatal("large article ID did not match its favorite")
	}
	response = requireSuccess(t, router, "GET", path, "", token)
	if !strings.Contains(response.Body.String(), `"issue_id":"`+issue+`"`) {
		t.Fatal(response.Body.String())
	}
	requireSuccess(t, router, "DELETE", path+"/"+article+"?db_name=fixture", "", token)
	response = requireSuccess(t, router, "GET", "/api/favorites/check?article_id="+article+"&db_name=fixture", "", token)
	if response.Body.String() != "[]" {
		t.Fatal("favorite with exact article ID was not removed", response.Body.String())
	}
}

func TestContentIDRoutesRejectNonCanonicalOrOutOfRangeIDs(t *testing.T) {
	auth, router, _ := authFixture(t)
	configuration := config.FromProjectRoot(t.TempDir())
	index := indexHandlers{storage: configuration, authenticator: auth.authenticator, pool: auth.pool, cache: &weekly.Cache{}}
	registerFixtureRoutes(t, router, index.routes())
	folders := &favoriteHandlers{favorites.New(auth.repository), configuration, auth.authenticator, auth.pool}
	registerFixtureRoutes(t, router, folders.routes())
	_, _, articleRouter, _ := articleFixture(t)
	for _, scenario := range contentIDParameters {
		for _, value := range []string{"0", "-1", "+1", "01", "1.0", "1e3", " 1", "1 ", "9223372036854775808", "18446744073709551615"} {
			t.Run(scenario.path+" "+scenario.name+"="+value, func(t *testing.T) {
				path := strings.ReplaceAll(scenario.path, "{folder_id}", "1")
				if scenario.location == "path" {
					path = strings.ReplaceAll(path, "{"+scenario.name+"}", url.PathEscape(value))
				} else {
					path += "?" + scenario.name + "=" + url.QueryEscape(value)
				}
				handler := router
				if strings.HasSuffix(scenario.path, "/abstract") || strings.HasSuffix(scenario.path, "/access") || strings.HasSuffix(scenario.path, "/fulltext") {
					handler = articleRouter
				}
				response := authRequest(handler, scenario.method, path, "", "")
				requireProblem(t, response, http.StatusUnprocessableEntity, "validation failed")
			})
		}
	}
	for _, path := range []string{"/api/articles?journal_id=", "/api/articles?issue_id=", "/api/articles?journal_id=&journal_id=1", "/api/issues?journal_id="} {
		requireProblem(t, authRequest(router, "GET", path, "", ""), http.StatusUnprocessableEntity, "validation failed")
	}
}

func TestGraphRootArticleIDRoundTripsAsDecimalString(t *testing.T) {
	_, _, router, token := graphFixture(t)
	const base = "/api/favorites/graphs"
	decimal := strconv.FormatInt(precisionArticleID, 10)
	body := `{"title":"Precision","db_name":"fixture","root_article_id":"` + decimal + `","node_count":1,"graph":{}}`
	created := requireGraphRequest(t, router, token, "POST", base, body, 200)
	graph := decodeGraphResponse(t, created)
	for _, response := range []string{created.Body.String(), requireGraphRequest(t, router, token, "GET", fmt.Sprintf("%s/%d", base, graph.ID), "", 200).Body.String(), requireGraphRequest(t, router, token, "GET", base, "", 200).Body.String()} {
		if !strings.Contains(response, `"root_article_id":"`+decimal+`"`) {
			t.Fatal("root article precision lost", response)
		}
	}
	for _, raw := range []string{decimal, `"0"`, `"+1"`, `"01"`, `"9223372036854775808"`, `null`} {
		invalid := strings.Replace(body, `"`+decimal+`"`, raw, 1)
		requireGraphRequest(t, router, token, "POST", base, invalid, 422)
	}
}
