package api

import (
	"context"
	"errors"
	"path/filepath"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
)

// updateNotification preserves database, delivery, endpoint and persistence validation order.
func (handlers *trackingHandlers) updateNotification(ctx context.Context, user int64, update domain.NotificationSettingsUpdate) (domain.NotificationSettingsResponse, *apiError) {
	available, failure := trackingStorage(ctx, handlers.pool, func() ([]string, error) {
		paths, err := handlers.storage.ListIndexDatabases()
		names := make([]string, len(paths))
		for index, path := range paths {
			names[index] = filepath.Base(path)
		}
		return names, err
	})
	if failure != nil {
		return domain.NotificationSettingsResponse{}, failure
	}

	if failure = normalizeTrackingDatabases(&update, available); failure != nil {
		return domain.NotificationSettingsResponse{}, failure
	}
	if update.DeliveryMethod != "folder" && update.DeliveryMethod != "pushplus" {
		return domain.NotificationSettingsResponse{}, badRequest("delivery_method must be one of: folder, pushplus")
	}
	if failure = handlers.validateTrackingDelivery(ctx, user, update); failure != nil {
		return domain.NotificationSettingsResponse{}, failure
	}
	allowed, failure := trackingStorage(ctx, handlers.pool, func() ([]string, error) { return handlers.settings.AiBaseUrls(context.WithoutCancel(ctx)) })
	if failure != nil {
		return domain.NotificationSettingsResponse{}, failure
	}
	if failure = normalizeTrackingEndpoints(&update, allowed); failure != nil {
		return domain.NotificationSettingsResponse{}, failure
	}
	trimNotificationUpdate(&update)
	return handlers.persistNotificationUpdate(ctx, user, update)
}

// normalizeTrackingDatabases deduplicates normalized selection before rejecting unknown entries.
func normalizeTrackingDatabases(update *domain.NotificationSettingsUpdate, available []string) *apiError {
	selected, invalid := []string{}, []string{}
	for _, raw := range update.SelectedDatabases {
		name := config.NormalizeDatabaseName(raw)
		if name == "" || slices.Contains(selected, name) {
			continue
		}
		selected = append(selected, name)
		if !slices.Contains(available, name) {
			invalid = append(invalid, name)
		}
	}
	if len(invalid) > 0 {
		return badRequest("Unknown databases: " + strings.Join(invalid, ", "))
	}
	if len(selected) > 0 && len(selected) == len(available) {
		selected = []string{}
	}
	update.SelectedDatabases = selected

	return nil
}

// effectivePushplusToken retains a blank replacement and clears an explicit null.
func effectivePushplusToken(existing *domain.NotificationSettings, secret domain.SecretUpdate) bool {
	hasToken := existing != nil && existing.PushplusToken != ""
	if secret.IsPresent {
		if secret.Value == nil {
			hasToken = false
		} else if strings.TrimSpace(*secret.Value) != "" {
			hasToken = true
		}
	}

	return hasToken
}

// validateTrackingDelivery loads existing secrets before checking token and folder prerequisites.
func (handlers *trackingHandlers) validateTrackingDelivery(ctx context.Context, user int64, update domain.NotificationSettingsUpdate) *apiError {
	existing, failure := trackingStorage(ctx, handlers.pool, func() (*domain.NotificationSettings, error) {
		return handlers.repository.GetNotificationSettings(context.WithoutCancel(ctx), handlers.codec, user)
	})
	if failure != nil {
		return failure
	}
	hasToken := effectivePushplusToken(existing, update.PushplusToken)
	hasFolder := true
	if update.DeliveryMethod == "folder" || update.SyncToTrackingFolder {
		var failure *apiError
		hasFolder, failure = trackingStorage(ctx, handlers.pool, func() (bool, error) {
			folder, err := handlers.favorites.TrackingFolder(context.WithoutCancel(ctx), identity.Id(user))
			return folder != nil, err
		})
		if failure != nil {
			return failure
		}
	}
	return validateTrackingPrerequisites(update, hasToken, hasFolder)
}

