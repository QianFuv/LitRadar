package api

import (
	"context"
	"errors"
	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/danielgtaylor/huma/v2"
	"html"
	"net/http"
	"net/netip"
	"time"
)

const oidcCookieName = "litradar_oidc"

// OIDCHeaders carries cache, privacy and flow-cookie response policy.
type OIDCHeaders struct {
	// SetCookie sets the Set-Cookie header.
	SetCookie []string `header:"Set-Cookie"`
	// CacheControl sets the Cache-Control header.
	CacheControl string `header:"Cache-Control"`
	// ReferrerPolicy sets the Referrer-Policy header.
	ReferrerPolicy string `header:"Referrer-Policy"`
}
type oidcOutput[Body any] struct {
	OIDCHeaders
	// Body contains the decoded operation payload.
	Body Body
}
type redirectOutput struct {
	OIDCHeaders
	// Body contains the decoded operation payload.
	Body []byte
	// Location sets the Location header.
	Location string `header:"Location"`
	// ContentType sets the Content-Type header.
	ContentType string `header:"Content-Type"`
}
type authProvidersResponse struct {
	// Enabled contains the oidc enabled value.
	Enabled bool `json:"oidc_enabled" nullable:"false"`
	// DisplayName contains the oidc display name value.
	DisplayName string `json:"oidc_display_name" nullable:"false"`
}
type oidcLinkResponse struct {
	// AuthorizationURL contains the authorization url value.
	AuthorizationURL string `json:"authorization_url" nullable:"false"`
}
type oidcStartResult struct{ target, cookie string }

