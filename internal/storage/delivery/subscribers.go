package delivery

import (
	"context"
	"database/sql"
	"errors"
	"strconv"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/transport"
)

var ErrNotificationList = errors.New("Stored notification list state is invalid")

const subscriberSelect = `SELECT ns.user_id,u.username,ns.keywords,ns.directions,ns.selected_databases,
ns.delivery_method,ns.pushplus_token,ns.pushplus_template,ns.pushplus_topic,ns.pushplus_channel,
ns.sync_to_tracking_folder,ns.ai_base_url,ns.ai_api_key,ns.ai_model,ns.ai_system_prompt,
ns.ai_backup_base_url,ns.ai_backup_api_key,ns.ai_backup_model,ns.ai_backup_system_prompt,ns.ai_retry_attempts,
(SELECT id FROM folders f WHERE f.user_id=ns.user_id AND f.is_tracking=1 LIMIT 1)
FROM notification_settings ns JOIN users u ON u.id=ns.user_id WHERE ns.enabled=1`

// ListSubscribers reads enabled subscriber credentials and validates their required delivery dependencies.
func (repository *Repository) ListSubscribers(ctx context.Context, codec *secrets.Codec) ([]domain.Subscriber, error) {
	rows, err := repository.database.QueryContext(ctx, subscriberSelect+" ORDER BY ns.user_id")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []domain.Subscriber{}
	for rows.Next() {
		subscriber, err := scanSubscriber(rows, codec)
		if err != nil {
			return nil, err
		}
		result = append(result, *subscriber)
	}
	return result, rows.Err()
}

// GetSubscriber reads only the requested enabled user, returning nil for missing or disabled settings.
func (repository *Repository) GetSubscriber(ctx context.Context, codec *secrets.Codec, userId int64) (*domain.Subscriber, error) {
	return scanSubscriber(repository.database.QueryRowContext(ctx, subscriberSelect+" AND ns.user_id=?", userId), codec)
}

func scanSubscriber(scanner rowScanner, codec *secrets.Codec) (*domain.Subscriber, error) {
	row := readNotificationRow(scanner, 21)
	if errors.Is(row.err, sql.ErrNoRows) {
		return nil, nil
	}
	userId := row.integer(0)
	result := domain.Subscriber{
		SubscriberId: strconv.FormatInt(userId, 10), UserId: userId, Name: row.text(1), Keywords: row.strings(2), Directions: row.strings(3), SelectedDatabases: row.strings(4), DeliveryMethod: row.text(5),
		PushplusToken: row.secret(6, codec, userId, "pushplus_token"), Template: optionalTrimmed(row.text(7)), Topic: optionalTrimmed(row.text(8)), Channel: optionalTrimmed(row.text(9)), SyncToTrackingFolder: row.integer(10) != 0,
		AiBaseUrl: optionalTrimmed(row.text(11)), AiApiKey: optionalTrimmed(row.secret(12, codec, userId, "ai_api_key")), AiModel: optionalTrimmed(row.text(13)), AiSystemPrompt: optionalTrimmed(row.text(14)),
		AiBackupBaseUrl: optionalTrimmed(row.text(15)), AiBackupApiKey: optionalTrimmed(row.secret(16, codec, userId, "ai_backup_api_key")), AiBackupModel: optionalTrimmed(row.text(17)), AiBackupSystemPrompt: optionalTrimmed(row.text(18)), AiRetryAttempts: min(max(row.integer(19), 1), 10), TrackingFolderId: row.optionalInteger(20),
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
	parsed, err := transport.ParseJson([]byte(value))
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
	if method == "folder" && subscriber.TrackingFolderId == nil {
		return errors.New("A tracking folder is required when delivery_method is 'folder'")
	}
	if method != "pushplus" {
		return nil
	}
	if strings.TrimSpace(subscriber.PushplusToken) == "" {
		return errors.New("pushplus_token is required when delivery_method is 'pushplus'")
	}
	if subscriber.SyncToTrackingFolder && subscriber.TrackingFolderId == nil {
		return errors.New("A tracking folder is required before enabling PushPlus sync to tracking")
	}
	return nil
}
