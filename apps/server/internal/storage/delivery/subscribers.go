package delivery

import (
	"context"
	"database/sql"
	"errors"
	"strconv"
	"strings"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/notificationdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
)

var ErrNotificationList = errors.New("stored notification list state is invalid")

// ListSubscribers reads enabled subscriber credentials and validates their required delivery dependencies.
func (repository *Repository) ListSubscribers(ctx context.Context, codec *secrets.Codec) ([]domain.Subscriber, error) {
	rows, err := notificationQueries.Subscribers(ctx, repository.database)
	if err != nil {
		return nil, err
	}
	result := make([]domain.Subscriber, 0, len(rows))
	for _, row := range rows {
		subscriber, err := decodeSubscriber(notificationdb.SubscriberRow(row), nil, codec)
		if err != nil {
			return nil, err
		}
		result = append(result, *subscriber)
	}
	return result, nil
}

// GetSubscriber reads only the requested enabled user, returning nil for missing or disabled settings.
func (repository *Repository) GetSubscriber(ctx context.Context, codec *secrets.Codec, userID int64) (*domain.Subscriber, error) {
	row, err := notificationQueries.Subscriber(ctx, repository.database, notificationdb.SubscriberParams{UserID: sqlite.Integer(userID)})
	return decodeSubscriber(row, err, codec)
}

func decodeSubscriber(record notificationdb.SubscriberRow, err error, codec *secrets.Codec) (*domain.Subscriber, error) {
	row := &notificationRow{values: []any{record.UserID, record.Username, record.Keywords, record.Directions, record.SelectedDatabases, record.DeliveryMethod, record.PushplusToken, record.PushplusTemplate, record.PushplusTopic, record.PushplusChannel, record.SyncToTrackingFolder, record.AIBaseURL, record.AIAPIKey, record.AIModel, record.AISystemPrompt, record.AIBackupBaseURL, record.AIBackupAPIKey, record.AIBackupModel, record.AIBackupSystemPrompt, record.AIRetryAttempts, record.ID}, err: err}
	if errors.Is(row.err, sql.ErrNoRows) {
		return nil, nil
	}
	userID := row.integer(0)
	result := domain.Subscriber{
		SubscriberID: strconv.FormatInt(userID, 10), UserID: userID, Name: row.text(1), Keywords: row.strings(2), Directions: row.strings(3), SelectedDatabases: row.strings(4), DeliveryMethod: row.text(5),
		PushplusToken: row.secret(6, codec, userID, "pushplus_token"), Template: optionalTrimmed(row.text(7)), Topic: optionalTrimmed(row.text(8)), Channel: optionalTrimmed(row.text(9)), SyncToTrackingFolder: row.integer(10) != 0,
		AiBaseURL: optionalTrimmed(row.text(11)), AiAPIKey: optionalTrimmed(row.secret(12, codec, userID, "ai_api_key")), AiModel: optionalTrimmed(row.text(13)), AiSystemPrompt: optionalTrimmed(row.text(14)),
		AiBackupBaseURL: optionalTrimmed(row.text(15)), AiBackupAPIKey: optionalTrimmed(row.secret(16, codec, userID, "ai_backup_api_key")), AiBackupModel: optionalTrimmed(row.text(17)), AiBackupSystemPrompt: optionalTrimmed(row.text(18)), AiRetryAttempts: min(max(row.integer(19), 1), 10), TrackingFolderID: row.optionalInteger(20),
	}
	if row.err != nil {
		return nil, row.err
	}
	if err := validateSubscriberDependencies(result); err != nil {
		return nil, err
	}
	return &result, nil
}

func notificationStrings(value string) ([]string, error) {
	parsed, err := transport.ParseJSON([]byte(value))
	if err != nil {
		return nil, ErrNotificationList
	}
	items, ok := parsed.([]any)
	if !ok {
		return nil, ErrNotificationList
	}
	result := make([]string, 0, len(items))
	for _, item := range items {
		value, ok := item.(string)
		if !ok {
			return nil, ErrNotificationList
		}
		result = append(result, value)
	}
	return result, nil
}

func optionalTrimmed(value string) *string {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	return &value
}

func validateSubscriberDependencies(subscriber domain.Subscriber) error {
	method := strings.TrimSpace(subscriber.DeliveryMethod)
	if method == "folder" && subscriber.TrackingFolderID == nil {
		return errors.New("a tracking folder is required when delivery_method is 'folder'")
	}
	if method != "pushplus" {
		return nil
	}
	if strings.TrimSpace(subscriber.PushplusToken) == "" {
		return errors.New("pushplus_token is required when delivery_method is 'pushplus'")
	}
	if subscriber.SyncToTrackingFolder && subscriber.TrackingFolderID == nil {
		return errors.New("a tracking folder is required before enabling PushPlus sync to tracking")
	}
	return nil
}
