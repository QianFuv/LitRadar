package api

import (
	"context"
	"errors"
	"net/http"
	"net/netip"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
)

const oidcCookieName = "litradar_oidc"

// handleOidc applies no-store, authentication and IP admission before OIDC network work.
func (handlers *authHandlers) handleOidc(writer http.ResponseWriter, request *http.Request, name string) {
	writer.Header().Set("Cache-Control", "no-store")
	writer.Header().Set("Referrer-Policy", "no-referrer")
	request = beginAuthAudit(request)
	var callbackState *auth.OidcState
	var callbackError error
	if name == "oidc_callback" {
		callbackError = auth.OidcError("state")
		if cookie, err := request.Cookie(oidcCookieName); err == nil && handlers.oidc != nil {
			callbackState, callbackError = handlers.oidc.ReadState(cookie.Value)
		}
		if callbackState == nil || callbackState.Native == nil {
			handlers.writeOidcCookie(writer, "", true)
		}
	}
	if name == "get_auth_providers" {
		if handlers.oidc == nil {
			writeResponse(writer, map[string]any{"oidc_enabled": false, "oidc_display_name": "authentik"})
			return
		}
		type providers struct {
			Enabled     bool   `json:"oidc_enabled"`
			DisplayName string `json:"oidc_display_name"`
		}
		result, failure := runAuth(request, handlers.pool, func() (providers, error) {
			enabled, name, err := handlers.oidc.Providers(request.Context())
			return providers{enabled, name}, err
		})
		if failure != nil {
			failure.write(writer)
			return
		}
		writeResponse(writer, result)
		return
	}
	action := "oidc_login"
	if name == "oidc_link" || name == "oidc_unlink" {
		action = name
	}
	event := domain.AuditEvent{Action: action, Outcome: "completed", RequestId: requestId(request), OccurredAt: currentTimestamp()}
	if name != "oidc_unlink" {
		peer, _ := netip.ParseAddrPort(request.RemoteAddr)
		if rejection := handlers.limiter.check(oidcAttempt, resolveAuthClientSource(peer.Addr(), request.Header, handlers.trustedProxies), ""); rejection != nil {
			handlers.respondAuthRateLimit(writer, request, &event, rejection)
			return
		}
	}
	if handlers.oidc == nil {
		if name == "oidc_native_login" {
			(&apiError{status: 404, detail: "Single sign-on is unavailable"}).write(writer)
			return
		}
		if name == "oidc_login" || name == "oidc_callback" {
			http.Redirect(writer, request, "/login?oidc_error=unavailable", http.StatusFound)
		} else {
			badRequest("Single sign-on is unavailable").write(writer)
		}
		return
	}
	switch name {
	case "oidc_callback":
		handlers.oidcCallback(writer, request, event, callbackState, callbackError)
	case "oidc_login", "oidc_link", "oidc_native_login":
		handlers.oidcStart(writer, request, name, event)
	case "oidc_unlink":
		current, failure := handlers.authenticator.requireUser(request)
		if failure != nil {
			handlers.reject(request, event, "authentication_required", failure).write(writer)
			return
		}
		actor := int64(current.authorization.User.Id)
		event.ActorId = &actor
		user, failure := runAuth(request, handlers.pool, func() (domain.User, error) {
			return handlers.oidc.Unlink(context.WithoutCancel(request.Context()), current.authorization, event)
		})
		if failure != nil {
			handlers.reject(request, event, "operation_failed", failure).write(writer)
			return
		}
		logAuth(request, event, "")
		writeResponse(writer, publicUser(user))
	}
}

func (handlers *authHandlers) writeOidcCookie(writer http.ResponseWriter, value string, shouldClear bool) {
	cookie := &http.Cookie{Name: oidcCookieName, Value: value, Path: "/api/auth/oidc", HttpOnly: true, Secure: handlers.isCookieSecure, SameSite: http.SameSiteLaxMode, MaxAge: 600, Expires: time.Now().Add(10 * time.Minute)}
	if shouldClear {
		cookie.MaxAge = -1
		cookie.Expires = time.Unix(1, 0)
	}
	http.SetCookie(writer, cookie)
}

func (handlers *authHandlers) oidcSession(request *http.Request) (currentIdentity, *apiError) {
	value, ok := sessionCookie(request.Header)
	if !ok || value == "" {
		return currentIdentity{}, &apiError{status: 401, detail: "Authentication required"}
	}
	clone := request.Clone(request.Context())
	clone.Header = request.Header.Clone()
	clone.Header.Del("Authorization")
	return handlers.authenticator.requireUser(clone)
}

