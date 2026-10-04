package api

import (
	"context"
	"log/slog"
	"net/http"
	"net/netip"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/httpwire"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
)

type requestIdKey struct{}
type auditStartKey struct{}

func beginAuthAudit(request *http.Request) *http.Request {
	return request.WithContext(context.WithValue(request.Context(), auditStartKey{}, time.Now()))
}
func currentTimestamp() float64 { return float64(time.Now().UnixNano()) / 1e9 }

type userResponse struct {
	Id       int64  `json:"id"`
	Username string `json:"username"`
	IsAdmin  bool   `json:"is_admin"`
}

func publicUser(user domain.User) userResponse {
	return userResponse{int64(user.Id), user.Username, user.IsAdmin}
}

func logAuth(request *http.Request, event domain.AuditEvent, reason string) {
	actor, target := int64(0), int64(0)
	if event.ActorId != nil {
		actor = *event.ActorId
	}
	if event.TargetId != nil {
		target = *event.TargetId
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

func requestId(request *http.Request) string {
	value, _ := request.Context().Value(requestIdKey{}).(string)
	return value
}

type authHandlers struct {
	service        *auth.Service
	repository     *storage.Repository
	authenticator  *Authenticator
	pool           *executor.Pool
	kdfPool        *executor.Pool
	limiter        *authRateLimiter
	trustedProxies []netip.Prefix
	isCookieSecure bool
}

func (handlers *authHandlers) routes() []route {
	routes := []route{}
	for _, operation := range []openapi.Operation{
		{Method: "POST", Path: "/api/auth/register", Id: "register"},
		{Method: "POST", Path: "/api/auth/login", Id: "login"},
		{Method: "POST", Path: "/api/auth/change-password", Id: "change_password"},
		{Method: "POST", Path: "/api/auth/tokens", Id: "create_token"},
		{Method: "GET", Path: "/api/auth/me", Id: "get_me"},
		{Method: "GET", Path: "/api/auth/tokens", Id: "get_tokens"},
		{Method: "DELETE", Path: "/api/auth/tokens/{token_id}", Id: "delete_token"},
		{Method: "POST", Path: "/api/auth/logout", Id: "logout"},
		{Method: "POST", Path: "/api/auth/logout-all", Id: "logout_all"},
		{Method: "POST", Path: "/api/auth/invite-code", Id: "generate_invite_code"},
		{Method: "GET", Path: "/api/auth/invite-code", Id: "get_invite_code"},
		{Method: "DELETE", Path: "/api/auth/invite-code", Id: "revoke_invite_code"},
		{Method: "POST", Path: "/api/auth/invite-code/rotate", Id: "rotate_invite_code"},
		{Method: "GET", Path: "/api/auth/invite-required", Id: "check_invite_required"},
	} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) {
			if kind, exists := authRequestTypes[operation.Id]; exists {
				handlers.handleBody(writer, request, operation.Id, kind)
			} else {
				handlers.handle(writer, request, operation.Id)
			}
		}})
	}
	return routes
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
	if event.ActorId != nil && *event.ActorId <= 0 {
		event.ActorId = nil
	}
	if event.TargetId != nil && *event.TargetId <= 0 {
		event.TargetId = nil
	}
	type result struct{ err error }
	value, err := executor.Run(request.Context(), handlers.pool, func() (result, error) {
		_, err := handlers.repository.AppendAudit(context.Background(), event)
		return result{err}, nil
	})
	if err != nil {
		storage.ReportAuditFailure("executor_unavailable")
		return serviceUnavailable()
	}
	if value.err != nil {
		return serviceUnavailable()
	}
	return nil
}