// validateTrackingPrerequisites checks folder, token and optional synchronization in order.
func validateTrackingPrerequisites(update domain.NotificationSettingsUpdate, hasToken, hasFolder bool) *apiError {
	if update.DeliveryMethod == "folder" && !hasFolder {
		return badRequest("A tracking folder is required when delivery_method is 'folder'")
	}
	if update.DeliveryMethod == "pushplus" && !hasToken {
		return badRequest("pushplus_token is required when delivery_method is 'pushplus'")
	}
	if update.DeliveryMethod == "pushplus" && update.SyncToTrackingFolder && !hasFolder {
		return badRequest("A tracking folder is required before enabling PushPlus sync to tracking")
	}

	return nil
}

// normalizeTrackingEndpoints validates primary then backup against the current allowlist.
func normalizeTrackingEndpoints(update *domain.NotificationSettingsUpdate, allowed []string) *apiError {
	for _, value := range []*string{&update.AiBaseUrl, &update.AiBackupBaseUrl} {
		if strings.TrimSpace(*value) == "" {
			*value = ""
			continue
		}
		normalized, err := settings.CanonicalizeBaseUrl(*value)
		if err != nil || !slices.Contains(allowed, normalized) {
			return badRequest("AI endpoint is not available")
		}
		*value = normalized
	}

	return nil
}

// trimNotificationUpdate preserves repeated entries, default template and secret presence intent.
func trimNotificationUpdate(update *domain.NotificationSettingsUpdate) {
	trimEntries := func(values []string) []string {
		result := []string{}
		for _, value := range values {
			if value = strings.TrimSpace(value); value != "" {
				result = append(result, value)
			}
		}
		return result
	}
	update.Keywords, update.Directions = trimEntries(update.Keywords), trimEntries(update.Directions)
	for _, value := range []*string{&update.PushplusTemplate, &update.PushplusTopic, &update.PushplusChannel, &update.AiModel, &update.AiSystemPrompt, &update.AiBackupModel, &update.AiBackupSystemPrompt} {
		*value = strings.TrimSpace(*value)
	}
	if update.PushplusTemplate == "" {
		update.PushplusTemplate = "markdown"
	}
	for _, secret := range []*domain.SecretUpdate{&update.PushplusToken, &update.AiApiKey, &update.AiBackupApiKey} {
		if secret.Value != nil {
			value := strings.TrimSpace(*secret.Value)
			secret.Value = &value
		}
	}
}

// persistNotificationUpdate revalidates within admitted work and maps repository rejections.
func (handlers *trackingHandlers) persistNotificationUpdate(ctx context.Context, user int64, update domain.NotificationSettingsUpdate) (domain.NotificationSettingsResponse, *apiError) {
	type result struct {
		value     *domain.NotificationSettings
		err       error
		isInvalid bool
	}
	observed, failure := trackingStorage(ctx, handlers.pool, func() (result, error) {
		if err := domain.ValidateNotificationSettings(update); err != nil {
			return result{err: err, isInvalid: true}, nil
		}
		value, err := handlers.repository.UpsertNotificationSettings(context.WithoutCancel(ctx), handlers.codec, user, update)
		return result{value: value, err: err}, nil
	})
	if failure != nil {
		return domain.NotificationSettingsResponse{}, failure
	}
	if observed.err != nil {
		if observed.isInvalid {
			return domain.NotificationSettingsResponse{}, badRequest(observed.err.Error())
		}
		if errors.Is(observed.err, storage.ErrEndpointNotAllowed) {
			return domain.NotificationSettingsResponse{}, badRequest("AI endpoint is not available")
		}
		if slices.Contains([]string{"A tracking folder is required when delivery_method is 'folder'", "pushplus_token is required when delivery_method is 'pushplus'", "A tracking folder is required before enabling PushPlus sync to tracking"}, observed.err.Error()) {
			return domain.NotificationSettingsResponse{}, badRequest(observed.err.Error())
		}
		return domain.NotificationSettingsResponse{}, internalError()
	}
	return observed.value.Public(), nil
}
