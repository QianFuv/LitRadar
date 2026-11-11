package api

import (
	"context"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
)

type folderSummary struct {
	// ID describes invite code row identifier.
	ID int64 `json:"id"`
	// Name describes folder display name.
	Name string `json:"name"`
}
type trackingStatusResponse struct {
	// TrackingFolder contains the tracking folder value.
	TrackingFolder *folderSummary `json:"tracking_folder" required:"false" nullable:"false" allowNull:"true"`
	// TotalFolders describes total folders.
	TotalFolders int `json:"total_folders" nullable:"false" minimum:"0" format:"int64"`
	// WeeklyArticlesAvailable describes number of weekly article ids currently available.
	WeeklyArticlesAvailable int `json:"weekly_articles_available" nullable:"false" minimum:"0" format:"int64"`
	// NotificationConfigured reports whether notification settings exist for the user.
	NotificationConfigured bool `json:"notification_configured" nullable:"false"`
}

func (handlers *trackingHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/tracking/status", ID: "status"}, func(ctx context.Context, input *StatusInput) (*output[trackingStatusResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (trackingStatusResponse, error) {
				value, failure := handlers.trackingStatus(workContext, current.authorization.User)
				if failure != nil {
					return value, failure
				}
				return value, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/tracking/ai-endpoints", ID: "get_ai_endpoints"}, func(ctx context.Context, input *GetAIEndpointsInput) (*output[[]string], error) {
			_, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() ([]string, error) { return handlers.settings.AiBaseUrls(workContext) }, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/tracking/notification-settings", ID: "get_notification_settings"}, func(ctx context.Context, input *GetNotificationSettingsInput) (*output[nullableBody[domain.NotificationSettingsResponse]], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			value, err := admitted(ctx, handlers.pool, func() (*domain.NotificationSettingsResponse, error) {
				value, err := handlers.repository.GetNotificationSettings(workContext, handlers.codec, int64(current.authorization.User.Id))
				if err != nil {
					return nil, err
				}
				if value == nil {
					return nil, nil
				}
				result := value.Public()
				return &result, nil
			}, mapBusinessError)
			if err != nil {
				return nil, err
			}
			return &output[nullableBody[domain.NotificationSettingsResponse]]{Body: nullableBody[domain.NotificationSettingsResponse]{value: value.Body}}, nil
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/tracking/push-weekly/status", ID: "get_push_weekly_status"}, func(ctx context.Context, input *GetPushWeeklyStatusInput) (*output[manualPushStatus], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (manualPushStatus, error) {
				record, err := handlers.repository.LoadLatestManualRun(workContext, int64(current.authorization.User.Id))
				if err != nil {
					return manualPushStatus{}, err
				}
				return manualStatus(record), nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/tracking/push-weekly", ID: "push_weekly_to_tracking"}, func(ctx context.Context, input *PushWeeklyToTrackingInput) (*output[manualPushStatus], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (manualPushStatus, error) {
				value, failure := handlers.admitManualRun(workContext, "push_weekly_to_tracking", int64(current.authorization.User.Id), "", requestId(input.request))
				if failure != nil {
					return value, failure
				}
				return value, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/tracking/push-weekly/runs/{run_id}", ID: "get_push_weekly_run"}, func(ctx context.Context, input *GetPushWeeklyRunInput) (*output[manualPushStatus], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (manualPushStatus, error) {
				value, failure := handlers.performManualRun(workContext, "get_push_weekly_run", current.authorization.User, input.RunID)
				if failure != nil {
					return value, failure
				}
				return value, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/tracking/push-weekly/runs/{run_id}/cancel", ID: "cancel_push_weekly_run"}, func(ctx context.Context, input *CancelPushWeeklyRunInput) (*output[manualPushStatus], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (manualPushStatus, error) {
				value, failure := handlers.performManualRun(workContext, "cancel_push_weekly_run", current.authorization.User, input.RunID)
				if failure != nil {
					return value, failure
				}
				return value, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/tracking/push-weekly/runs/{run_id}/acknowledge", ID: "acknowledge_unknown_push_weekly_run"}, func(ctx context.Context, input *AcknowledgeUnknownPushWeeklyRunInput) (*output[manualPushStatus], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			workContext := context.WithoutCancel(ctx)
			return admitted(ctx, handlers.pool, func() (manualPushStatus, error) {
				value, failure := handlers.admitManualRun(workContext, "acknowledge_unknown_push_weekly_run", int64(current.authorization.User.Id), input.RunID, requestId(input.request))
				if failure != nil {
					return value, failure
				}
				return value, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/tracking/notification-settings", ID: "update_notification_settings"}, func(ctx context.Context, input *UpdateNotificationSettingsInput) (*output[domain.NotificationSettingsResponse], error) {
			current, failure := handlers.authenticator.requireUser(input.request)
			if failure != nil {
				return nil, failure.problem()
			}
			update := domain.NotificationSettingsUpdate{Keywords: input.Body.Keywords, Directions: input.Body.Directions, SelectedDatabases: input.Body.SelectedDatabases, DeliveryMethod: input.Body.DeliveryMethod, PushplusToken: input.Body.PushplusToken, PushplusTemplate: input.Body.PushplusTemplate, PushplusTopic: input.Body.PushplusTopic, PushplusChannel: input.Body.PushplusChannel, SyncToTrackingFolder: input.Body.SyncToTrackingFolder, AiBaseUrl: input.Body.AIBaseURL, AiApiKey: input.Body.AIAPIKey, AiModel: input.Body.AIModel, AiSystemPrompt: input.Body.AISystemPrompt, AiBackupBaseUrl: input.Body.AIBackupBaseURL, AiBackupApiKey: input.Body.AIBackupAPIKey, AiBackupModel: input.Body.AIBackupModel, AiBackupSystemPrompt: input.Body.AIBackupSystemPrompt, AiRetryAttempts: *input.Body.AIRetryAttempts, Enabled: *input.Body.Enabled}
			if err := domain.ValidateNotificationSettings(update); err != nil {
				return nil, badRequest(err.Error()).problem()
			}
			if update.AiRetryAttempts < 1 || update.AiRetryAttempts > 10 {
				return nil, badRequest("ai_retry_attempts must be between 1 and 10").problem()
			}
			value, failure := handlers.updateNotification(ctx, int64(current.authorization.User.Id), update)
			if failure != nil {
				return nil, failure.problem()
			}
			return &output[domain.NotificationSettingsResponse]{Body: value}, nil
		}),
	}
}
