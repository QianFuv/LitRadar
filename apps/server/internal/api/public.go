package api

import (
	"context"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/announcements"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	schedulerstorage "github.com/QianFuv/LitRadar/apps/server/internal/storage/scheduler"
	"time"
)

type publicHandlers struct {
	storage   config.Config
	pool      *executor.Pool
	scheduler *schedulerstorage.Repository
}
type healthResponse struct {
	// Status describes current lifecycle state.
	Status string `json:"status" nullable:"false" enum:"ok,unhealthy"`
}

func (handlers *publicHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/health/live", ID: "live"}, func(ctx context.Context, input *LiveInput) (*output[healthResponse], error) {
			return &output[healthResponse]{Body: healthResponse{Status: "ok"}}, nil
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/health/ready", ID: "ready"}, handlers.ready),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/announcements", ID: "get_announcements"}, func(ctx context.Context, input *GetAnnouncementsInput) (*output[[]announcements.Announcement], error) {
			return admitted(ctx, handlers.pool, func() ([]announcements.Announcement, error) {
				return announcements.ListActive(context.WithoutCancel(ctx), handlers.storage.AuthDBPath)
			}, func(error) *apiError { return internalError() })
		}),
	}
}
func (handlers *publicHandlers) ready(ctx context.Context, input *ReadyInput) (*output[healthResponse], error) {
	request := input.request
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
		return nil, failure.problem()
	}
	return &output[healthResponse]{Body: healthResponse{Status: "ok"}}, nil
}
