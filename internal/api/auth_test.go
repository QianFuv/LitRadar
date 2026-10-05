package api

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	sqlite3 "github.com/mattn/go-sqlite3"
)

func authFixture(t *testing.T) (*authHandlers, *http.ServeMux, string) {
	t.Helper()
	ctx := context.Background()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(ctx, filename); err != nil {
		t.Fatal(err)
	}
	repository, err := storage.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	service := auth.New(repository, 2)
	user, err := service.Bootstrap(ctx, "api_fixture", "correct password long", nil)
	if err != nil {
		t.Fatal(err)
	}
	token, err := service.CreateTrustedToken(ctx, user.Id, "fixture", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	pool := executor.New(8, 30*time.Second)
	kdf := executor.New(2, 30*time.Second)
	t.Cleanup(pool.Close)
	t.Cleanup(kdf.Close)
	raw, _ := settings.Default("auth_rate_limit_policy")
	policy, err := settings.ParseRateLimitPolicy(raw)
	if err != nil {
		t.Fatal(err)
	}
	handlers := &authHandlers{service: service, repository: repository, authenticator: NewAuthenticator(service, pool), pool: pool, kdfPool: kdf, limiter: newAuthRateLimiter(policy), isCookieSecure: true}
	router := http.NewServeMux()
	routes := handlers.routes()
	if len(routes) != 14 {
		t.Fatal("auth route count")
	}
	for _, route := range routes {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	return handlers, router, token.Token
}

func authRequest(router http.Handler, method, path, body, token string) *httptest.ResponseRecorder {
	request := httptest.NewRequest(method, path, strings.NewReader(body))
	request.Header.Set("Content-Type", "application/json")
	if token != "" {
		request.Header.Set("Authorization", "Bearer "+token)
	}
	request = request.WithContext(context.WithValue(request.Context(), requestIdKey{}, "fixture-request"))
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)
	return recorder
}

func TestAuthRouteLifecycleAndNumericUserIdentity(t *testing.T) {
	_, router, token := authFixture(t)
	me := authRequest(router, "GET", "/api/auth/me", "", token)
	if me.Code != 200 || me.Body.String() != `{"id":1,"username":"api_fixture","is_admin":true}` {
		t.Fatal(me.Code, me.Body.String())
	}
	recorder := httptest.NewRecorder()
	writeResponse(recorder, publicUser(domain.User{Id: identity.Id(9007199254740993), Username: "large", IsAdmin: false}))
	if !strings.Contains(recorder.Body.String(), `"id":9007199254740993`) {
		t.Fatal("user precision", recorder.Body.String())
	}
	login := authRequest(router, "POST", "/api/auth/login", `{"username":" api_fixture ","password":"correct password long"}`, "")
	if login.Code != 200 || strings.Contains(login.Body.String(), `"token"`) || !strings.Contains(login.Body.String(), `"id":1`) {
		t.Fatal(login.Code, login.Body.String())
	}
	cookies := login.Result().Cookies()
	if len(cookies) != 1 || !cookies[0].Secure || !cookies[0].HttpOnly || cookies[0].MaxAge <= 0 {
		t.Fatal("session cookie", cookies)
	}
	session := cookies[0].Value
	issued := authRequest(router, "POST", "/api/auth/tokens", `["browser-issued",3600]`, session)
	if issued.Code != 200 {
		t.Fatal(issued.Code, issued.Body.String())
	}
	var access struct {
		Token string
		Id    int64
	}
	if err := json.Unmarshal(issued.Body.Bytes(), &access); err != nil {
		t.Fatal(err)
	}
	if access.Token == "" || access.Id == 0 {
		t.Fatal("missing issued token")
	}
	for _, path := range []string{"/api/auth/tokens", "/api/auth/invite-code", "/api/auth/invite-required"} {
		response := authRequest(router, "GET", path, "", access.Token)
		if response.Code != 200 {
			t.Fatal(path, response.Code, response.Body.String())
		}
	}
	for _, path := range []string{"/api/auth/invite-code", "/api/auth/invite-code/rotate"} {
		response := authRequest(router, "POST", path, "", access.Token)
		if response.Code != 200 {
			t.Fatal(path, response.Code, response.Body.String())
		}
	}
	response := authRequest(router, "DELETE", "/api/auth/invite-code", "", access.Token)
	if response.Code != 200 {
		t.Fatal(response.Body.String())
	}
	response = authRequest(router, "POST", "/api/auth/logout", "", session)
	if response.Code != 200 || response.Body.String() != `{"ok":true,"user_id":1}` {
		t.Fatal(response.Code, response.Body.String())
	}
	if response = authRequest(router, "GET", "/api/auth/me", "", session); response.Code != 401 {
		t.Fatal("session not revoked")
	}
	if response = authRequest(router, "GET", "/api/auth/me", "", access.Token); response.Code != 200 {
		t.Fatal("independent access token revoked")
	}
	if response = authRequest(router, "POST", "/api/auth/logout-all", "", access.Token); response.Code != 200 {
		t.Fatal(response.Body.String())
	}
	if response = authRequest(router, "GET", "/api/auth/me", "", token); response.Code != 401 {
		t.Fatal("all-session revocation failed")
	}
}

func TestAuthValidationPrecedenceAndRateLimit(t *testing.T) {
	_, router, _ := authFixture(t)
	for _, scenario := range []struct {
		path, body string
		status     int
		detail     string
	}{
		{"/api/auth/change-password", `{"old_password":"x","new_password":"x"}`, 400, "Password must be"},
		{"/api/auth/change-password", `{"old_password":"x","new_password":"long enough password"}`, 401, "Authentication required"},
		{"/api/auth/tokens", `{"ttl":0}`, 401, "Authentication required"},
		{"/api/auth/login", `{"username":false}`, 422, "expected a string"},
	} {
		response := authRequest(router, "POST", scenario.path, scenario.body, "")
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.detail) {
			t.Fatal(scenario.path, response.Code, response.Body.String())
		}
	}
	for attempt := 0; attempt < 6; attempt++ {
		response := authRequest(router, "POST", "/api/auth/login", `{"username":"absent","password":"bad"}`, "")
		expected := 401
		if attempt == 5 {
			expected = 429
		}
		if response.Code != expected {
			t.Fatal(attempt, response.Code, response.Body.String())
		}
		if expected == 429 && response.Header().Get("Retry-After") == "" {
			t.Fatal("missing retry")
		}
	}
}

