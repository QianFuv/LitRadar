package api

import (
	"context"
	"log/slog"
	"net/http"
	"net/netip"

	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"

	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
)

type requestIDKey struct{}
type auditStartKey struct{}

func beginAuthAudit(request *http.Request) *http.Request {
	return request.WithContext(context.WithValue(request.Context(), auditStartKey{}, time.Now()))
}
func currentTimestamp() float64 { return float64(time.Now().UnixNano()) / 1e9 }

type userResponse struct {
	Email        *string `json:"email" nullable:"true"`
	AuthKind     string  `json:"auth_kind" nullable:"false" enum:"local,oidc"`
	IsOidcLinked bool    `json:"is_oidc_linked" nullable:"false"`
	ID           int64   `json:"id" nullable:"false" format:"int64"`
	Username     string  `json:"username" nullable:"false"`
	IsAdmin      bool    `json:"is_admin" nullable:"false"`
}

func publicUser(user domain.User) userResponse {
	return userResponse{ID: int64(user.ID), Username: user.Username, IsAdmin: user.IsAdmin, Email: user.Email, AuthKind: user.AuthKind, IsOidcLinked: user.IsOidcLinked}
}

func logAuth(request *http.Request, event domain.AuditEvent, reason string) {
	actor, target := int64(0), int64(0)
	if event.ActorID != nil {
		actor = *event.ActorID
	}
	if event.TargetID != nil {
		target = *event.TargetID
	}
	started, _ := request.Context().Value(auditStartKey{}).(time.Time)
	duration := int64(0)
	if !started.IsZero() {
		duration = time.Since(started).Milliseconds()
	}
	name, level, outcome := "security.auth.completed", slog.LevelInfo, "completed"
	if reason != "" {
		name, level, outcome = "security.auth.rejected", slog.LevelWarn, "rejected"
	}
	slog.Log(request.Context(), level, name, "event", name, "component", "security", "action", event.Action, "outcome", outcome, "actor_id", actor, "target_id", target, "duration_ms", duration, "reason", reason)
}

func requestID(request *http.Request) string {
	value, _ := request.Context().Value(requestIDKey{}).(string)
	return value
}

type authHandlers struct {
	service        *auth.Service
	oidc           *auth.Oidc
	oidcPool       *executor.Pool
	repository     *storage.Repository
	authenticator  *Authenticator
	pool           *executor.Pool
	kdfPool        *executor.Pool
	limiter        *authRateLimiter
	trustedProxies []netip.Prefix
	isCookieSecure bool
}

func runAuth[Value any](request *http.Request, pool *executor.Pool, work func() (Value, error)) (Value, *apiError) {
	type outcome struct {
		value Value
		err   error
	}
	result, err := executor.Run(request.Context(), pool, func() (outcome, error) { value, err := work(); return outcome{value, err}, nil })
	if err != nil {
		var zero Value
		return zero, mapExecutorError(err)
	}
	if result.err != nil {
		var zero Value
		return zero, mapAuthError(result.err)
	}
	return result.value, nil
}

func (handlers *authHandlers) reject(request *http.Request, event domain.AuditEvent, reason string, original *apiError) *apiError {
	event.Outcome, event.Reason = "rejected", reason
	if failure := handlers.persistAudit(request, event); failure != nil {
		if event.Action != "logout" && event.Action != "logout_all" {
			reason = "operation_failed"
		}
		logAuth(request, event, reason)
		return failure
	}
	logAuth(request, event, reason)
	return original
}

func (handlers *authHandlers) persistAudit(request *http.Request, event domain.AuditEvent) *apiError {
	event.OccurredAt = currentTimestamp()
	if event.ActorID != nil && *event.ActorID <= 0 {
		event.ActorID = nil
	}
	if event.TargetID != nil && *event.TargetID <= 0 {
		event.TargetID = nil
	}
	type result struct{ err error }
	value, err := executor.Run(request.Context(), handlers.pool, func() (result, error) {
		_, err := handlers.repository.AppendAudit(context.WithoutCancel(request.Context()), event)
		return result{err}, nil
	})
	if err != nil {
		storage.ReportAuditFailureContext(request.Context(), "executor_unavailable")
		return serviceUnavailable()
	}
	if value.err != nil {
		return serviceUnavailable()
	}
	return nil
}

// authOperationAction identifies only audited mutations.
func authOperationAction(name string) string {
	action := ""
	switch name {
	case "delete_token":
		action = "token_revoke"
	case "logout", "logout_all":
		action = name
	case "generate_invite_code":
		action = "invite_create"
	case "rotate_invite_code":
		action = "invite_rotate"
	case "revoke_invite_code":
		action = "invite_revoke"
	}

	return action
}

// applyAuthAuditIdentity records the actor and operation-specific target before execution.
func applyAuthAuditIdentity(event *domain.AuditEvent, current currentIdentity, name string, tokenID int64) {
	actor := int64(current.authorization.User.ID)
	event.ActorID = &actor
	if name == "logout_all" {
		event.TargetID = &actor
	}
	if name == "delete_token" {
		event.TargetID = &tokenID
	}
	event.OccurredAt = currentTimestamp()
}

// authOperationFailure audits mutation rejection while preserving read-operation failures.
func (handlers *authHandlers) authOperationFailure(request *http.Request, event domain.AuditEvent, reason string, failure *apiError) *apiError {
	if event.Action != "" {
		return handlers.reject(request, event, reason, failure)
	}
	return failure
}

func retryRevocation[Value any](work func() (Value, error)) (Value, error) {
	value, err := work()
	if storage.IsTransientContention(err) {
		time.Sleep(25 * time.Millisecond)
		return work()
	}
	return value, err
}
