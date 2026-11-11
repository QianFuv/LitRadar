package api

import (
	"context"
	"errors"
	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
)

func (handlers *authHandlers) nativeOIDCExchange(ctx context.Context, input *OIDCNativeExchangeInput) (*oidcOutput[nativeLoginResponse], error) {
	request, event, err := handlers.oidcAdmission(input.request, "oidc_native_exchange")
	if err != nil {
		return nil, oidcProblem(err, nil)
	}
	if handlers.oidc == nil {
		return nil, oidcProblem(handlers.reject(request, event, "invalid_grant", badRequest("invalid_grant")).problem(), nil)
	}
	type completion struct {
		session domain.LoginSession
		err     error
	}
	result, failure := runAuth(request, handlers.pool, func() (completion, error) {
		session, err := handlers.oidc.ExchangeNative(context.WithoutCancel(ctx), input.Body.Code, input.Body.CodeVerifier, input.Body.RedirectURI, input.Body.DeviceName, event)
		return completion{session: session, err: err}, nil
	})
	if result.err != nil {
		if errors.Is(result.err, auth.OidcError("invalid_grant")) {
			failure = badRequest("invalid_grant")
		} else {
			failure = mapAuthError(result.err)
		}
	}
	if failure != nil {
		return nil, oidcProblem(handlers.reject(request, event, "invalid_grant", failure).problem(), nil)
	}
	session := result.session
	actor := int64(session.User.Id)
	event.ActorId = &actor
	logAuth(request, event, "")
	return &oidcOutput[nativeLoginResponse]{OIDCHeaders: privateOIDCHeaders(), Body: nativeLoginResponse{Token: session.Token, ExpiresAt: session.ExpiresAt, User: publicUser(session.User)}}, nil
}
