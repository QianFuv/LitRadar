package api

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/sources/zjlib"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

func cnkiFixture(t *testing.T, mode zjlib.FixtureMode) (*cnkiHandlers, *http.ServeMux, string) {
	t.Helper()
	auth, router, token := authFixture(t)
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	upstream := executor.New(4, 30*time.Second)
	t.Cleanup(upstream.Close)
	handlers := &cnkiHandlers{sessions: storage.NewCnkiSessions(auth.repository, codec), authenticator: auth.authenticator, pool: auth.pool, upstream: upstream, newClient: func() (*zjlib.Client, func(), error) {
		return zjlib.NewClient(zjlib.NewFixtureTransport(mode)), func() {}, nil
	}}
	for _, route := range handlers.routes() {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	return handlers, router, token
}

func TestCnkiRoutesPreserveLifecycleAndClassifiedErrors(t *testing.T) {
	for _, scenario := range []struct {
		mode   zjlib.FixtureMode
		status int
		code   string
	}{
		{zjlib.Success, 200, ""}, {zjlib.StartFailure, 502, "cnki_login_start_failed"}, {zjlib.PollTimeout, 408, "cnki_login_timeout"}, {zjlib.PollFailure, 400, "cnki_login_failed"}, {zjlib.WarmupFailure, 502, "cnki_warmup_failed"},
	} {
		t.Run(string(scenario.mode), func(t *testing.T) {
			handlers, router, token := cnkiFixture(t, scenario.mode)
			response := authRequest(router, "POST", "/api/cnki/login/poll", `{}`, token)
			if response.Code != 400 || !strings.Contains(response.Body.String(), "cnki_login_not_started") {
				t.Fatal(response.Code, response.Body.String())
			}
			response = authRequest(router, "POST", "/api/cnki/login/start", "", token)
			if scenario.mode == zjlib.StartFailure {
				if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.code) {
					t.Fatal(response.Code, response.Body.String())
				}
				return
			}
			if response.Code != 200 || !strings.Contains(response.Body.String(), `"status":"WAITING_SCAN"`) || !strings.Contains(response.Body.String(), `"status":"waiting_scan"`) {
				t.Fatal(response.Code, response.Body.String())
			}
			response = authRequest(router, "POST", "/api/cnki/login/poll", `{}`, token)
			if response.Code != scenario.status {
				t.Fatal(response.Code, response.Body.String())
			}
			if scenario.code != "" && !strings.Contains(response.Body.String(), scenario.code) {
				t.Fatal(response.Body.String())
			}
			data, err := handlers.sessions.Data(context.Background(), 1, false)
			if err != nil || data == nil {
				t.Fatal("missing session", err)
			}
			if scenario.mode == zjlib.Success {
				if data.Status != "active" || !strings.Contains(response.Body.String(), `"status":"COMPLETE"`) || strings.Contains(response.Body.String(), "SECRET_VPN_VALUE") || strings.Contains(response.Body.String(), "bff_user_token\":\"") {
					t.Fatal("bad safe result", response.Body.String())
				}
			} else if data.Status != "waiting_scan" {
				t.Fatal("failed poll persisted session")
			}
			response = authRequest(router, "DELETE", "/api/cnki/session", "", token)
			if response.Code != 200 || !strings.Contains(response.Body.String(), `"status":"empty"`) {
				t.Fatal(response.Body.String())
			}
		})
	}
}

func TestCnkiPollValidationOccursBeforeAuthentication(t *testing.T) {
	_, router, _ := cnkiFixture(t, zjlib.Success)
	for _, scenario := range []struct {
		body   string
		status int
		detail string
	}{
		{`{"timeout_seconds":0,"interval_seconds":0}`, 400, "timeout_seconds"},
		{`{"interval_seconds":0}`, 400, "interval_seconds"},
		{`{"timeout_seconds":null}`, 422, "expected i64"},
		{`{}`, 401, "Authentication required"},
		{`[180,2]`, 401, "Authentication required"},
	} {
		response := authRequest(router, "POST", "/api/cnki/login/poll", scenario.body, "")
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.detail) {
			t.Fatal(scenario.body, response.Code, response.Body.String())
		}
	}
}

func TestCnkiLateCompletionCannotResurrectClearedSession(t *testing.T) {
	for _, operation := range []string{"start", "poll"} {
		t.Run(operation, func(t *testing.T) {
			handlers, router, token := cnkiFixture(t, zjlib.Success)
			if operation == "poll" {
				response := authRequest(router, "POST", "/api/cnki/login/start", "", token)
				if response.Code != 200 {
					t.Fatal(response.Body.String())
				}
			}
			original := handlers.newClient
			handlers.newClient = func() (*zjlib.Client, func(), error) {
				if _, err := handlers.sessions.Clear(context.Background(), 1); err != nil {
					return nil, nil, err
				}
				return original()
			}
			response := authRequest(router, "POST", "/api/cnki/login/"+operation, `{}`, token)
			if response.Code != 409 || !strings.Contains(response.Body.String(), "cnki_login_superseded") {
				t.Fatal(response.Code, response.Body.String())
			}
			data, err := handlers.sessions.Data(context.Background(), 1, false)
			if err != nil || data != nil {
				t.Fatal("late completion restored deleted state", err)
			}
		})
	}
}

func TestCnkiPollPreservesExplicitNullQrIdentifier(t *testing.T) {
	handlers, router, token := cnkiFixture(t, zjlib.Success)
	qr := "stored-qr"
	_, err := handlers.sessions.Upsert(context.Background(), 1, json.RawMessage(`{"qr_uuid":null}`), "waiting_scan", &qr)
	if err != nil {
		t.Fatal(err)
	}
	response := authRequest(router, "POST", "/api/cnki/login/poll", `{}`, token)
	if response.Code != 400 || !strings.Contains(response.Body.String(), "cnki_login_failed") {
		t.Fatal("null QR replaced by fallback", response.Code, response.Body.String())
	}
}
