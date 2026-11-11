package delivery

import (
	"context"
	"database/sql"
	"errors"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/notificationdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// GetNotificationSettings reads raw preferences and decrypted credentials, including disabled settings.
func (repository *Repository) GetNotificationSettings(ctx context.Context, codec *secrets.Codec, userID int64) (*domain.NotificationSettings, error) {
	row, err := notificationQueries.NotificationSettings(ctx, repository.database, notificationdb.NotificationSettingsParams{UserID: sqlite.Integer(userID)})
	return decodeNotificationSettings(row, err, codec)
}
func decodeNotificationSettings(record notificationdb.NotificationSetting, err error, codec *secrets.Codec) (*domain.NotificationSettings, error) {
	row := &notificationRow{values: []any{record.ID, record.UserID, record.Keywords, record.Directions, record.SelectedDatabases, record.DeliveryMethod, record.PushplusToken, record.PushplusTemplate, record.PushplusTopic, record.PushplusChannel, record.SyncToTrackingFolder, record.AIBaseURL, record.AIAPIKey, record.AIModel, record.AISystemPrompt, record.AIBackupBaseURL, record.AIBackupAPIKey, record.AIBackupModel, record.AIBackupSystemPrompt, record.AIRetryAttempts, record.Enabled, record.CreatedAt, record.UpdatedAt}, err: err}
	if errors.Is(row.err, sql.ErrNoRows) {
		return nil, nil
	}
	userID := row.integer(1)
	result := domain.NotificationSettings{
		ID:                   row.integer(0),
		UserID:               userID,
		Keywords:             row.strings(2),
		Directions:           row.strings(3),
		SelectedDatabases:    row.strings(4),
		DeliveryMethod:       row.text(5),
		PushplusToken:        row.secret(6, codec, userID, "pushplus_token"),
		PushplusTemplate:     row.text(7),
		PushplusTopic:        row.text(8),
		PushplusChannel:      row.text(9),
		SyncToTrackingFolder: row.integer(10) != 0,
		AiBaseURL:            row.text(11),
		AiAPIKey:             row.secret(12, codec, userID, "ai_api_key"),
		AiModel:              row.text(13),
		AiSystemPrompt:       row.text(14),
		AiBackupBaseURL:      row.text(15),
		AiBackupAPIKey:       row.secret(16, codec, userID, "ai_backup_api_key"),
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
