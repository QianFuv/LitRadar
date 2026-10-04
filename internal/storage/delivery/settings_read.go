package delivery

import (
	"context"
	"database/sql"
	"errors"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

const notificationColumns = `id,user_id,keywords,directions,selected_databases,delivery_method,pushplus_token,pushplus_template,pushplus_topic,pushplus_channel,sync_to_tracking_folder,ai_base_url,ai_api_key,ai_model,ai_system_prompt,ai_backup_base_url,ai_backup_api_key,ai_backup_model,ai_backup_system_prompt,ai_retry_attempts,enabled,created_at,updated_at`

// GetNotificationSettings reads raw preferences and decrypted credentials, including disabled settings.
func (repository *Repository) GetNotificationSettings(ctx context.Context, codec *secrets.Codec, userId int64) (*domain.NotificationSettings, error) {
	return scanNotificationSettings(repository.database.QueryRowContext(ctx, "SELECT "+notificationColumns+" FROM notification_settings WHERE user_id=?", userId), codec)
}
func scanNotificationSettings(scanner rowScanner, codec *secrets.Codec) (*domain.NotificationSettings, error) {
	row := readNotificationRow(scanner, 23)
	if errors.Is(row.err, sql.ErrNoRows) {
		return nil, nil
	}
	userId := row.integer(1)
	result := domain.NotificationSettings{
		Id:                   row.integer(0),
		UserId:               userId,
		Keywords:             row.strings(2),
		Directions:           row.strings(3),
		SelectedDatabases:    row.strings(4),
		DeliveryMethod:       row.text(5),
		PushplusToken:        row.secret(6, codec, userId, "pushplus_token"),
		PushplusTemplate:     row.text(7),
		PushplusTopic:        row.text(8),
		PushplusChannel:      row.text(9),
		SyncToTrackingFolder: row.integer(10) != 0,
		AiBaseUrl:            row.text(11),
		AiApiKey:             row.secret(12, codec, userId, "ai_api_key"),
		AiModel:              row.text(13),
		AiSystemPrompt:       row.text(14),
		AiBackupBaseUrl:      row.text(15),
		AiBackupApiKey:       row.secret(16, codec, userId, "ai_backup_api_key"),
		AiBackupModel:        row.text(17),
		AiBackupSystemPrompt: row.text(18),
		AiRetryAttempts:      min(max(row.integer(19), 1), 10),
		Enabled:              row.integer(20) != 0,
		CreatedAt:            row.number(21),
		UpdatedAt:            row.number(22),
	}
	if row.err != nil {
		return nil, row.err
	}
	return &result, nil
}