func TestAuditFailureIsCountedOnceAndStillLogged(t *testing.T) {
	for _, mode := range []string{"sqlite", "closed", "queue"} {
		t.Run(mode, func(t *testing.T) {
			handlers, router, _ := authFixture(t)
			var logs bytes.Buffer
			previous := slog.Default()
			slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
			defer slog.SetDefault(previous)
			var release chan struct{}
			switch mode {
			case "sqlite":
				err := handlers.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
					_, err := connection.ExecContext(context.Background(), "CREATE TRIGGER reject_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT, 'private failure'); END")
					return err
				})
				if err != nil {
					t.Fatal(err)
				}
			case "closed":
				handlers.pool.Close()
			case "queue":
				handlers.pool = executor.New(1, 5*time.Millisecond)
				defer handlers.pool.Close()
				handlers.authenticator = NewAuthenticator(handlers.service, handlers.pool)
				entered := make(chan struct{})
				release = make(chan struct{})
				completed := make(chan struct{})
				defer func() { close(release); <-completed }()
				go func() {
					defer close(completed)
					_, _ = executor.Run(context.Background(), handlers.pool, func() (bool, error) { close(entered); <-release; return true, nil })
				}()
				<-entered
			}
			before := storage.AuditFailureCount()
			response := authRequest(router, "POST", "/api/auth/tokens", `{}`, "")
			if response.Code != 503 || response.Header().Get("Retry-After") != "5" {
				t.Fatal(response.Code, response.Body.String())
			}
			if storage.AuditFailureCount() != before+1 {
				t.Fatal("audit failure not counted exactly once")
			}
			if !strings.Contains(logs.String(), `"event":"security.auth.rejected"`) || !strings.Contains(logs.String(), `"reason":"operation_failed"`) || strings.Contains(logs.String(), "private failure") {
				t.Fatal(logs.String())
			}
			if mode != "sqlite" && !strings.Contains(logs.String(), `"error_kind":"executor_unavailable"`) {
				t.Fatal(logs.String())
			}
		})
	}
}

func TestLogoutUnconfirmedClearsCookieAndKeepsDedicatedError(t *testing.T) {
	handlers, router, token := authFixture(t)
	err := handlers.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "CREATE TRIGGER reject_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT, 'private failure'); END")
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	request := httptest.NewRequest("POST", "/api/auth/logout", nil)
	request.AddCookie(&http.Cookie{Name: "litradar_session", Value: token})
	request = request.WithContext(context.WithValue(request.Context(), requestIdKey{}, "logout-request"))
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != 503 || !strings.Contains(response.Body.String(), `"code":"session_revocation_unconfirmed"`) || response.Header().Get("Retry-After") != "" || !strings.Contains(response.Header().Get("Set-Cookie"), "Max-Age=0") {
		t.Fatal(response.Code, response.Header(), response.Body.String())
	}
	if !strings.Contains(logs.String(), "security.auth.revocation_unconfirmed") || !strings.Contains(logs.String(), "security.audit.persistence_failed") {
		t.Fatal(logs.String())
	}
	if response = authRequest(router, "GET", "/api/auth/me", "", token); response.Code != 200 {
		t.Fatal("failed audited revoke modified token")
	}
}

func TestRevocationRetryOnlyForTransientSqliteContention(t *testing.T) {
	for _, failure := range []error{sqlite3.Error{Code: sqlite3.ErrBusy}, sqlite3.Error{Code: sqlite3.ErrLocked}, domain.ErrAudit} {
		calls := 0
		_, _ = retryRevocation(func() (bool, error) { calls++; return false, failure })
		expected := 1
		if storage.IsTransientContention(failure) {
			expected = 2
		}
		if calls != expected {
			t.Fatal(calls, failure)
		}
	}
}
