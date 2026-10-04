package delivery

import (
	"fmt"
	"log/slog"
	"strings"
	"unicode/utf8"
)

// SecretUpdate distinguishes preservation, explicit clearing and replacement.
type SecretUpdate struct {
	IsPresent bool
	Value     *string
}

func (update SecretUpdate) String() string       { return "SecretUpdate([REDACTED])" }
func (update SecretUpdate) GoString() string     { return update.String() }
func (update SecretUpdate) LogValue() slog.Value { return slog.StringValue(update.String()) }

// DefaultNotificationSettingsUpdate supplies the original defaults for programmatic callers.
func DefaultNotificationSettingsUpdate() NotificationSettingsUpdate {
	return NotificationSettingsUpdate{Keywords: []string{}, Directions: []string{}, SelectedDatabases: []string{}, DeliveryMethod: "folder", PushplusTemplate: "markdown", PushplusChannel: "wechat", AiRetryAttempts: 3, Enabled: true}
}

// ValidateNotificationSettings preserves original collection, Unicode-length and required-field checks.
func ValidateNotificationSettings(value NotificationSettingsUpdate) error {
	collections := []struct {
		name, label   string
		values        []string
		count, length int
	}{{"keywords", "keyword", value.Keywords, 100, 500}, {"directions", "direction", value.Directions, 100, 500}, {"selected_databases", "selected database", value.SelectedDatabases, 500, 255}}
	for _, collection := range collections {
		if len(collection.values) > collection.count {
			return fmt.Errorf("%s must contain at most %d items", collection.name, collection.count)
		}
	}
	check := func(label, text string, maximum int) error {
		if utf8.RuneCountInString(text) > maximum {
			return fmt.Errorf("%s must be at most %d characters", label, maximum)
		}
		return nil
	}
	for _, collection := range collections {
		for _, text := range collection.values {
			if err := check(collection.label, text, collection.length); err != nil {
				return err
			}
		}
	}
	if count := utf8.RuneCountInString(strings.TrimSpace(value.DeliveryMethod)); count == 0 || count > 32 {
		return fmt.Errorf("delivery_method must be 1-32 characters")
	}
	fields := []struct {
		name, text string
		maximum    int
	}{{"delivery_method", value.DeliveryMethod, 32}, {"pushplus_template", value.PushplusTemplate, 64}, {"pushplus_topic", value.PushplusTopic, 200}, {"pushplus_channel", value.PushplusChannel, 64}, {"ai_base_url", value.AiBaseUrl, 2048}, {"ai_backup_base_url", value.AiBackupBaseUrl, 2048}, {"ai_model", value.AiModel, 200}, {"ai_backup_model", value.AiBackupModel, 200}, {"ai_system_prompt", value.AiSystemPrompt, 10000}, {"ai_backup_system_prompt", value.AiBackupSystemPrompt, 10000}}
	for _, field := range fields {
		if err := check(field.name, field.text, field.maximum); err != nil {
			return err
		}
	}
	for _, field := range []struct {
		name   string
		secret SecretUpdate
	}{{"pushplus_token", value.PushplusToken}, {"ai_api_key", value.AiApiKey}, {"ai_backup_api_key", value.AiBackupApiKey}} {
		if field.secret.IsPresent && field.secret.Value != nil {
			if err := check(field.name, *field.secret.Value, 4096); err != nil {
				return err
			}
		}
	}
	return nil
}

// NotificationSettingsUpdate preserves the notification settings contract.
type NotificationSettingsUpdate struct {
	Keywords             []string     `json:"keywords"`
	Directions           []string     `json:"directions"`
	SelectedDatabases    []string     `json:"selected_databases"`
	DeliveryMethod       string       `json:"delivery_method"`
	PushplusToken        SecretUpdate `json:"pushplus_token"`
	PushplusTemplate     string       `json:"pushplus_template"`
	PushplusTopic        string       `json:"pushplus_topic"`
	PushplusChannel      string       `json:"pushplus_channel"`
	SyncToTrackingFolder bool         `json:"sync_to_tracking_folder"`
	AiBaseUrl            string       `json:"ai_base_url"`
	AiApiKey             SecretUpdate `json:"ai_api_key"`
	AiModel              string       `json:"ai_model"`
	AiSystemPrompt       string       `json:"ai_system_prompt"`
	AiBackupBaseUrl      string       `json:"ai_backup_base_url"`
	AiBackupApiKey       SecretUpdate `json:"ai_backup_api_key"`
	AiBackupModel        string       `json:"ai_backup_model"`
	AiBackupSystemPrompt string       `json:"ai_backup_system_prompt"`
	AiRetryAttempts      int64        `json:"ai_retry_attempts"`
	Enabled              bool         `json:"enabled"`
}

func (value NotificationSettingsUpdate) String() string {
	return "NotificationSettingsUpdate([REDACTED])"
}
func (value NotificationSettingsUpdate) GoString() string { return value.String() }
func (value NotificationSettingsUpdate) LogValue() slog.Value {
	return slog.StringValue(value.String())
}

