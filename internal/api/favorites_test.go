package api

import (
	"context"

	"database/sql"

	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"

	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
)

func favoriteFixture(t *testing.T) (*authHandlers, *favoriteHandlers, *http.ServeMux, string) {
	t.Helper()
	auth, router, token := authFixture(t)
	handlers := &favoriteHandlers{favorites.New(auth.repository), config.FromProjectRoot(t.TempDir()), auth.authenticator, auth.pool}
	routes := handlers.routes()
	if len(routes) != 17 {
		t.Fatal("favorite inventory")
	}
	for _, route := range routes {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	return auth, handlers, router, token
}

func TestFavoriteLifecycleOwnerIsolationAndExport(t *testing.T) {
	auth, handlers, router, token := favoriteFixture(t)
	create := func(name string) int64 {
		response := authRequest(router, "POST", "/api/favorites/folders", fmt.Sprintf(`{"name":%q}`, name), token)
		if response.Code != 200 {
			t.Fatal(response.Code, response.Body.String())
		}
		var folder favorites.Folder
		if err := json.Unmarshal(response.Body.Bytes(), &folder); err != nil {
			t.Fatal(err)
		}
		return folder.Id
	}
	source, target := create("文献"), create("target")
	path := fmt.Sprintf("/api/favorites/folders/%d", source)
	require := func(method, path, body string, status int) *httptest.ResponseRecorder {
		response := authRequest(router, method, path, body, token)
		if response.Code != status {
			t.Fatalf("%s %s: %d %s", method, path, response.Code, response.Body.String())
		}
		return response
	}
	require("POST", path+"/articles", `{"article_id":"9007199254740993","db_name":"unavailable","note":"original"}`, 200)
	duplicate := require("POST", path+"/articles", `{"article_id":"9007199254740993","db_name":"unavailable","note":"replacement"}`, 200)
	if !strings.Contains(duplicate.Body.String(), `"note":"original"`) || !strings.Contains(duplicate.Body.String(), `"article_id":"9007199254740993"`) {
		t.Fatal(duplicate.Body.String())
	}
	require("PUT", "/api/favorites/tracking", fmt.Sprintf(`{"folder_id":%d}`, source), 200)
	tracking := require("GET", "/api/favorites/tracking", "", 200)
	if !strings.Contains(tracking.Body.String(), `"folder_name":"文献"`) {
		t.Fatal(tracking.Body.String())
	}
	require("POST", path+"/articles/bulk", `{"articles":[{"article_id":2},{"article_id":3}]}`, 200)
	page := require("GET", path+"/articles/page?limit=1", "", 200)
	var parsed struct {
		Page struct {
			NextCursor *string `json:"next_cursor"`
		} `json:"page"`
	}
	if err := json.Unmarshal(page.Body.Bytes(), &parsed); err != nil || parsed.Page.NextCursor == nil {
		t.Fatal("missing cursor", page.Body.String())
	}
	require("GET", path+"/articles/page?limit=1&cursor="+*parsed.Page.NextCursor, "", 200)
	for _, format := range []string{"bibtex", "ris", "endnote"} {
		response := require("GET", path+"/export?format="+format, "", 200)
		if !strings.Contains(response.Header().Get("Content-Disposition"), `filename="favorites.`) || response.Body.Len() == 0 {
			t.Fatal(response.Header(), response.Body.String())
		}
	}
	require("POST", path+"/articles/bulk-move", fmt.Sprintf(`{"target_folder_id":%d,"articles":[{"article_id":2}]}`, target), 200)
	require("POST", path+"/articles/bulk-remove", `{"articles":[{"article_id":3}]}`, 200)
	require("DELETE", path+"/articles/9007199254740993?db_name=unavailable", "", 200)
	count := require("GET", path+"/count", "", 200)
	if count.Body.String() != `{"count":0}` {
		t.Fatal(count.Body.String())
	}
	require("PUT", path, `{"name":"renamed"}`, 200)
	require("DELETE", path, "", 200)
	require("GET", path+"/articles/page", "", 404)
	invite, err := auth.service.IssueInvite(context.Background(), 1, false, nil)
	if err != nil {
		t.Fatal(err)
	}
	foreign, err := auth.service.Register(context.Background(), "other_user", "long enough password", &invite.Code, nil)
	if err != nil {
		t.Fatal(err)
	}
	foreignToken, err := auth.service.CreateTrustedToken(context.Background(), foreign.Id, "foreign", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, suffix := range []string{"/articles/page", "/export"} {
		response := authRequest(router, "GET", fmt.Sprintf("/api/favorites/folders/%d", target)+suffix, "", foreignToken.Token)
		if response.Code != 404 {
			t.Fatal("foreign access", response.Code, response.Body.String())
		}
	}
	foreignFolder, err := handlers.repository.CreateFolder(context.Background(), foreign.Id, "foreign", false)
	if err != nil {
		t.Fatal(err)
	}
	require("POST", fmt.Sprintf("/api/favorites/folders/%d/articles", foreignFolder.Id), `{"article_id":1}`, 404)
}

func TestFavoriteExportRejectsOversizeSnapshotBeforeMetadata(t *testing.T) {
	auth, _, router, token := favoriteFixture(t)
	response := authRequest(router, "POST", "/api/favorites/folders", `{"name":"large"}`, token)
	var folder favorites.Folder
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &folder) != nil {
		t.Fatal(response.Body.String())
	}
	err := auth.repository.Immediate(context.Background(), false, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "WITH RECURSIVE ids(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM ids WHERE value<10001) INSERT INTO favorites(user_id,folder_id,article_id,db_name,note,created_at) SELECT 1,?,value,'unavailable','',1 FROM ids", folder.Id)
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	response = authRequest(router, "GET", fmt.Sprintf("/api/favorites/folders/%d/export", folder.Id), "", token)
	if response.Code != 413 || strings.Contains(response.Header().Get("Content-Disposition"), "attachment") || !strings.Contains(response.Body.String(), "10000 items") {
		t.Fatal(response.Code, response.Header(), response.Body.String())
	}
}