func privateOIDCHeaders() OIDCHeaders {
	return OIDCHeaders{CacheControl: "no-store", ReferrerPolicy: "no-referrer"}
}
func redirect(target string, cookies []string) *redirectOutput {
	headers := privateOIDCHeaders()
	headers.SetCookie = cookies
	return &redirectOutput{OIDCHeaders: headers, Location: target, ContentType: "text/html; charset=utf-8", Body: []byte("<a href=\"" + html.EscapeString(target) + "\">Found</a>.\n\n")}
}
func oidcProblem(err error, cookies []string) error {
	return huma.ErrorWithHeaders(err, http.Header{"Cache-Control": {"no-store"}, "Referrer-Policy": {"no-referrer"}, "Set-Cookie": cookies})
}
func (handlers *authHandlers) oidcCookie(value string, shouldClear bool) string {
	cookie := http.Cookie{Name: oidcCookieName, Value: value, Path: "/api/auth/oidc", HttpOnly: true, Secure: handlers.isCookieSecure, SameSite: http.SameSiteLaxMode, MaxAge: 600, Expires: time.Now().Add(10 * time.Minute)}
	if shouldClear {
		cookie.MaxAge = -1
		cookie.Expires = time.Unix(1, 0)
	}
	return cookie.String()
}
func (handlers *authHandlers) authProviders(ctx context.Context, input *GetAuthProvidersInput) (*oidcOutput[authProvidersResponse], error) {
	value := authProvidersResponse{DisplayName: "authentik"}
	if handlers.oidc != nil {
		result, failure := runAuth(input.request, handlers.pool, func() (authProvidersResponse, error) {
			enabled, name, err := handlers.oidc.Providers(ctx)
			return authProvidersResponse{Enabled: enabled, DisplayName: name}, err
		})
		if failure != nil {
			return nil, oidcProblem(failure.problem(), nil)
		}
		value = result
	}
	return &oidcOutput[authProvidersResponse]{OIDCHeaders: privateOIDCHeaders(), Body: value}, nil
}
func (handlers *authHandlers) oidcAdmission(request *http.Request, action string) (*http.Request, domain.AuditEvent, error) {
	request = beginAuthAudit(request)
	event := domain.AuditEvent{Action: action, Outcome: "completed", RequestId: requestId(request), OccurredAt: currentTimestamp()}
	peer, _ := netip.ParseAddrPort(request.RemoteAddr)
	if rejection := handlers.limiter.check(oidcAttempt, resolveAuthClientSource(peer.Addr(), request.Header, handlers.trustedProxies), ""); rejection != nil {
		return request, event, handlers.authRateLimit(request, &event, rejection).problem()
	}
	return request, event, nil
}
func (handlers *authHandlers) oidcLogin(ctx context.Context, input *OIDCLoginInput) (*redirectOutput, error) {
	request, event, err := handlers.oidcAdmission(input.request, "oidc_login")
	if err != nil {
		return nil, oidcProblem(err, nil)
	}
	if handlers.oidc == nil {
		return redirect("/login?oidc_error=unavailable", nil), nil
	}
	result, code, failure := handlers.startOIDC(request, input.Next, nil, nil)
	if failure != nil {
		return handlers.oidcFailure(request, nil, event, code, nil)
	}
	return redirect(result.target, []string{handlers.oidcCookie(result.cookie, false)}), nil
}
func (handlers *authHandlers) oidcNativeLogin(ctx context.Context, input *OIDCNativeLoginInput) (*redirectOutput, error) {
	request, event, err := handlers.oidcAdmission(input.request, "oidc_login")
	if err != nil {
		return nil, oidcProblem(err, nil)
	}
	native := auth.NativeOIDCParameters{RedirectURI: input.RedirectURI[0], CodeChallenge: input.CodeChallenge[0], State: input.State[0]}
	if !native.Valid() {
		return nil, oidcProblem(badRequest("invalid_request").problem(), nil)
	}
	if handlers.oidc == nil {
		return nil, oidcProblem((&apiError{status: 404, detail: "Single sign-on is unavailable"}).problem(), nil)
	}
	result, code, failure := handlers.startOIDC(request, "", nil, &native)
	if failure != nil {
		if code == "unavailable" {
			return nil, oidcProblem((&apiError{status: 404, detail: "Single sign-on is unavailable"}).problem(), nil)
		}
		return handlers.oidcFailure(request, &auth.OidcState{Native: &native}, event, code, nil)
	}
	return redirect(result.target, []string{handlers.oidcCookie(result.cookie, false)}), nil
}
func (handlers *authHandlers) oidcLink(ctx context.Context, input *OIDCLinkInput) (*oidcOutput[oidcLinkResponse], error) {
	request, event, err := handlers.oidcAdmission(input.request, "oidc_link")
	if err != nil {
		return nil, oidcProblem(err, nil)
	}
	if handlers.oidc == nil {
		return nil, oidcProblem(badRequest("Single sign-on is unavailable").problem(), nil)
	}
	current, failure := handlers.oidcSession(request)
	if failure != nil {
		return nil, oidcProblem(handlers.reject(request, event, "authentication_required", failure).problem(), nil)
	}
	actor := int64(current.authorization.User.Id)
	event.ActorId = &actor
	if current.authorization.User.AuthKind != "local" || current.authorization.User.IsOidcLinked {
		return nil, oidcProblem(handlers.reject(request, event, "account_conflict", badRequest("account_conflict")).problem(), nil)
	}
	result, code, failure := handlers.startOIDC(request, "", &current.authorization, nil)
	if failure != nil {
		return nil, oidcProblem(handlers.reject(request, event, code, failure).problem(), nil)
	}
	headers := privateOIDCHeaders()
	headers.SetCookie = []string{handlers.oidcCookie(result.cookie, false)}
	return &oidcOutput[oidcLinkResponse]{OIDCHeaders: headers, Body: oidcLinkResponse{AuthorizationURL: result.target}}, nil
}
func (handlers *authHandlers) startOIDC(request *http.Request, next string, link *domain.Authorization, native *auth.NativeOIDCParameters) (oidcStartResult, string, *apiError) {
	type start struct {
		result oidcStartResult
		err    error
	}
	result, failure := runAuth(request, handlers.oidcPool, func() (start, error) {
		var target, cookie string
		var err error
		if native != nil {
			target, cookie, err = handlers.oidc.StartNative(request.Context(), *native)
		} else {
			target, cookie, err = handlers.oidc.Start(request.Context(), next, link)
		}
		return start{result: oidcStartResult{target: target, cookie: cookie}, err: err}, nil
	})
	if failure != nil {
		return oidcStartResult{}, "provider", failure
	}
	if result.err != nil {
		return oidcStartResult{}, oidcErrorCode(result.err), serviceUnavailable()
	}
	return result.result, "", nil
}
func (handlers *authHandlers) oidcUnlink(ctx context.Context, input *OIDCUnlinkInput) (*oidcOutput[userResponse], error) {
	request := beginAuthAudit(input.request)
	event := domain.AuditEvent{Action: "oidc_unlink", Outcome: "completed", RequestId: requestId(request), OccurredAt: currentTimestamp()}
	if handlers.oidc == nil {
		return nil, oidcProblem(badRequest("Single sign-on is unavailable").problem(), nil)
	}
	current, failure := handlers.authenticator.requireUser(request)
	if failure != nil {
		return nil, oidcProblem(handlers.reject(request, event, "authentication_required", failure).problem(), nil)
	}
	actor := int64(current.authorization.User.Id)
	event.ActorId = &actor
	user, failure := runAuth(request, handlers.pool, func() (domain.User, error) {
		return handlers.oidc.Unlink(context.WithoutCancel(ctx), current.authorization, event)
	})
	if failure != nil {
		return nil, oidcProblem(handlers.reject(request, event, "operation_failed", failure).problem(), nil)
	}
	logAuth(request, event, "")
	return &oidcOutput[userResponse]{OIDCHeaders: privateOIDCHeaders(), Body: publicUser(user)}, nil
}
func (handlers *authHandlers) oidcCallback(ctx context.Context, input *OIDCCallbackInput) (*redirectOutput, error) {
	request := input.request
	var state *auth.OidcState
	var stateError error = auth.OidcError("state")
	if cookie, err := request.Cookie(oidcCookieName); err == nil && handlers.oidc != nil {
		state, stateError = handlers.oidc.ReadState(cookie.Value)
	}
	cookies := []string{}
	if state == nil || state.Native == nil {
		cookies = append(cookies, handlers.oidcCookie("", true))
	}
	request, event, err := handlers.oidcAdmission(request, "oidc_login")
	if err != nil {
		return nil, oidcProblem(err, cookies)
	}
	if handlers.oidc == nil {
		return redirect("/login?oidc_error=unavailable", cookies), nil
	}
	if state != nil && state.UserId != 0 {
		event.Action = "oidc_link"
		actor := int64(state.UserId)
		event.ActorId = &actor
	}
	fail := func(code string) (*redirectOutput, error) {
		return handlers.oidcFailure(request, state, event, code, cookies)
	}
	if stateError != nil || len(input.State) != 1 || !auth.MatchOidcState(state.State, input.State[0]) {
		return fail("state")
	}
	if input.Error.Pointer() != nil && *input.Error.Pointer() != "" {
		if *input.Error.Pointer() == "access_denied" {
			return fail("denied")
		}
		return fail("provider")
	}
	if len(input.Code) != 1 || input.Code[0] == "" {
		return fail("provider")
	}
	var link *domain.Authorization
	if state.UserId != 0 {
		current, failure := handlers.oidcSession(request)
		if failure != nil || current.authorization.User.Id != state.UserId || current.authorization.TokenGeneration != state.TokenGeneration {
			return fail("expired_session")
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
			code, user, err := handlers.oidc.FinishNative(ctx, *state, input.Code[0], event)
			return completion{session: domain.LoginSession{User: user}, code: code, err: err}, nil
		}
		session, err := handlers.oidc.Finish(ctx, *state, input.Code[0], link, event)
		return completion{session: session, err: err}, nil
	})
	if failure != nil {
		return fail("provider")
	}
	if result.err != nil {
		return fail(oidcErrorCode(result.err))
	}
	actor := int64(result.session.User.Id)
	event.ActorId = &actor
	logAuth(request, event, "")
	if state.Native != nil {
		return redirect(state.Native.Redirect("code", result.code), cookies), nil
	}
	if link == nil {
		cookies = append(cookies, sessionCookieHeader(result.session.Token, result.session.ExpiresAt, currentTimestamp(), handlers.isCookieSecure))
	}
	return redirect(auth.SafeOidcNext(state.Next), cookies), nil
}
func (handlers *authHandlers) oidcFailure(request *http.Request, state *auth.OidcState, event domain.AuditEvent, code string, cookies []string) (*redirectOutput, error) {
	event.Outcome, event.Reason = "rejected", code
	if failure := handlers.persistAudit(request, event); failure != nil {
		return nil, oidcProblem(failure.problem(), cookies)
	}
	logAuth(request, event, code)
	target := "/login?oidc_error=" + code
	if state != nil && state.UserId != 0 {
		target = "/?oidc_link=" + code
	}
	if state != nil && state.Native != nil && state.Native.Valid() {
		target = state.Native.Redirect("error", code)
	}
	return redirect(target, cookies), nil
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
