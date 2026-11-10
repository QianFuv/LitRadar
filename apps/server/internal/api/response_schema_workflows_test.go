package api

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"path/filepath"
	"strconv"

	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/sources/zjlib"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/weekly"
	"github.com/QianFuv/LitRadar/apps/server/internal/testkit/content"
)

func requireSuccess(t *testing.T, router http.Handler, method, path, body, token string) *httptest.ResponseRecorder {
	t.Helper()
	response := authRequest(router, method, path, body, token)
	if response.Code < 200 || response.Code >= 400 {
		t.Fatalf("%s %s: %d %s", method, path, response.Code, response.Body.String())
	}
	return response
}

func exerciseAdditionalAuthResponses(t *testing.T) {
	handlers, router, token := authFixture(t)
	requireSuccess(t, router, "GET", "/api/auth/providers", "", "")
	invite, err := handlers.service.IssueInvite(context.Background(), 1, false, nil)
	if err != nil {
		t.Fatal(err)
	}
	body, err := json.Marshal(map[string]string{"username": "registered", "password": "a long new password", "invite_code": invite.Code})
	if err != nil {
		t.Fatal(err)
	}
	requireSuccess(t, router, "POST", "/api/auth/register", string(body), "")
	requireSuccess(t, router, "POST", "/api/auth/change-password", `{"old_password":"correct password long","new_password":"another long password"}`, token)
}

func seedResponseIndex(t *testing.T, configuration config.Config) {
	t.Helper()
	filename := filepath.Join(configuration.IndexDir, "fixture.sqlite")
	if err := content.Create(filename, 9); err != nil {
		t.Fatal(err)
	}
	database, err := storage.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	_, execution := database.ExecContext(context.Background(), `INSERT INTO issues(issue_id,journal_id,title,date,publication_year) VALUES(1,1,'Issue One','2026-01-01',2026); UPDATE articles SET issue_id=1,publication_year=2026 WHERE article_id=1001; UPDATE journals SET area='Business',utd_rating='A',abs_rating='4*' WHERE journal_id=1`)
	closed := database.Close()
	if execution != nil {
		t.Fatal(execution)
	}
	if closed != nil {
		t.Fatal(closed)
	}
}

func exerciseIndexResponses(t *testing.T) {
	auth, _, token := authFixture(t)
	configuration := config.FromProjectRoot(t.TempDir())
	seedResponseIndex(t, configuration)
	handlers := indexHandlers{storage: configuration, authenticator: auth.authenticator, pool: auth.pool, cache: &weekly.Cache{}}
	router := fixtureMux(handlers.routes())
	for _, path := range []string{
		"/api/meta/databases", "/api/meta/areas", "/api/meta/ratings", "/api/meta/journals", "/api/years",
		"/api/journals", "/api/journals/1", "/api/issues", "/api/issues/1", "/api/articles", "/api/articles/1001", "/api/articles/1001/related",
		"/api/weekly-updates", "/api/weekly-updates/summary",
	} {
		requireSuccess(t, router, "GET", path, "", token)
	}
	window := url.QueryEscape(weekly.FromTime(time.Now()).Format(false))
	requireSuccess(t, router, "GET", "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=1&window_end="+window, "", token)
}

func exerciseAdditionalFavoriteResponses(t *testing.T) {
	_, _, router, token := favoriteFixture(t)
	created := requireSuccess(t, router, "POST", "/api/favorites/folders", `{"name":"schema"}`, token)
	var folder struct {
		ID int64 `json:"id"`
	}
	if err := json.Unmarshal(created.Body.Bytes(), &folder); err != nil {
		t.Fatal(err)
	}
	path := "/api/favorites/folders/" + strconv.FormatInt(folder.ID, 10)
	requireSuccess(t, router, "POST", path+"/articles", `{"article_id":"1001","db_name":"missing"}`, token)
	requireSuccess(t, router, "GET", "/api/favorites/folders", "", token)
	requireSuccess(t, router, "GET", path+"/articles", "", token)
	requireSuccess(t, router, "GET", "/api/favorites/check?article_id=1001&db_name=missing", "", token)
	requireSuccess(t, router, "POST", "/api/favorites/check/batch", `{"article_ids":["1001"],"db_name":"missing"}`, token)
}

