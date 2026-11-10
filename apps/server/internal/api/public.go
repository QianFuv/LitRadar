package api

import (
	"context"
	"net/http"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/announcements"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	schedulerstorage "github.com/QianFuv/LitRadar/apps/server/internal/storage/scheduler"
)

type publicHandlers struct {
	storage   config.Config
	pool      *executor.Pool
	scheduler *schedulerstorage.Repository
}

func (handlers *publicHandlers) routes() []route {
	return []route{
		{openapi.Operation{Method: "GET", Path: "/health/live", Id: "live"}, func(writer http.ResponseWriter, request *http.Request) {
			writeResponse(writer, map[string]string{"status": "ok"})
		}},
		{openapi.Operation{Method: "GET", Path: "/health/ready", Id: "ready"}, handlers.ready},
		{openapi.Operation{Method: "GET", Path: "/api/announcements", Id: "get_announcements"}, handlers.announcements},
	}
}

// ready reports scheduler admission failures using the retryable problem contract.
func (handlers *publicHandlers) ready(writer http.ResponseWriter, request *http.Request) {
	now := currentTimestamp()
	isHealthy, err := executor.RunWithQueueTimeout(request.Context(), handlers.pool, time.Second, func() (bool, error) {
		status, err := handlers.scheduler.Status(context.WithoutCancel(request.Context()), now, scheduler.HealthWindowSeconds, 0)
		if err != nil {
			return false, err
		}
		for _, worker := range status.Workers {
			if worker.IsHealthy {
				return true, nil
			}
		}
		return false, nil
	})
	if err != nil || !isHealthy {
		failure := serviceUnavailable()
		failure.detail = "Scheduler is not ready"
		failure.write(writer)
		return
	}
	writeResponse(writer, map[string]string{"status": "ok"})
}

func (handlers *publicHandlers) announcements(writer http.ResponseWriter, request *http.Request) {
	type result struct {
		rows []announcements.Announcement
		err  error
	}
	value, err := executor.Run(request.Context(), handlers.pool, func() (result, error) {
		rows, err := announcements.ListActive(context.WithoutCancel(request.Context()), handlers.storage.AuthDbPath)
		return result{rows, err}, nil
	})
	if err != nil {
		mapExecutorError(err).write(writer)
		return
	}
	if value.err != nil {
		internalError().write(writer)
		return
	}
	writeResponse(writer, value.rows)
}