func (handlers *authHandlers) oidcStart(writer http.ResponseWriter, request *http.Request, name string, event domain.AuditEvent) {
	var link *domain.Authorization
	var native *auth.NativeOIDCParameters
	if name == "oidc_native_login" {
		query := request.URL.Query()
		native = &auth.NativeOIDCParameters{RedirectURI: query.Get("redirect_uri"), CodeChallenge: query.Get("code_challenge"), State: query.Get("state")}
		if len(query["redirect_uri"]) != 1 || len(query["code_challenge"]) != 1 || len(query["state"]) != 1 || !native.Valid() {
			badRequest("invalid_request").write(writer)
			return
		}
	}
	if name == "oidc_link" {
		current, failure := handlers.oidcSession(request)
		if failure != nil {
			handlers.reject(request, event, "authentication_required", failure).write(writer)
			return
		}
		actor := int64(current.authorization.User.Id)
		event.ActorId = &actor
		if current.authorization.User.AuthKind != "local" || current.authorization.User.IsOidcLinked {
			handlers.reject(request, event, "account_conflict", badRequest("account_conflict")).write(writer)
			return
		}
		link = &current.authorization
	}
	type start struct {
		url, cookie string
		err         error
	}
	result, failure := runAuth(request, handlers.oidcPool, func() (start, error) {
		if native != nil {
			target, cookie, err := handlers.oidc.StartNative(request.Context(), *native)
			return start{target, cookie, err}, nil
		}
		target, cookie, err := handlers.oidc.Start(request.Context(), request.URL.Query().Get("next"), link)
		return start{target, cookie, err}, nil
	})
	if failure != nil || result.err != nil {
		code := oidcErrorCode(result.err)
		if native != nil && code == "unavailable" {
			(&apiError{status: 404, detail: "Single sign-on is unavailable"}).write(writer)
			return
		}
		if link != nil {
			if failure == nil {
				failure = serviceUnavailable()
			}
			handlers.reject(request, event, code, failure).write(writer)
		} else {
			var state *auth.OidcState
			if native != nil {
				state = &auth.OidcState{Native: native}
			}
			handlers.oidcFailure(writer, request, state, event, code)
		}
		return
	}
	handlers.writeOidcCookie(writer, result.cookie, false)
	if link != nil {
		writeResponse(writer, map[string]string{"authorization_url": result.url})
	} else {
		http.Redirect(writer, request, result.url, http.StatusFound)
	}
}

func (handlers *authHandlers) oidcCallback(writer http.ResponseWriter, request *http.Request, event domain.AuditEvent, state *auth.OidcState, err error) {
	if state != nil && state.UserId != 0 {
		event.Action = "oidc_link"
		actor := int64(state.UserId)
		event.ActorId = &actor
	}
	if err != nil {
		handlers.oidcFailure(writer, request, state, event, "state")
		return
	}
	query := request.URL.Query()
	if len(query["state"]) != 1 || !auth.MatchOidcState(state.State, query.Get("state")) {
		handlers.oidcFailure(writer, request, state, event, "state")
		return
	}
	if value := query.Get("error"); value != "" {
		reason := "provider"
		if value == "access_denied" {
			reason = "denied"
		}
		handlers.oidcFailure(writer, request, state, event, reason)
		return
	}
	if len(query["code"]) != 1 || query.Get("code") == "" {
		handlers.oidcFailure(writer, request, state, event, "provider")
		return
	}
	var link *domain.Authorization
	if state.UserId != 0 {
		current, failure := handlers.oidcSession(request)
		if failure != nil || current.authorization.User.Id != state.UserId || current.authorization.TokenGeneration != state.TokenGeneration {
			handlers.oidcFailure(writer, request, state, event, "expired_session")
			return
		}
		link = &current.authorization
	}
	type completion struct {
		session domain.LoginSession
		code    string
		err     error
	}
	result, failure := runAuth(request, handlers.oidcPool, func() (completion, error) {
		if state.Native != nil {
			code, user, err := handlers.oidc.FinishNative(request.Context(), *state, query.Get("code"), event)
			return completion{session: domain.LoginSession{User: user}, code: code, err: err}, nil
		}
		session, err := handlers.oidc.Finish(request.Context(), *state, query.Get("code"), link, event)
		return completion{session: session, err: err}, nil
	})
	if failure != nil {
		handlers.oidcFailure(writer, request, state, event, "provider")
		return
	}
	if result.err != nil {
		handlers.oidcFailure(writer, request, state, event, oidcErrorCode(result.err))
		return
	}
	actor := int64(result.session.User.Id)
	event.ActorId = &actor
	logAuth(request, event, "")
	if state.Native != nil {
		http.Redirect(writer, request, state.Native.Redirect("code", result.code), http.StatusFound)
		return
	}
	if link == nil {
		writer.Header().Add("Set-Cookie", sessionCookieHeader(result.session.Token, result.session.ExpiresAt, currentTimestamp(), handlers.isCookieSecure))
	}
	http.Redirect(writer, request, auth.SafeOidcNext(state.Next), http.StatusFound)
}

func oidcErrorCode(err error) string {
	var failure auth.OidcError
	if errors.As(err, &failure) {
		switch string(failure) {
		case "unavailable", "state", "denied", "provider", "email_unverified", "email_conflict", "account_conflict", "expired_session":
			return string(failure)
		}
	}
	return "provider"
}

func (handlers *authHandlers) oidcFailure(writer http.ResponseWriter, request *http.Request, state *auth.OidcState, event domain.AuditEvent, code string) {
	event.Outcome, event.Reason = "rejected", code
	if failure := handlers.persistAudit(request, event); failure != nil {
		failure.write(writer)
		return
	}
	logAuth(request, event, code)
	target := "/login?oidc_error=" + code
	if state != nil && state.UserId != 0 {
		target = "/?oidc_link=" + code
	}
	if state != nil && state.Native != nil && state.Native.Valid() {
		target = state.Native.Redirect("error", code)
	}
	http.Redirect(writer, request, target, http.StatusFound)
}
