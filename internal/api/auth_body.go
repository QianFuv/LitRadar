package api

import (
	"context"
	"log/slog"
	"math"
	"net/http"
	"net/netip"

	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/platform/cryptography"
)

var authRequestTypes = map[string]string{"register": "RegisterRequest", "login": "LoginRequest", "change_password": "ChangePasswordRequest", "create_token": "TokenCreateRequest"}

// handleBody decodes before auditing and dispatches authentication phases.
func (handlers *authHandlers) handleBody(writer http.ResponseWriter, request *http.Request, name, bodyName string) {
	decoded, failure := extractBody(request, requestBodies[bodyName], false)
	if failure != nil {
		failure.write(writer)
		return
	}
	request = beginAuthAudit(request)
	body := decoded.(map[string]any)
	action := name
	if name == "change_password" {
		action = "password_change"
	}
	if name == "create_token" {
		action = "token_create"
	}
	event := domain.AuditEvent{Action: action, Outcome: "completed", RequestId: requestId(request)}

	ctx := context.WithoutCancel(request.Context())
	var payload any
	var isCompleted bool
	if name == "login" || name == "register" {
		payload, isCompleted = handlers.authBodyAttempt(writer, request, ctx, name, body, &event)
	} else {
		payload, isCompleted = handlers.authCredentialBody(writer, request, ctx, name, body, &event)
	}
	if !isCompleted {
		return
	}
	logAuth(request, event, "")
	writeResponse(writer, payload)
}

// authBodyAttempt consumes rate limits before registration or login validation.
func (handlers *authHandlers) authBodyAttempt(writer http.ResponseWriter, request *http.Request, ctx context.Context, name string, body map[string]any, event *domain.AuditEvent) (any, bool) {
	kind := loginAttempt
	if name == "register" {
		kind = registerAttempt
	}
	username := strings.TrimSpace(body["username"].(string))
	password := body["password"].(string)
	peer, _ := netip.ParseAddrPort(request.RemoteAddr)

	if rejection := handlers.limiter.check(kind, resolveAuthClientSource(peer.Addr(), request.Header, handlers.trustedProxies), username); rejection != nil {
		handlers.respondAuthRateLimit(writer, request, event, rejection)
		return nil, false
	}
	var payload any
	var isCompleted bool
	if name == "register" {
		payload, isCompleted = handlers.registerAuthUser(writer, request, ctx, body, username, password, event)
	} else {
		payload, isCompleted = handlers.loginAuthUser(writer, request, ctx, username, password, event)
	}
	if !isCompleted {
		return nil, false
	}
	handlers.limiter.clearUsername(kind, username)
	return payload, true
}

// respondAuthRateLimit preserves conditional audit persistence and capped counters.
func (handlers *authHandlers) respondAuthRateLimit(writer http.ResponseWriter, request *http.Request, event *domain.AuditEvent, rejection *rateLimitRejection) {
	limited := *event
	limited.Outcome, limited.Reason = "rate_limited", rejection.reason
	limited.SourceClass, limited.Bucket = rejection.sourceClass, rejection.bucket
	limited.RejectedCount = int64(min(rejection.rejectedCount, math.MaxInt64))
	limited.RetryAfterSeconds = int64(min(rejection.retryAfter, math.MaxInt64))
	if rejection.shouldPersistAudit {
		if failure := handlers.persistAudit(request, limited); failure != nil {
			logAuth(request, *event, "operation_failed")
			failure.write(writer)
			return
		}
	}
	started, _ := request.Context().Value(auditStartKey{}).(time.Time)
	slog.WarnContext(request.Context(), "security.auth.rate_limited", "event", "security.auth.rate_limited", "component", "security", "action", event.Action, "outcome", "rate_limited", "actor_id", 0, "target_id", 0, "reason", rejection.reason, "bucket", rejection.bucket, "source_class", rejection.sourceClass, "rejected_count", rejection.rejectedCount, "request_id", event.RequestId, "retry_after_seconds", rejection.retryAfter, "duration_ms", time.Since(started).Milliseconds())
	(&apiError{status: 429, detail: "Too many authentication attempts; try again later", retryAfter: &rejection.retryAfter}).write(writer)
	return
}

