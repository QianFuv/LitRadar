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
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
)

// TestNativeLoginPreservesWebSessionAndSupportsBearerLifecycle checks independent device credentials.
func TestNativeLoginPreservesWebSessionAndSupportsBearerLifecycle(t *testing.T) {
	handlers, router, fixtureToken := authFixture(t)
	web := authRequest(router, "POST", "/api/auth/login", `{"username":"api_fixture","password":"correct password long"}`, "")
	if web.Code != 200 || len(web.Result().Cookies()) != 1 {
		t.Fatal(web.Code, web.Body.String())
	}
	cookie := web.Result().Cookies()[0]
	var tokens []string
	for range 2 {
		started := float64(time.Now().UnixNano()) / 1e9
		response := authRequest(router, "POST", "/api/auth/native/login", `{"username":"api_fixture","password":"correct password long","device_name":"  Windows 电脑  "}`, "")
		var result struct {
			Token     string       `json:"token"`
			ExpiresAt float64      `json:"expires_at"`
			User      userResponse `json:"user"`
		}
		if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &result) != nil || result.Token == "" || result.User.ID != 1 || len(response.Header().Values("Set-Cookie")) != 0 {
			t.Fatal(response.Code, response.Header(), response.Body.String())
		}
		expires := started + 90*24*60*60
		if result.ExpiresAt < expires || result.ExpiresAt > expires+5 {
			t.Fatal("TTL is not 90 days", result.ExpiresAt, expires)
		}
		if response := authRequest(router, "GET", "/api/auth/me", "", result.Token); response.Code != 200 {
			t.Fatal(response.Code, response.Body.String())
		}
		tokens = append(tokens, result.Token)
	}
	if tokens[0] == tokens[1] {
		t.Fatal("device logins reused a token")
	}
	response := authRequest(router, "GET", "/api/auth/tokens", "", fixtureToken)
	var listed []domain.TokenInfo
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &listed) != nil {
		t.Fatal(response.Code, response.Body.String())
	}
	var deviceID int64
	devices := 0
	for _, token := range listed {
		if token.Name == "device:Windows 电脑" {
			devices++
			deviceID = token.ID
			if token.ExpiresAt-token.CreatedAt != 90*24*60*60 {
				t.Fatal(token)
			}
		}
	}
	if devices != 2 {
		t.Fatal("devices missing from personal token list", listed)
	}
	response = authRequest(router, "DELETE", fmt.Sprintf("/api/auth/tokens/%d", deviceID), "", fixtureToken)
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	active := 0
	for _, token := range tokens {
		response = authRequest(router, "GET", "/api/auth/me", "", token)
		if response.Code == 200 {
			active++
			logout := authRequest(router, "POST", "/api/auth/logout", "", token)
			if logout.Code != 200 || logout.Header().Get("Set-Cookie") != "" {
				t.Fatal(logout.Code, logout.Body.String())
			}
			if authRequest(router, "GET", "/api/auth/me", "", token).Code != 401 {
				t.Fatal("logout did not revoke bearer")
			}
		} else if response.Code != 401 {
			t.Fatal(response.Code, response.Body.String())
		}
	}
	if active != 1 {
		t.Fatal("token ID revocation did not affect exactly one device")
	}
	request := httptest.NewRequest(http.MethodGet, "/api/auth/me", nil)
	request.AddCookie(cookie)
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)
	if recorder.Code != 200 {
		t.Fatal("native lifecycle revoked web login", recorder.Code, recorder.Body.String())
	}
	events, err := handlers.repository.ListAudit(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	completed := 0
	for _, event := range events {
		if event.Action == "native_login" && event.Outcome == "completed" {
			completed++
			if event.ActorID == nil || *event.ActorID != 1 || event.TargetID == nil {
				t.Fatal(event)
			}
		}
	}
	if completed != 2 {
		t.Fatal("native login not audited atomically", events)
	}
}

// TestNativeLoginUsesPasswordFailuresAndSharedLockout prevents a parallel password guessing budget.
func TestNativeLoginUsesPasswordFailuresAndSharedLockout(t *testing.T) {
	for _, isOIDC := range []bool{false, true} {
		t.Run(fmt.Sprint(isOIDC), func(t *testing.T) {
			handlers, router, _ := authFixture(t)
			if isOIDC {
				if err := handlers.repository.Immediate(context.Background(), false, func(connection *sql.Conn) error {
					_, err := connection.ExecContext(context.Background(), "UPDATE users SET auth_kind='oidc',password_hash='!oidc',salt='' WHERE id=1")
					return err
				}); err != nil {
					t.Fatal(err)
				}
			}
			password := "wrong password"
			if isOIDC {
				password = "correct password long"
			}
			body := fmt.Sprintf(`{"username":"api_fixture","password":%q,"device_name":"Android"}`, password)
			var previous string
			for attempt := range 6 {
				path := "/api/auth/native/login"
				if attempt%2 == 1 && attempt < 5 {
					path = "/api/auth/login"
				}
				requestBody := body
				if path == "/api/auth/login" {
					requestBody = fmt.Sprintf(`{"username":"api_fixture","password":%q}`, password)
				}
				response := authRequest(router, "POST", path, requestBody, "")
				expected := 401
				if attempt == 5 {
					expected = 429
				}
				if response.Code != expected || response.Header().Get("Set-Cookie") != "" {
					t.Fatal(attempt, response.Code, response.Body.String())
				}
				if attempt < 5 {
					if previous != "" && previous != response.Body.String() {
						t.Fatal("password error differs from web login")
					}
					previous = response.Body.String()
				}
				if attempt == 5 && response.Header().Get("Retry-After") == "" {
					t.Fatal("missing retry-after")
				}
			}
		})
	}
}

// TestNativeLoginValidatesDeviceNameAndReusesQuota checks bounded naming and the existing personal-token cap.
func TestNativeLoginValidatesDeviceNameAndReusesQuota(t *testing.T) {
	handlers, router, _ := authFixture(t)
	for _, name := range []string{"", "   ", "Android\nphone", strings.Repeat("界", 94)} {
		body, _ := json.Marshal(map[string]string{"username": "api_fixture", "password": "correct password long", "device_name": name})
		response := authRequest(router, "POST", "/api/auth/native/login", string(body), "")
		expected := 400
		if name == "" || len([]rune(name)) > 93 {
			expected = 422
		}
		if response.Code != expected || response.Header().Get("Set-Cookie") != "" {
			t.Fatal(response.Code, response.Body.String())
		}
	}
	if response := authRequest(router, "POST", "/api/auth/native/login", `{"username":"api_fixture","password":"correct password long"}`, ""); response.Code != 422 {
		t.Fatal(response.Code, response.Body.String())
	}
	if response := authRequest(router, "POST", "/api/auth/native/login", `{"username":"api_fixture","password":"correct password long","device_name":false}`, ""); response.Code != 422 {
		t.Fatal(response.Code, response.Body.String())
	}
	handlers.limiter = newAuthRateLimiter(handlers.limiter.policy)
	for ordinal := range 49 {
		if _, err := handlers.service.CreateTrustedToken(context.Background(), 1, fmt.Sprint(ordinal), 3600, nil); err != nil {
			t.Fatal(err)
		}
	}
	response := authRequest(router, "POST", "/api/auth/native/login", `{"username":"api_fixture","password":"correct password long","device_name":"Android"}`, "")
	if response.Code != 409 || !strings.Contains(response.Body.String(), "limit of 50") {
		t.Fatal(response.Code, response.Body.String())
	}
}