func exerciseAdditionalTrackingResponses(t *testing.T) {
	_, _, router, token, _ := trackingFixture(t)
	requireSuccess(t, router, "GET", "/api/tracking/status", "", token)
	requireSuccess(t, router, "GET", "/api/tracking/ai-endpoints", "", token)
	created := requireSuccess(t, router, "POST", "/api/tracking/push-weekly", "", token)
	var status manualPushStatus
	if err := json.Unmarshal(created.Body.Bytes(), &status); err != nil {
		t.Fatal(err)
	}
	if status.JobId == nil {
		t.Fatal("missing durable run identifier")
	}
	requireSuccess(t, router, "GET", "/api/tracking/push-weekly/runs/"+*status.JobId, "", token)
	_, cnki, cnkiToken := cnkiFixture(t, zjlib.Success)
	requireSuccess(t, cnki, "GET", "/api/cnki/session", "", cnkiToken)
}

func exerciseArticleResponses(t *testing.T) {
	handlers, _, router, token := articleFixture(t)
	seedResponseIndex(t, handlers.storage)
	registerArticleFixture(t, handlers, "fixture", articleFixtureProvider{hasSupport: true, resolve: func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
		return domain.ArticleFullTextResolution{Redirect: &domain.ArticleRedirect{Location: "https://example.org/article"}}, nil
	}})
	setArticleOrder(t, handlers, `{"default":["fixture"],"catalogs":{}}`)
	for _, suffix := range []string{"access", "abstract", "fulltext"} {
		requireSuccess(t, router, "GET", "/api/articles/1001/"+suffix+"?db=fixture.sqlite", "", token)
	}
}

func exerciseOIDCLinkResponses(t *testing.T) {
	_, router, provider := nativeApiFixture(t)
	login := requireSuccess(t, router, "POST", "/api/auth/login", `{"username":"api_fixture","password":"correct password long"}`, "")
	cookies := login.Result().Cookies()
	if len(cookies) != 1 {
		t.Fatal("missing browser session")
	}
	request := httptest.NewRequest("POST", "/api/auth/oidc/link", nil)
	request.AddCookie(cookies[0])
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	observeResponse(request, response)
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	var link oidcLinkResponse
	if err := json.Unmarshal(response.Body.Bytes(), &link); err != nil {
		t.Fatal(err)
	}
	target, err := url.Parse(link.AuthorizationURL)
	if err != nil {
		t.Fatal(err)
	}
	provider.mutex.Lock()
	provider.nonce, provider.challenge = target.Query().Get("nonce"), target.Query().Get("code_challenge")
	provider.mutex.Unlock()
	callback := httptest.NewRequest("GET", "/api/auth/oidc/callback?"+url.Values{"state": {target.Query().Get("state")}, "code": {"idp-code"}}.Encode(), nil)
	callback.AddCookie(cookies[0])
	for _, cookie := range response.Result().Cookies() {
		callback.AddCookie(cookie)
	}
	completed := httptest.NewRecorder()
	router.ServeHTTP(completed, callback)
	observeResponse(callback, completed)
	if completed.Code != 302 || completed.Header().Get("Location") != "/?oidc_link=linked" {
		t.Fatal(completed.Code, completed.Header(), completed.Body.String())
	}
	request = httptest.NewRequest("DELETE", "/api/auth/oidc/link", nil)
	request.AddCookie(cookies[0])
	response = httptest.NewRecorder()
	router.ServeHTTP(response, request)
	observeResponse(request, response)
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
}
