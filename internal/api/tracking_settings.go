package api

import (
	"context"
	"errors"
	"path/filepath"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

type omittedSecret struct{}

func notificationBody() bodyType {
	stringsBody := listBody(stringBody)
	secret := optionalBody(stringBody)
	return structBody("NotificationSettingsUpdate",
		defaultBodyField("keywords", stringsBody, []any{}), defaultBodyField("directions", stringsBody, []any{}), defaultBodyField("selected_databases", stringsBody, []any{}),
		defaultBodyField("delivery_method", stringBody, "folder"), defaultBodyField("pushplus_token", secret, omittedSecret{}), defaultBodyField("pushplus_template", stringBody, "markdown"), defaultBodyField("pushplus_topic", stringBody, ""), defaultBodyField("pushplus_channel", stringBody, "wechat"), defaultBodyField("sync_to_tracking_folder", boolBody, false),
		defaultBodyField("ai_base_url", stringBody, ""), defaultBodyField("ai_api_key", secret, omittedSecret{}), defaultBodyField("ai_model", stringBody, ""), defaultBodyField("ai_system_prompt", stringBody, ""), defaultBodyField("ai_backup_base_url", stringBody, ""), defaultBodyField("ai_backup_api_key", secret, omittedSecret{}), defaultBodyField("ai_backup_model", stringBody, ""), defaultBodyField("ai_backup_system_prompt", stringBody, ""), defaultBodyField("ai_retry_attempts", integerBody, int64(3)), defaultBodyField("enabled", boolBody, true))
}
func notificationUpdate(body map[string]any) domain.NotificationSettingsUpdate {
	stringsOf := func(name string) []string {
		items := body[name].([]any)
		result := make([]string, len(items))
		for index, item := range items {
			result[index] = item.(string)
		}
		return result
	}
	secretOf := func(name string) domain.SecretUpdate {
		value := body[name]
		if _, omitted := value.(omittedSecret); omitted {
			return domain.SecretUpdate{}
		}
		if value == nil {
			return domain.SecretUpdate{IsPresent: true}
		}
		text := value.(string)
		return domain.SecretUpdate{IsPresent: true, Value: &text}
	}
	return domain.NotificationSettingsUpdate{Keywords: stringsOf("keywords"), Directions: stringsOf("directions"), SelectedDatabases: stringsOf("selected_databases"), DeliveryMethod: body["delivery_method"].(string), PushplusToken: secretOf("pushplus_token"), PushplusTemplate: body["pushplus_template"].(string), PushplusTopic: body["pushplus_topic"].(string), PushplusChannel: body["pushplus_channel"].(string), SyncToTrackingFolder: body["sync_to_tracking_folder"].(bool), AiBaseUrl: body["ai_base_url"].(string), AiApiKey: secretOf("ai_api_key"), AiModel: body["ai_model"].(string), AiSystemPrompt: body["ai_system_prompt"].(string), AiBackupBaseUrl: body["ai_backup_base_url"].(string), AiBackupApiKey: secretOf("ai_backup_api_key"), AiBackupModel: body["ai_backup_model"].(string), AiBackupSystemPrompt: body["ai_backup_system_prompt"].(string), AiRetryAttempts: body["ai_retry_attempts"].(int64), Enabled: body["enabled"].(bool)}
}

func (handlers *trackingHandlers) updateNotification(ctx context.Context, user int64, update domain.NotificationSettingsUpdate) (any, *apiError) {
	available, failure := trackingStorage(ctx, handlers.pool, func() ([]string, error) {
		paths, err := handlers.storage.ListIndexDatabases()
		names := make([]string, len(paths))
		for index, path := range paths {
			names[index] = filepath.Base(path)
		}
		return names, err
	})
	if failure != nil {
		return nil, failure
	}
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
		return nil, badRequest("Unknown databases: " + strings.Join(invalid, ", "))
	}
	if len(selected) > 0 && len(selected) == len(available) {
		selected = []string{}
	}
	update.SelectedDatabases = selected
	if update.DeliveryMethod != "folder" && update.DeliveryMethod != "pushplus" {
		return nil, badRequest("delivery_method must be one of: folder, pushplus")
	}
	existing, failure := trackingStorage(ctx, handlers.pool, func() (*domain.NotificationSettings, error) {
		return handlers.repository.GetNotificationSettings(context.Background(), handlers.codec, user)
	})
	if failure != nil {
		return nil, failure
	}
	hasToken := existing != nil && existing.PushplusToken != ""
	if update.PushplusToken.IsPresent {
		if update.PushplusToken.Value == nil {
			hasToken = false
		} else if strings.TrimSpace(*update.PushplusToken.Value) != "" {
			hasToken = true
		}
	}
	hasFolder := true
	if update.DeliveryMethod == "folder" || update.SyncToTrackingFolder {
		var failure *apiError
		hasFolder, failure = trackingStorage(ctx, handlers.pool, func() (bool, error) {
			folder, err := handlers.favorites.TrackingFolder(context.Background(), identity.Id(user))
			return folder != nil, err
		})
		if failure != nil {
			return nil, failure
		}
	}
	if update.DeliveryMethod == "folder" && !hasFolder {
		return nil, badRequest("A tracking folder is required when delivery_method is 'folder'")
	}
	if update.DeliveryMethod == "pushplus" && !hasToken {
		return nil, badRequest("pushplus_token is required when delivery_method is 'pushplus'")
	}
	if update.DeliveryMethod == "pushplus" && update.SyncToTrackingFolder && !hasFolder {
		return nil, badRequest("A tracking folder is required before enabling PushPlus sync to tracking")
	}
	allowed, failure := trackingStorage(ctx, handlers.pool, func() ([]string, error) { return handlers.settings.AiBaseUrls(context.Background()) })
	if failure != nil {
		return nil, failure
	}
	for _, value := range []*string{&update.AiBaseUrl, &update.AiBackupBaseUrl} {
		if strings.TrimSpace(*value) == "" {
			*value = ""
			continue
		}
		normalized, err := settings.CanonicalizeBaseUrl(*value)
		if err != nil || !slices.Contains(allowed, normalized) {
			return nil, badRequest("AI endpoint is not available")
		}
		*value = normalized
	}
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
	type result struct {
		value     *domain.NotificationSettings
		err       error
		isInvalid bool
	}
	observed, failure := trackingStorage(ctx, handlers.pool, func() (result, error) {
		if err := domain.ValidateNotificationSettings(update); err != nil {
			return result{err: err, isInvalid: true}, nil
		}
		value, err := handlers.repository.UpsertNotificationSettings(context.Background(), handlers.codec, user, update)
		return result{value: value, err: err}, nil
	})
	if failure != nil {
		return nil, failure
	}
	if observed.err != nil {
		if observed.isInvalid {
			return nil, badRequest(observed.err.Error())
		}
		if errors.Is(observed.err, storage.ErrEndpointNotAllowed) {
			return nil, badRequest("AI endpoint is not available")
		}
		if slices.Contains([]string{"A tracking folder is required when delivery_method is 'folder'", "pushplus_token is required when delivery_method is 'pushplus'", "A tracking folder is required before enabling PushPlus sync to tracking"}, observed.err.Error()) {
			return nil, badRequest(observed.err.Error())
		}
		return nil, internalError()
	}
	return observed.value.Public(), nil
}