func (handlers *authHandlers) handle(writer http.ResponseWriter, request *http.Request, name string) {
	ctx := context.Background()
	if name == "check_invite_required" {
		bootstrap, err := runAuth(request, handlers.pool, func() (bool, error) { return handlers.service.BootstrapRequired(ctx) })
		if err != nil {
			err.write(writer)
			return
		}
		writeResponse(writer, map[string]bool{"required": handlers.service.InviteRequired(), "bootstrap_required": bootstrap})
		return
	}
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
	var tokenId int64
	if name == "delete_token" {
		value, err := extractPathInteger(request, "token_id")
		if err != nil {
			err.write(writer)
			return
		}
		tokenId = value
	}
	request = beginAuthAudit(request)
	isLogout := name == "logout" || name == "logout_all"
	if _, hasCookie := sessionCookie(request.Header); isLogout && hasCookie {
		writer.Header().Add("Set-Cookie", sessionCookieHeader("", 0, 0, handlers.isCookieSecure))
	}
	event := domain.AuditEvent{Action: action, Outcome: "completed", RequestId: requestId(request), OccurredAt: float64(time.Now().UnixNano()) / 1e9}
	current, err := handlers.authenticator.requireUser(request)
	if err != nil {
		if action != "" {
			err = handlers.reject(request, event, "authentication_required", err)
		}
		err.write(writer)
		return
	}
	actor := int64(current.authorization.User.Id)
	event.ActorId = &actor
	if name == "logout_all" {
		event.TargetId = &actor
	}
	if name == "delete_token" {
		event.TargetId = &tokenId
	}
	event.OccurredAt = currentTimestamp()
	var payload any
	switch name {
	case "get_me":
		payload = publicUser(current.authorization.User)
	case "get_tokens":
		payload, err = runAuth(request, handlers.pool, func() ([]domain.TokenInfo, error) {
			return handlers.service.ListTokens(ctx, current.authorization.User.Id)
		})
	case "get_invite_code":
		payload, err = runAuth(request, handlers.pool, func() (*domain.InviteResponse, error) {
			return handlers.service.Invite(ctx, current.authorization.User.Id)
		})
	case "generate_invite_code", "rotate_invite_code":
		payload, err = runAuth(request, handlers.pool, func() (domain.InviteResponse, error) {
			return handlers.service.IssueInvite(ctx, current.authorization.User.Id, name == "rotate_invite_code", &event)
		})
		if err == nil {
			target := payload.(domain.InviteResponse).Id
			event.TargetId = &target
		}
	case "delete_token", "revoke_invite_code":
		var changed bool
		changed, err = runAuth(request, handlers.pool, func() (bool, error) {
			if name == "delete_token" {
				return handlers.service.RevokeTokenId(ctx, current.authorization.User.Id, tokenId, event)
			}
			return handlers.service.RevokeInvite(ctx, current.authorization.User.Id, &event)
		})
		if err == nil && !changed {
			detail := "Token not found"
			if name == "revoke_invite_code" {
				detail = "Unrevoked invite code not found"
			}
			handlers.reject(request, event, "not_found", &apiError{status: 404, detail: detail}).write(writer)
			return
		}
		payload = map[string]bool{"ok": true}
	case "logout", "logout_all":
		_, err = runAuth(request, handlers.pool, func() (bool, error) {
			return retryRevocation(func() (bool, error) {
				if name == "logout" {
					return handlers.service.RevokeToken(ctx, current.token, event)
				}
				_, err := handlers.service.RevokeAll(ctx, current.authorization.User.Id, event)
				return err == nil, err
			})
		})
		if err != nil {
			slog.Warn("security.auth.revocation_unconfirmed", "event", "security.auth.revocation_unconfirmed", "component", "security", "action", event.Action, "actor_id", actor, "request_id", event.RequestId)
			rejection := event
			rejection.Outcome, rejection.Reason = "rejected", "operation_failed"
			if handlers.persistAudit(request, rejection) != nil {
				slog.Error("security.audit.persistence_failed", "event", "security.audit.persistence_failed", "component", "security", "action", event.Action, "request_id", event.RequestId)
			}
			logAuth(request, event, "operation_failed")
			(&apiError{status: 503, structured: map[string]string{"code": "session_revocation_unconfirmed", "message": "Session revocation could not be confirmed", "request_id": event.RequestId}}).write(writer)
			return
		}
		payload = struct {
			Ok     bool  `json:"ok"`
			UserId int64 `json:"user_id"`
		}{true, actor}
	}
	if err != nil {
		if action != "" {
			err = handlers.reject(request, event, "operation_failed", err)
		}
		err.write(writer)
		return
	}
	if action != "" {
		logAuth(request, event, "")
	}
	writeResponse(writer, payload)
}

func retryRevocation[Value any](work func() (Value, error)) (Value, error) {
	value, err := work()
	if storage.IsTransientContention(err) {
		time.Sleep(25 * time.Millisecond)
		return work()
	}
	return value, err
}

func writeResponse(writer http.ResponseWriter, value any) {
	if err := httpwire.JSON(writer, 200, value); err != nil {
		internalError().write(writer)
	}
}
