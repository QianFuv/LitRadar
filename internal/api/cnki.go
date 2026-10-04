package api

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/sources/zjlib"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/transport"
)

type cnkiHandlers struct {
	sessions       *storage.CnkiSessions
	authenticator  *Authenticator
	pool, upstream *executor.Pool
	newClient      func() (*zjlib.Client, func(), error)
}

func liveCnkiClient(proxy transport.Proxy) func() (*zjlib.Client, func(), error) {
	return func() (*zjlib.Client, func(), error) {
		live, err := zjlib.NewLiveTransport(zjlib.DefaultLiveConfig(), proxy, time.Time{})
		if err != nil {
			return nil, nil, err
		}
		return zjlib.NewClient(live), live.Close, nil
	}
}

func (handlers *cnkiHandlers) routes() []route {
	routes := []route{}
	for _, operation := range []openapi.Operation{
		{Method: "GET", Path: "/api/cnki/session", Id: "get_session"},
		{Method: "DELETE", Path: "/api/cnki/session", Id: "clear_session"},
		{Method: "POST", Path: "/api/cnki/login/start", Id: "start_login"},
		{Method: "POST", Path: "/api/cnki/login/poll", Id: "poll_login"},
	} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) {
			handlers.handle(writer, request, operation.Id)
		}})
	}
	return routes
}

func runCnki[Value any](request *http.Request, pool *executor.Pool, work func() (Value, error)) (Value, *apiError) {
	type result struct {
		value Value
		err   error
	}
	observed, err := executor.RunWithQueueTimeout(request.Context(), pool, 30*time.Second, func() (result, error) { value, err := work(); return result{value, err}, nil })
	var zero Value
	if err != nil {
		return zero, mapExecutorError(err)
	}
	if observed.err != nil {
		var failure *apiError
		if errors.As(observed.err, &failure) {
			return zero, failure
		}
		return zero, internalError()
	}
	return observed.value, nil
}

func cnkiFailure(status int, code, phase, message string) *apiError {
	return &apiError{status: status, structured: map[string]string{"code": code, "phase": phase, "message": message}}
}
func cnkiSuperseded() *apiError {
	return cnkiFailure(409, "cnki_login_superseded", "login", "CNKI login operation was superseded")
}

