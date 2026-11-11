package api

import (
	"context"
	"encoding/json"
	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/sources/zjlib"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"net/http"
	"strings"
)

type cnkiStartResponse struct {
	// UUID describes qR UUID.
	UUID string `json:"uuid" nullable:"false"`
	// Status describes current lifecycle state.
	Status string `json:"status" nullable:"false"`
	// QRCode describes qR code URL or payload.
	QRCode string `json:"qr_code" nullable:"false"`
	// Session describes safe session status.
	Session *storage.CnkiStatus `json:"session" nullable:"false"`
}
type cnkiPollResponse struct {
	// Status describes current lifecycle state.
	Status string `json:"status" nullable:"false"`
	// Session describes safe session status.
	Session *storage.CnkiStatus `json:"session" nullable:"false"`
}

func (handlers *cnkiHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/cnki/session", ID: "get_session"}, func(ctx context.Context, input *GetSessionInput) (*output[storage.CnkiStatus], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			return handlers.cnkiSession(input.request, context.WithoutCancel(ctx), owner, "get_session")
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/cnki/session", ID: "clear_session"}, func(ctx context.Context, input *ClearSessionInput) (*output[storage.CnkiStatus], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			return handlers.cnkiSession(input.request, context.WithoutCancel(ctx), owner, "clear_session")
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/cnki/login/start", ID: "start_login"}, func(ctx context.Context, input *StartLoginInput) (*output[cnkiStartResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			return handlers.startCnkiLogin(input.request, context.WithoutCancel(ctx), owner)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/cnki/login/poll", ID: "poll_login"}, func(ctx context.Context, input *PollLoginInput) (*output[cnkiPollResponse], error) {
			if *input.Body.TimeoutSeconds < 1 || *input.Body.TimeoutSeconds > 600 {
				return nil, badRequest("timeout_seconds must be between 1 and 600").problem()
			}
			if *input.Body.IntervalSeconds < 0.1 || *input.Body.IntervalSeconds > 10 {
				return nil, badRequest("interval_seconds must be between 0.1 and 10.0").problem()
			}
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			owner := current.authorization.User.Id
			return handlers.pollCnkiLogin(input.request, context.WithoutCancel(ctx), owner, *input.Body.TimeoutSeconds, *input.Body.IntervalSeconds)
		}),
	}
}
func (handlers *cnkiHandlers) cnkiSession(request *http.Request, ctx context.Context, owner identity.Id, name string) (*output[storage.CnkiStatus], error) {
	status, failure := runCnki(request, handlers.pool, func() (storage.CnkiStatus, error) {
		if name == "clear_session" {
			if _, err := handlers.sessions.Clear(ctx, owner); err != nil {
				return storage.CnkiStatus{}, err
			}
		}
		return handlers.sessions.Status(ctx, owner)
	})
	if failure != nil {
		return nil, failure.problem()
	}
	return &output[storage.CnkiStatus]{Body: status}, nil
}

// startCnkiLogin publishes QR state only for the reserved generation.
func (handlers *cnkiHandlers) startCnkiLogin(request *http.Request, ctx context.Context, owner identity.Id) (*output[cnkiStartResponse], error) {
	generation, failure := runCnki(request, handlers.pool, func() (int64, error) { return handlers.sessions.Reserve(ctx, owner) })
	if failure != nil {
		return nil, failure.problem()
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
		state, err := jsonvalue.EncodeJson(client.StateData())
		return loginResult{qr, json.RawMessage(state)}, err
	})
	if failure != nil {
		return nil, failure.problem()
	}
	status, failure := runCnki(request, handlers.pool, func() (*storage.CnkiStatus, error) {
		return handlers.sessions.Complete(ctx, owner, generation, nil, login.state, "waiting_scan", &login.qr.Uuid)
	})
	if failure != nil {
		return nil, failure.problem()
	}
	if status == nil {
		return nil, cnkiSuperseded().problem()
	}
	return &output[cnkiStartResponse]{Body: cnkiStartResponse{UUID: login.qr.Uuid, Status: login.qr.Status, QRCode: login.qr.QrCode, Session: status}}, nil
}

// pollCnkiLogin preserves stored QR identity and generation-checked completion.
func (handlers *cnkiHandlers) pollCnkiLogin(request *http.Request, ctx context.Context, owner identity.Id, timeout int64, interval float64) (*output[cnkiPollResponse], error) {
	row, failure := runCnki(request, handlers.pool, func() (*storage.CnkiData, error) { return handlers.sessions.Data(ctx, owner, false) })
	if failure != nil {
		return nil, failure.problem()
	}
	if row == nil || strings.TrimSpace(row.QrUuid) == "" {
		return nil, cnkiFailure(400, "cnki_login_not_started", "login", "CNKI QR login has not been started").problem()
	}
	state, failure := runCnki(request, handlers.upstream, func() (json.RawMessage, error) {
		return handlers.pollCnkiClient(ctx, row, timeout, interval)
	})
	if failure != nil {
		return nil, failure.problem()
	}
	status, failure := runCnki(request, handlers.pool, func() (*storage.CnkiStatus, error) {
		var object map[string]json.RawMessage
		if err := json.Unmarshal(state, &object); err != nil {
			return nil, err
		}
		qr := row.QrUuid
		if raw, exists := object["qr_uuid"]; exists && len(raw) > 0 && raw[0] == '"' {
			if err := json.Unmarshal(raw, &qr); err != nil {
				return nil, err
			}
		}
		return handlers.sessions.Complete(ctx, owner, row.Generation, &row.QrUuid, state, "active", &qr)
	})
	if failure != nil {
		return nil, failure.problem()
	}
	if status == nil {
		return nil, cnkiSuperseded().problem()
	}
	return &output[cnkiPollResponse]{Body: cnkiPollResponse{Status: "COMPLETE", Session: status}}, nil
}

// parseCnkiSession adds a missing QR identity without replacing a stored field.
