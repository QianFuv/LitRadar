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
	reject := func(reason string, failure *apiError) { handlers.reject(request, event, reason, failure).write(writer) }
	ctx := context.WithoutCancel(request.Context())
	var payload any
	if name == "login" || name == "register" {
		kind := loginAttempt
		if name == "register" {
			kind = registerAttempt
		}
		username := strings.TrimSpace(body["username"].(string))
		password := body["password"].(string)
		peer, _ := netip.ParseAddrPort(request.RemoteAddr)
		if rejection := handlers.limiter.check(kind, resolveAuthClientSource(peer.Addr(), request.Header, handlers.trustedProxies), username); rejection != nil {
			limited := event
			limited.Outcome, limited.Reason = "rate_limited", rejection.reason
			limited.SourceClass, limited.Bucket = rejection.sourceClass, rejection.bucket
			limited.RejectedCount = int64(min(rejection.rejectedCount, math.MaxInt64))
			limited.RetryAfterSeconds = int64(min(rejection.retryAfter, math.MaxInt64))
			if rejection.shouldPersistAudit {
				if failure = handlers.persistAudit(request, limited); failure != nil {
					logAuth(request, event, "operation_failed")
					failure.write(writer)
					return
				}
			}
			started, _ := request.Context().Value(auditStartKey{}).(time.Time)
			slog.WarnContext(request.Context(), "security.auth.rate_limited", "event", "security.auth.rate_limited", "component", "security", "action", action, "outcome", "rate_limited", "actor_id", 0, "target_id", 0, "reason", rejection.reason, "bucket", rejection.bucket, "source_class", rejection.sourceClass, "rejected_count", rejection.rejectedCount, "request_id", event.RequestId, "retry_after_seconds", rejection.retryAfter, "duration_ms", time.Since(started).Milliseconds())
			(&apiError{status: 429, detail: "Too many authentication attempts; try again later", retryAfter: &rejection.retryAfter}).write(writer)
			return
		}
		if name == "register" {
			if !auth.ValidUsername(username) {
				reject("validation_failed", mapAuthError(domain.ErrUsername))
				return
			}
			if !cryptography.ValidNewPassword(password) {
				reject("validation_failed", mapAuthError(domain.ErrPasswordShort))
				return
			}
			invite := body["invite_code"].(string)
			var inviteCode *string
			if invite != "" {
				inviteCode = &invite
			}
			event.OccurredAt = currentTimestamp()
			user, err := runAuth(request, handlers.kdfPool, func() (domain.User, error) {
				return handlers.service.Register(ctx, username, password, inviteCode, &event)
			})
			if err != nil {
				reject("registration_failed", err)
				return
			}
			actor := int64(user.Id)
			event.ActorId = &actor
			payload = publicUser(user)
		} else {
			event.OccurredAt = currentTimestamp()
			session, err := runAuth(request, handlers.kdfPool, func() (domain.LoginSession, error) { return handlers.service.Login(ctx, username, password, &event) })
			if err != nil {
				reject("authentication_failed", err)
				return
			}
			actor := int64(session.User.Id)
			event.ActorId = &actor
			payload = struct {
				User      userResponse `json:"user"`
				ExpiresAt float64      `json:"expires_at"`
			}{publicUser(session.User), session.ExpiresAt}
			writer.Header().Add("Set-Cookie", sessionCookieHeader(session.Token, session.ExpiresAt, currentTimestamp(), handlers.isCookieSecure))
		}
		handlers.limiter.clearUsername(kind, username)
	} else {
		if name == "change_password" && !cryptography.ValidNewPassword(body["new_password"].(string)) {
			reject("validation_failed", mapAuthError(domain.ErrPasswordShort))
			return
		}
		current, err := handlers.authenticator.requireUser(request)
		if err != nil {
			reject("authentication_required", err)
			return
		}
		actor := int64(current.authorization.User.Id)
		event.ActorId = &actor
		event.OccurredAt = currentTimestamp()
		if name == "create_token" {
			token, err := runAuth(request, handlers.pool, func() (domain.IssuedToken, error) {
				return handlers.service.CreateToken(ctx, current.authorization, body["name"].(string), body["ttl"].(int64), &event)
			})
			if err != nil {
				reject("operation_failed", err)
				return
			}
			event.TargetId = &token.Id
			payload = token
		} else {
			changed, err := runAuth(request, handlers.kdfPool, func() (bool, error) {
				return handlers.service.ChangePassword(ctx, current.authorization.User.Id, body["old_password"].(string), body["new_password"].(string), &event)
			})
			if err != nil {
				reject("operation_failed", err)
				return
			}
			if !changed {
				reject("authentication_failed", &apiError{status: 400, detail: "Old password is incorrect"})
				return
			}
			payload = map[string]bool{"ok": true}
		}
	}
	logAuth(request, event, "")
	writeResponse(writer, payload)
}
