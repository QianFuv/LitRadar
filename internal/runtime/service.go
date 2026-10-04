package runtime

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/runtime/observability"
	"github.com/QianFuv/LitRadar/internal/storage/backup"
)

type component struct {
	name string
	run  func(context.Context) error
}

type componentResult struct {
	name string
	err  error
}

// Serve reports the complete preparation and coordinated service lifetime.
func Serve(ctx context.Context, configuration Config) (result error) {
	started := time.Now()
	slog.InfoContext(ctx, "service.starting", "event", "service.starting", "component", "runtime")
	defer func() {
		if result != nil {
			slog.ErrorContext(ctx, "service.failed", "event", "service.failed", "component", "runtime", "outcome", "failure", "error_kind", "service_failure", "duration_ms", time.Since(started).Milliseconds())
		} else {
			slog.InfoContext(ctx, "service.stopped", "event", "service.stopped", "component", "runtime", "outcome", "success", "duration_ms", time.Since(started).Milliseconds())
		}
	}()
	prepared, err := Prepare(ctx, configuration)
	if err != nil {
		return err
	}
	return prepared.Run(ctx)
}

// Run serves the prepared listener and joins all four components before releasing resources.
func (prepared *Prepared) Run(ctx context.Context) (result error) {
	defer func() { result = errors.Join(result, prepared.Close()) }()
	if prepared.listener == nil {
		return errors.New("service listener is not prepared")
	}
	slog.InfoContext(ctx, "service.ready", "event", "service.ready", "component", "runtime", "component_count", 4)
	return coordinate(ctx, []component{{"api", prepared.runHttp}, {"scheduler", prepared.runScheduler}, {"audit_retention", prepared.runAuditRetention}, {"delivery_dispatcher", prepared.runManualDispatcher}})
}

func coordinate(ctx context.Context, components []component) error {
	serviceContext, cancel := context.WithCancel(context.WithoutCancel(ctx))
	defer cancel()
	results := make(chan componentResult, len(components))
	for _, current := range components {
		go func() {
			var result error
			defer func() {
				if recover() != nil {
					observability.ReportPanic(serviceContext)
					result = errors.New("component task failed")
				}
				results <- componentResult{current.name, result}
			}()
			result = current.run(serviceContext)
		}()
	}
	remaining := len(components)
	var failure error
	select {
	case <-ctx.Done():
		if signal, exists := context.Cause(ctx).(interface{ SignalName() string }); exists {
			slog.InfoContext(ctx, "service.signal.received", "event", "service.signal.received", "component", "runtime", "signal", signal.SignalName())
		}
		slog.InfoContext(ctx, "service.shutdown.requested", "event", "service.shutdown.requested", "component", "runtime", "reason", "signal")
	case first := <-results:
		remaining--
		outcome, kind := "failure", "component_failure"
		if first.err == nil {
			outcome, kind = "unexpected_stop", "unexpected_stop"
		}
		slog.ErrorContext(ctx, "service.component.failed", "event", "service.component.failed", "component", first.name, "outcome", outcome, "error_kind", kind)
		if first.err == nil && ctx.Err() == nil {
			first.err = errors.New("component stopped unexpectedly")
		}
		if first.err != nil && !isOnlyCancellation(first.err) {
			failure = fmt.Errorf("%s: %w", first.name, first.err)
		}
	}
	cancel()
	for remaining > 0 {
		finished := <-results
		remaining--
		if finished.err != nil && !isOnlyCancellation(finished.err) {
			failure = errors.Join(failure, fmt.Errorf("%s: %w", finished.name, finished.err))
		}
	}
	if failure == nil {
		slog.InfoContext(ctx, "service.shutdown.completed", "event", "service.shutdown.completed", "component", "runtime", "outcome", "success")
	}
	return failure
}

func isOnlyCancellation(err error) bool {
	if err == context.Canceled {
		return true
	}
	if joined, exists := err.(interface{ Unwrap() []error }); exists {
		for _, cause := range joined.Unwrap() {
			if !isOnlyCancellation(cause) {
				return false
			}
		}
		return true
	}
	if wrapped := errors.Unwrap(err); wrapped != nil {
		return isOnlyCancellation(wrapped)
	}
	return false
}

func (prepared *Prepared) runHttp(ctx context.Context) error {
	instance := fmt.Sprintf("api-%d-%d", os.Getpid(), time.Now().UnixNano())
	filename := prepared.configuration.Storage.AuthDbPath
	if err := backup.RecordHeartbeat(ctx, filename, backup.Api, instance, unixTime()); err != nil {
		return err
	}
	heartbeatContext, cancelHeartbeat := context.WithCancel(observability.WithoutSpans(ctx))
	defer cancelHeartbeat()
	heartbeatDone := make(chan error, 1)
	go func() {
		heartbeatDone <- guardedTask(heartbeatContext, "API heartbeat task failed", func() error { return prepared.runHeartbeat(heartbeatContext, instance, 10*time.Second) })
	}()
	requestContext, cancelRequests := context.WithCancel(ctx)
	defer cancelRequests()
	server := &http.Server{Handler: redactHttpPanics(prepared.handler), BaseContext: func(net.Listener) context.Context { return observability.WithoutSpans(requestContext) }}
	serverDone := make(chan error, 1)
	go func() {
		serverDone <- guardedTask(ctx, "API server task failed", func() error { return server.Serve(prepared.listener) })
	}()
	var result error
	isServerDone, isHeartbeatDone := false, false
	select {
	case <-ctx.Done():
	case result = <-serverDone:
		isServerDone = true
	case result = <-heartbeatDone:
		isHeartbeatDone = true
		if result == nil && ctx.Err() == nil {
			result = errors.New("API heartbeat stopped unexpectedly")
		}
	}
	cancelHeartbeat()
	cancelRequests()
	result = errors.Join(result, prepared.handler.Close())
	result = errors.Join(result, server.Shutdown(context.Background()))
	if !isServerDone {
		serverError := <-serverDone
		if !errors.Is(serverError, http.ErrServerClosed) {
			result = errors.Join(result, serverError)
		}
	}
	if !isHeartbeatDone {
		result = errors.Join(result, <-heartbeatDone)
	}
	result = errors.Join(result, backup.DeleteHeartbeat(context.Background(), filename, backup.Api, instance))
	return result
}

func guardedTask(ctx context.Context, message string, operation func() error) (result error) {
	defer func() {
		if recover() != nil {
			observability.ReportPanic(ctx)
			result = errors.New(message)
		}
	}()
	return operation()
}

func redactHttpPanics(next http.Handler) http.Handler {
	return http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		defer func() {
			if cause := recover(); cause != nil {
				if cause != http.ErrAbortHandler {
					observability.ReportPanic(request.Context())
				}
				panic(http.ErrAbortHandler)
			}
		}()
		next.ServeHTTP(writer, request)
	})
}

func (prepared *Prepared) runHeartbeat(ctx context.Context, instance string, interval time.Duration) error {
	for waitDuration(ctx, interval) {
		_, err := executor.Run(context.Background(), prepared.services.StoragePool, func() (struct{}, error) {
			return struct{}{}, backup.RecordHeartbeat(context.Background(), prepared.configuration.Storage.AuthDbPath, backup.Api, instance, unixTime())
		})
		if err != nil {
			return errors.New("API heartbeat persistence failed")
		}
	}
	return nil
}

func waitDuration(ctx context.Context, duration time.Duration) bool {
	timer := time.NewTimer(duration)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-timer.C:
		return true
	}
}

func unixTime() float64 { return float64(time.Now().UnixNano()) / 1e9 }