// NotificationSettings preserves the notification settings contract.
type NotificationSettings struct {
	Id                   int64    `json:"-"`
	UserId               int64    `json:"-"`
	Keywords             []string `json:"-"`
	Directions           []string `json:"-"`
	SelectedDatabases    []string `json:"-"`
	DeliveryMethod       string   `json:"-"`
	PushplusToken        string   `json:"-"`
	PushplusTemplate     string   `json:"-"`
	PushplusTopic        string   `json:"-"`
	PushplusChannel      string   `json:"-"`
	SyncToTrackingFolder bool     `json:"-"`
	AiBaseUrl            string   `json:"-"`
	AiApiKey             string   `json:"-"`
	AiModel              string   `json:"-"`
	AiSystemPrompt       string   `json:"-"`
	AiBackupBaseUrl      string   `json:"-"`
	AiBackupApiKey       string   `json:"-"`
	AiBackupModel        string   `json:"-"`
	AiBackupSystemPrompt string   `json:"-"`
	AiRetryAttempts      int64    `json:"-"`
	Enabled              bool     `json:"-"`
	CreatedAt            float64  `json:"-"`
	UpdatedAt            float64  `json:"-"`
}

func (value NotificationSettings) String() string       { return "NotificationSettings([REDACTED])" }
func (value NotificationSettings) GoString() string     { return value.String() }
func (value NotificationSettings) LogValue() slog.Value { return slog.StringValue(value.String()) }

// NotificationSettingsResponse preserves the notification settings contract.
type NotificationSettingsResponse struct {
	Id                   int64    `json:"id"`
	UserId               int64    `json:"user_id"`
	Keywords             []string `json:"keywords"`
	Directions           []string `json:"directions"`
	SelectedDatabases    []string `json:"selected_databases"`
	DeliveryMethod       string   `json:"delivery_method"`
	HasPushplusToken     bool     `json:"has_pushplus_token"`
	PushplusTokenMask    string   `json:"pushplus_token_mask"`
	PushplusTemplate     string   `json:"pushplus_template"`
	PushplusTopic        string   `json:"pushplus_topic"`
	PushplusChannel      string   `json:"pushplus_channel"`
	SyncToTrackingFolder bool     `json:"sync_to_tracking_folder"`
	AiBaseUrl            string   `json:"ai_base_url"`
	HasAiApiKey          bool     `json:"has_ai_api_key"`
	AiApiKeyMask         string   `json:"ai_api_key_mask"`
	AiModel              string   `json:"ai_model"`
	AiSystemPrompt       string   `json:"ai_system_prompt"`
	AiBackupBaseUrl      string   `json:"ai_backup_base_url"`
	HasAiBackupApiKey    bool     `json:"has_ai_backup_api_key"`
	AiBackupApiKeyMask   string   `json:"ai_backup_api_key_mask"`
	AiBackupModel        string   `json:"ai_backup_model"`
	AiBackupSystemPrompt string   `json:"ai_backup_system_prompt"`
	AiRetryAttempts      int64    `json:"ai_retry_attempts"`
	Enabled              bool     `json:"enabled"`
	CreatedAt            float64  `json:"created_at"`
	UpdatedAt            float64  `json:"updated_at"`
}

func (value NotificationSettingsResponse) String() string {
	return "NotificationSettingsResponse([REDACTED])"
}
func (value NotificationSettingsResponse) GoString() string { return value.String() }
func (value NotificationSettingsResponse) LogValue() slog.Value {
	return slog.StringValue(value.String())
}

// Public returns the fixed-mask HTTP projection without exposing decrypted credentials.
func (value NotificationSettings) Public() NotificationSettingsResponse {
	mask := func(secret string) string {
		if secret == "" {
			return ""
		}
		return "••••"
	}
	return NotificationSettingsResponse{
		Id:                   value.Id,
		UserId:               value.UserId,
		Keywords:             value.Keywords,
		Directions:           value.Directions,
		SelectedDatabases:    value.SelectedDatabases,
		DeliveryMethod:       value.DeliveryMethod,
		HasPushplusToken:     value.PushplusToken != "",
		PushplusTokenMask:    mask(value.PushplusToken),
		PushplusTemplate:     value.PushplusTemplate,
		PushplusTopic:        value.PushplusTopic,
		PushplusChannel:      value.PushplusChannel,
		SyncToTrackingFolder: value.SyncToTrackingFolder,
		AiBaseUrl:            value.AiBaseUrl,
		HasAiApiKey:          value.AiApiKey != "",
		AiApiKeyMask:         mask(value.AiApiKey),
		AiModel:              value.AiModel,
		AiSystemPrompt:       value.AiSystemPrompt,
		AiBackupBaseUrl:      value.AiBackupBaseUrl,
		HasAiBackupApiKey:    value.AiBackupApiKey != "",
		AiBackupApiKeyMask:   mask(value.AiBackupApiKey),
		AiBackupModel:        value.AiBackupModel,
		AiBackupSystemPrompt: value.AiBackupSystemPrompt,
		AiRetryAttempts:      value.AiRetryAttempts,
		Enabled:              value.Enabled,
		CreatedAt:            value.CreatedAt,
		UpdatedAt:            value.UpdatedAt,
	}
}
