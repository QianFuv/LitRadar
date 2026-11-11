package api

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"

	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/sources/zjlib"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
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

// parseCnkiSession adds a missing QR identity without replacing a stored field.
func parseCnkiSession(row *storage.CnkiData) (any, error) {
	data, err := transport.ParseJson(row.SessionData)
	if err != nil {
		return nil, err
	}
	if object, ok := data.(map[string]any); ok {
		if _, exists := object["qr_uuid"]; !exists {
			object["qr_uuid"] = row.QrUuid
		}
	}

	return data, nil
}

// pollCnkiClient retains timeout classification and full-text warm-up before persistence.
func (handlers *cnkiHandlers) pollCnkiClient(ctx context.Context, row *storage.CnkiData, timeout int64, interval float64) (json.RawMessage, error) {
	data, err := parseCnkiSession(row)
	if err != nil {
		return nil, err
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
	encoded, err := jsonvalue.EncodeJson(client.StateData())
	return json.RawMessage(encoded), err
}
