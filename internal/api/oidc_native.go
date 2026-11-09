package api

import (
	"context"
	"errors"
	"net/http"
	"net/netip"

	"github.com/QianFuv/LitRadar/internal/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
)

// nativeOIDCExchange applies the auth IP/global budgets before consuming a one-time grant.
func (handlers *authHandlers) nativeOIDCExchange(writer http.ResponseWriter, request *http.Request) {
	writer.Header().Set("Cache-Control", "no-store")
	writer.Header().Set("Referrer-Policy", "no-referrer")
	request = beginAuthAudit(request)
	event := domain.AuditEvent{Action: "oidc_native_exchange", Outcome: "completed", RequestId: requestId(request), OccurredAt: currentTimestamp()}
	peer, _ := netip.ParseAddrPort(request.RemoteAddr)
	if rejection := handlers.limiter.check(oidcAttempt, resolveAuthClientSource(peer.Addr(), request.Header, handlers.trustedProxies), ""); rejection != nil {
		handlers.respondAuthRateLimit(writer, request, &event, rejection)
		return
	}
	decoded, failure := extractBody(request, requestBodies["NativeOidcExchangeRequest"], false)
	if failure != nil {
		failure.write(writer)
		return
	}
	body := decoded.(map[string]any)
	if handlers.oidc == nil {
		handlers.reject(request, event, "invalid_grant", badRequest("invalid_grant")).write(writer)
		return
	}
	type result struct {
		session domain.LoginSession
		err     error
	}
	completion, failure := runAuth(request, handlers.pool, func() (result, error) {
		session, err := handlers.oidc.ExchangeNative(context.WithoutCancel(request.Context()), body["code"].(string), body["code_verifier"].(string), body["redirect_uri"].(string), body["device_name"].(string), event)
		return result{session, err}, nil
	})
	if completion.err != nil {
		if errors.Is(completion.err, auth.OidcError("invalid_grant")) {
			failure = badRequest("invalid_grant")
		} else {
			failure = mapAuthError(completion.err)
		}
	}
	if failure != nil {
		handlers.reject(request, event, "invalid_grant", failure).write(writer)
		return
	}
	session := completion.session
	actor := int64(session.User.Id)
	event.ActorId = &actor
	logAuth(request, event, "")
	writeResponse(writer, struct {
		Token     string       `json:"token"`
		ExpiresAt float64      `json:"expires_at"`
		User      userResponse `json:"user"`
	}{session.Token, session.ExpiresAt, publicUser(session.User)})
}
