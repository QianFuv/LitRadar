package api

import (
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"log/slog"
	"math"
	"net/http"
	"time"
)

func (handlers *authHandlers) authRateLimit(request *http.Request, event *domain.AuditEvent, rejection *rateLimitRejection) *apiError {
	limited := *event
	limited.Outcome, limited.Reason = "rate_limited", rejection.reason
	limited.SourceClass, limited.Bucket = rejection.sourceClass, rejection.bucket
	limited.RejectedCount = int64(min(rejection.rejectedCount, math.MaxInt64))
	limited.RetryAfterSeconds = int64(min(rejection.retryAfter, math.MaxInt64))
	if rejection.shouldPersistAudit {
		if failure := handlers.persistAudit(request, limited); failure != nil {
			logAuth(request, *event, "operation_failed")
			return failure
		}
	}
	started, _ := request.Context().Value(auditStartKey{}).(time.Time)
	slog.WarnContext(request.Context(), "security.auth.rate_limited", "event", "security.auth.rate_limited", "component", "security", "action", event.Action, "outcome", "rate_limited", "actor_id", 0, "target_id", 0, "reason", rejection.reason, "bucket", rejection.bucket, "source_class", rejection.sourceClass, "rejected_count", rejection.rejectedCount, "request_id", event.RequestID, "retry_after_seconds", rejection.retryAfter, "duration_ms", time.Since(started).Milliseconds())
	return &apiError{status: 429, detail: "Too many authentication attempts; try again later", retryAfter: &rejection.retryAfter}
}

// registerAuthUser validates credentials before registering through the bounded KDF pool.
