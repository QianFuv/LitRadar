package api

import (
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
)

func TestFavoriteValidationPrecedesBusinessAdmission(t *testing.T) {
	_, handlers, router, token := favoriteFixture(t)
	closed := executor.New(1, time.Second)
	closed.Close()
	handlers.pool = closed
	for _, scenario := range []struct{ method, path, body, detail string }{
		{"POST", "/api/favorites/folders", `{"name":" "}`, "Folder name must be 1-100 characters"},
		{"PUT", "/api/favorites/folders/1", `{"name":" "}`, "Folder name must be 1-100 characters"},
		{"PUT", "/api/favorites/tracking", `{"folder_id":0}`, "folder_id must be a positive integer"},
		{"GET", "/api/favorites/folders/1/articles?limit=0", "", "limit must be between 1 and 500"},
		{"GET", "/api/favorites/folders/1/articles?offset=-1", "", "offset must be greater than or equal to 0"},
		{"GET", "/api/favorites/folders/1/articles/page?limit=0", "", "limit must be between 1 and 500"},
		{"GET", "/api/favorites/folders/1/export?format=other", "", "Invalid export format"},
		{"POST", "/api/favorites/folders/0/articles", `{"article_id":0}`, "folder_id must be a positive integer"},
		{"POST", "/api/favorites/folders/1/articles", `{"article_id":0}`, "article_id must be a positive integer"},
		{"DELETE", "/api/favorites/folders/1/articles/0", "", "article_id must be a positive integer"},
		{"POST", "/api/favorites/folders/1/articles/bulk", `{"articles":[{"article_id":0}]}`, "article_id must be a positive integer"},
		{"POST", "/api/favorites/folders/1/articles/bulk-remove", `{"articles":[{"article_id":0}]}`, "article_id must be a positive integer"},
		{"POST", "/api/favorites/folders/1/articles/bulk-move", `{"target_folder_id":1,"articles":[{"article_id":0}]}`, "article_id must be a positive integer"},
		{"GET", "/api/favorites/check?article_id=0", "", "article_id must be a positive integer"},
		{"POST", "/api/favorites/check/batch", `{"article_ids":[],"db_name":"` + strings.Repeat("x", 256) + `"}`, "db_name must be at most 255 characters"},
	} {
		response := authRequest(router, scenario.method, scenario.path, scenario.body, token)
		if response.Code != 400 || !strings.Contains(response.Body.String(), scenario.detail) {
			t.Errorf("%s: %d %s", scenario.path, response.Code, response.Body.String())
		}
		if response := authRequest(router, scenario.method, scenario.path, scenario.body, ""); response.Code != 401 {
			t.Errorf("validation preceded authentication: %s %d", scenario.path, response.Code)
		}
	}
	for _, scenario := range []struct{ method, path, body string }{
		{"POST", "/api/favorites/folders/1/articles/bulk-move", `{"target_folder_id":1,"articles":[]}`},
		{"GET", "/api/favorites/folders/1/articles/page?cursor=invalid", ""},
		{"POST", "/api/favorites/check/batch", `{"article_ids":[0,-1]}`},
		{"POST", "/api/favorites/folders/1/articles/bulk", `{"articles":[]}`},
	} {
		if response := authRequest(router, scenario.method, scenario.path, scenario.body, token); response.Code != 503 {
			t.Errorf("business validation escaped admission: %s %d %s", scenario.path, response.Code, response.Body.String())
		}
	}
}

func TestFavoriteMoveValidatesItemsBeforeSameFolder(t *testing.T) {
	_, _, router, token := favoriteFixture(t)
	response := authRequest(router, "POST", "/api/favorites/folders/1/articles/bulk-move", `{"target_folder_id":1,"articles":[{"article_id":0}]}`, token)
	if response.Code != 400 || !strings.Contains(response.Body.String(), "article_id must be a positive integer") {
		t.Fatal(response.Code, response.Body.String())
	}
}