func (handlers *cnkiHandlers) handle(writer http.ResponseWriter, request *http.Request, name string) {
	timeout, interval := int64(180), 2.0
	if name == "poll_login" {
		kind := structBody("CnkiLoginPollRequest", defaultBodyField("timeout_seconds", integerBody, int64(180)), defaultBodyField("interval_seconds", bodyType{kind: "float"}, 2.0))
		decoded, failure := extractBody(request, kind, false)
		if failure != nil {
			failure.write(writer)
			return
		}
		body := decoded.(map[string]any)
		timeout, interval = body["timeout_seconds"].(int64), body["interval_seconds"].(float64)
		if timeout < 1 || timeout > 600 {
			badRequest("timeout_seconds must be between 1 and 600").write(writer)
			return
		}
		if interval < 0.1 || interval > 10 {
			badRequest("interval_seconds must be between 0.1 and 10.0").write(writer)
			return
		}
	}
	current, failure := handlers.authenticator.requireUser(request)
	if failure != nil {
		failure.write(writer)
		return
	}
	owner := current.authorization.User.Id
	ctx := context.WithoutCancel(request.Context())
	if name == "get_session" || name == "clear_session" {
		status, failure := runCnki(request, handlers.pool, func() (storage.CnkiStatus, error) {
			if name == "clear_session" {
				if _, err := handlers.sessions.Clear(ctx, owner); err != nil {
					return storage.CnkiStatus{}, err
				}
			}
			return handlers.sessions.Status(ctx, owner)
		})
		if failure != nil {
			failure.write(writer)
			return
		}
		writeResponse(writer, status)
		return
	}
	if name == "start_login" {
		generation, failure := runCnki(request, handlers.pool, func() (int64, error) { return handlers.sessions.Reserve(ctx, owner) })
		if failure != nil {
			failure.write(writer)
			return
		}
		type loginResult struct {
			qr    zjlib.QrLogin
			state json.RawMessage
		}
		login, failure := runCnki(request, handlers.upstream, func() (loginResult, error) {
			client, close, err := handlers.newClient()
			if err != nil {
				return loginResult{}, cnkiFailure(502, "cnki_login_start_failed", "login", "CNKI login start failed")
			}
			defer close()
			qr, err := client.StartQrLogin(ctx)
			if err != nil {
				return loginResult{}, cnkiFailure(502, "cnki_login_start_failed", "login", "CNKI login start failed")
			}
			state, err := domain.EncodeJson(client.StateData())
			return loginResult{qr, json.RawMessage(state)}, err
		})
		if failure != nil {
			failure.write(writer)
			return
		}
		status, failure := runCnki(request, handlers.pool, func() (*storage.CnkiStatus, error) {
			return handlers.sessions.Complete(ctx, owner, generation, nil, login.state, "waiting_scan", &login.qr.Uuid)
		})
		if failure != nil {
			failure.write(writer)
			return
		}
		if status == nil {
			cnkiSuperseded().write(writer)
			return
		}
		writeResponse(writer, struct {
			Uuid    string              `json:"uuid"`
			Status  string              `json:"status"`
			QrCode  string              `json:"qr_code"`
			Session *storage.CnkiStatus `json:"session"`
		}{login.qr.Uuid, login.qr.Status, login.qr.QrCode, status})
		return
	}
	row, failure := runCnki(request, handlers.pool, func() (*storage.CnkiData, error) { return handlers.sessions.Data(ctx, owner, false) })
	if failure != nil {
		failure.write(writer)
		return
	}
	if row == nil || strings.TrimSpace(row.QrUuid) == "" {
		cnkiFailure(400, "cnki_login_not_started", "login", "CNKI QR login has not been started").write(writer)
		return
	}
	state, failure := runCnki(request, handlers.upstream, func() (json.RawMessage, error) {
		data, err := transport.ParseJson(row.SessionData)
		if err != nil {
			return nil, err
		}
		if object, ok := data.(map[string]any); ok {
			if _, exists := object["qr_uuid"]; !exists {
				object["qr_uuid"] = row.QrUuid
			}
		}
		client, close, err := handlers.newClient()
		if err != nil {
			return nil, cnkiFailure(400, "cnki_login_failed", "login", "CNKI login failed")
		}
		defer close()
		client.LoadStateData(data)
		if _, err = client.PollQrLogin(ctx, timeout, interval); err != nil {
			var upstream *zjlib.Error
			if errors.As(err, &upstream) && upstream.IsTimeout() {
				return nil, cnkiFailure(408, "cnki_login_timeout", "login", "CNKI login timed out")
			}
			return nil, cnkiFailure(400, "cnki_login_failed", "login", "CNKI login failed")
		}
		if _, err = client.WarmUpFulltextSession(ctx); err != nil {
			return nil, cnkiFailure(502, "cnki_warmup_failed", "warmup", "CNKI full-text session warm-up failed")
		}
		encoded, err := domain.EncodeJson(client.StateData())
		return json.RawMessage(encoded), err
	})
	if failure != nil {
		failure.write(writer)
		return
	}
	status, failure := runCnki(request, handlers.pool, func() (*storage.CnkiStatus, error) {
		var object map[string]json.RawMessage
		_ = json.Unmarshal(state, &object)
		qr := row.QrUuid
		if raw, exists := object["qr_uuid"]; exists && len(raw) > 0 && raw[0] == '"' {
			_ = json.Unmarshal(raw, &qr)
		}
		return handlers.sessions.Complete(ctx, owner, row.Generation, &row.QrUuid, state, "active", &qr)
	})
	if failure != nil {
		failure.write(writer)
		return
	}
	if status == nil {
		cnkiSuperseded().write(writer)
		return
	}
	writeResponse(writer, struct {
		Status  string              `json:"status"`
		Session *storage.CnkiStatus `json:"session"`
	}{"COMPLETE", status})
}
