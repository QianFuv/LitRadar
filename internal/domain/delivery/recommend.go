// Package delivery defines recommendation and durable delivery contracts.
package delivery

import "log/slog"

// Subscriber contains the notification preferences and decrypted integration credentials.
type Subscriber struct {
	SubscriberId         string   `json:"subscriber_id"`
	UserId               int64    `json:"user_id"`
	Name                 string   `json:"name"`
	PushplusToken        string   `json:"pushplus_token"`
	Channel              *string  `json:"channel"`
	Keywords             []string `json:"keywords"`
	Directions           []string `json:"directions"`
	SelectedDatabases    []string `json:"selected_databases"`
	Topic                *string  `json:"topic"`
	Template             *string  `json:"template"`
	DeliveryMethod       string   `json:"delivery_method"`
	TrackingFolderId     *int64   `json:"tracking_folder_id"`
	SyncToTrackingFolder bool     `json:"sync_to_tracking_folder"`
	AiBaseUrl            *string  `json:"ai_base_url"`
	AiApiKey             *string  `json:"ai_api_key"`
	AiModel              *string  `json:"ai_model"`
	AiSystemPrompt       *string  `json:"ai_system_prompt"`
	AiBackupBaseUrl      *string  `json:"ai_backup_base_url"`
	AiBackupApiKey       *string  `json:"ai_backup_api_key"`
	AiBackupModel        *string  `json:"ai_backup_model"`
	AiBackupSystemPrompt *string  `json:"ai_backup_system_prompt"`
	AiRetryAttempts      int64    `json:"ai_retry_attempts"`
}

// String prevents formatted logs from exposing subscriber content or credentials.
func (subscriber Subscriber) String() string { return "Subscriber([REDACTED])" }

// GoString redacts Go-syntax diagnostics.
func (subscriber Subscriber) GoString() string { return subscriber.String() }

// LogValue redacts structured diagnostics.
func (subscriber Subscriber) LogValue() slog.Value { return slog.StringValue(subscriber.String()) }

// RankedSelection preserves the model score for one article.
type RankedSelection struct {
	ArticleId int64   `json:"article_id"`
	Score     float64 `json:"score"`
}

// SelectionResult contains a model summary and its ordered article choices.
type SelectionResult struct {
	Summary    string            `json:"summary"`
	Selections []RankedSelection `json:"selections"`
}

// String redacts model-generated prose.
func (result SelectionResult) String() string { return "SelectionResult([REDACTED])" }

// GoString redacts Go-syntax diagnostics.
func (result SelectionResult) GoString() string { return result.String() }

// LogValue redacts structured diagnostics.
func (result SelectionResult) LogValue() slog.Value { return slog.StringValue(result.String()) }