// registerAuthUser validates credentials before registering through the bounded KDF pool.
func (handlers *authHandlers) registerAuthUser(writer http.ResponseWriter, request *http.Request, ctx context.Context, body map[string]any, username, password string, event *domain.AuditEvent) (any, bool) {
	var payload any

	reject := func(reason string, failure *apiError) {
		handlers.reject(request, *event, reason, failure).write(writer)
	}
	if !auth.ValidUsername(username) {
		reject("validation_failed", mapAuthError(domain.ErrUsername))
		return nil, false
	}
	if !cryptography.ValidNewPassword(password) {
		reject("validation_failed", mapAuthError(domain.ErrPasswordShort))
		return nil, false
	}
	invite := body["invite_code"].(string)
	var inviteCode *string
	if invite != "" {
		inviteCode = &invite
	}
	event.OccurredAt = currentTimestamp()
	user, err := runAuth(request, handlers.kdfPool, func() (domain.User, error) {
		return handlers.service.Register(ctx, username, password, inviteCode, event)
	})
	if err != nil {
		reject("registration_failed", err)
		return nil, false
	}
	actor := int64(user.Id)
	event.ActorId = &actor
	payload = publicUser(user)
	return payload, true
}

// loginAuthUser sets the session cookie only after successful KDF work.
func (handlers *authHandlers) loginAuthUser(writer http.ResponseWriter, request *http.Request, ctx context.Context, username, password string, event *domain.AuditEvent) (any, bool) {
	var payload any

	reject := func(reason string, failure *apiError) {
		handlers.reject(request, *event, reason, failure).write(writer)
	}
	event.OccurredAt = currentTimestamp()
	session, err := runAuth(request, handlers.kdfPool, func() (domain.LoginSession, error) { return handlers.service.Login(ctx, username, password, event) })
	if err != nil {
		reject("authentication_failed", err)
		return nil, false
	}
	actor := int64(session.User.Id)
	event.ActorId = &actor
	payload = struct {
		User      userResponse `json:"user"`
		ExpiresAt float64      `json:"expires_at"`
	}{publicUser(session.User), session.ExpiresAt}
	writer.Header().Add("Set-Cookie", sessionCookieHeader(session.Token, session.ExpiresAt, currentTimestamp(), handlers.isCookieSecure))
	return payload, true
}

// authCredentialBody checks new-password policy before authenticating the current user.
func (handlers *authHandlers) authCredentialBody(writer http.ResponseWriter, request *http.Request, ctx context.Context, name string, body map[string]any, event *domain.AuditEvent) (any, bool) {

	reject := func(reason string, failure *apiError) {
		handlers.reject(request, *event, reason, failure).write(writer)
	}
	if name == "change_password" && !cryptography.ValidNewPassword(body["new_password"].(string)) {
		reject("validation_failed", mapAuthError(domain.ErrPasswordShort))
		return nil, false
	}
	current, err := handlers.authenticator.requireUser(request)
	if err != nil {
		reject("authentication_required", err)
		return nil, false
	}
	actor := int64(current.authorization.User.Id)
	event.ActorId = &actor
	event.OccurredAt = currentTimestamp()

	if name == "create_token" {
		return handlers.createAuthToken(writer, request, ctx, current, body, event)
	}
	return handlers.changeAuthPassword(writer, request, ctx, current, body, event)
}

// createAuthToken records the issued token as the audit target after successful storage work.
func (handlers *authHandlers) createAuthToken(writer http.ResponseWriter, request *http.Request, ctx context.Context, current currentIdentity, body map[string]any, event *domain.AuditEvent) (any, bool) {
	var payload any

	reject := func(reason string, failure *apiError) {
		handlers.reject(request, *event, reason, failure).write(writer)
	}
	token, err := runAuth(request, handlers.pool, func() (domain.IssuedToken, error) {
		return handlers.service.CreateToken(ctx, current.authorization, body["name"].(string), body["ttl"].(int64), event)
	})
	if err != nil {
		reject("operation_failed", err)
		return nil, false
	}
	event.TargetId = &token.Id
	payload = token
	return payload, true
}

// changeAuthPassword distinguishes execution failure from an incorrect old password.
func (handlers *authHandlers) changeAuthPassword(writer http.ResponseWriter, request *http.Request, ctx context.Context, current currentIdentity, body map[string]any, event *domain.AuditEvent) (any, bool) {
	var payload any

	reject := func(reason string, failure *apiError) {
		handlers.reject(request, *event, reason, failure).write(writer)
	}
	changed, err := runAuth(request, handlers.kdfPool, func() (bool, error) {
		return handlers.service.ChangePassword(ctx, current.authorization.User.Id, body["old_password"].(string), body["new_password"].(string), event)
	})
	if err != nil {
		reject("operation_failed", err)
		return nil, false
	}
	if !changed {
		reject("authentication_failed", &apiError{status: 400, detail: "Old password is incorrect"})
		return nil, false
	}
	payload = map[string]bool{"ok": true}
	return payload, true
}
