package delivery

import (
	"context"
	"database/sql"
	"errors"
	"slices"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

var ErrEndpointNotAllowed = errors.New("Outbound endpoint is not allowed")

// UpsertNotificationSettings authenticates existing secrets and checks endpoint and folder policy in the same write transaction.
func (repository *Repository) UpsertNotificationSettings(ctx context.Context, codec *secrets.Codec, userId int64, update domain.NotificationSettingsUpdate) (*domain.NotificationSettings, error) {
	if err := domain.ValidateNotificationSettings(update); err != nil {
		return nil, err
	}
	encoded, err := encodeNotificationUpdateLists(update)
	if err != nil {
		return nil, err
	}
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer connection.Close()
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return nil, err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	prepared, err := prepareNotificationSettingsWrite(ctx, connection, codec, userId, update)
	if err != nil {
		return nil, err
	}
	now := float64(time.Now().UnixNano()) / 1e9
	_, err = connection.ExecContext(ctx, notificationUpsert, userId, encoded.keywords, encoded.directions, encoded.databases, update.DeliveryMethod, prepared.push, update.PushplusTemplate, update.PushplusTopic, update.PushplusChannel, update.SyncToTrackingFolder, prepared.primaryBase, prepared.primary, update.AiModel, update.AiSystemPrompt, prepared.backupBase, prepared.backup, update.AiBackupModel, update.AiBackupSystemPrompt, update.AiRetryAttempts, update.Enabled, now, now)
	if err != nil {
		return nil, err
	}
	stored, err := scanNotificationSettings(connection.QueryRowContext(ctx, "SELECT "+notificationColumns+" FROM notification_settings WHERE user_id=?", userId), codec)
	if err != nil {
		return nil, err
	}
	if stored == nil {
		return nil, sql.ErrNoRows
	}
	if _, err := connection.ExecContext(ctx, "COMMIT"); err != nil {
		return nil, err
	}
	return stored, nil
}

func encodeNotificationStrings(values []string) (string, error) {
	if values == nil {
		values = []string{}
	}
	encoded, err := jsonvalue.EncodeJson(values)
	return string(encoded), err
}

func selectedEndpoint(value string, allowed []string) (string, error) {
	if strings.TrimSpace(value) == "" {
		return "", nil
	}
	canonical, err := settings.CanonicalizeBaseUrl(value)
	if err != nil || !slices.Contains(allowed, canonical) {
		return "", ErrEndpointNotAllowed
	}
	return canonical, nil
}

func resolveNotificationSecret(codec *secrets.Codec, userId int64, field string, update domain.SecretUpdate, existing string) (string, error) {
	if !update.IsPresent {
		return existing, nil
	}
	if update.Value == nil {
		return "", nil
	}
	value := strings.TrimSpace(*update.Value)
	if value == "" {
		return existing, nil
	}
	return codec.Encrypt(value, secrets.NotificationContext(userId, field))
}

const notificationUpsert = `INSERT INTO notification_settings
(user_id,keywords,directions,selected_databases,delivery_method,pushplus_token,pushplus_template,pushplus_topic,pushplus_channel,sync_to_tracking_folder,ai_base_url,ai_api_key,ai_model,ai_system_prompt,ai_backup_base_url,ai_backup_api_key,ai_backup_model,ai_backup_system_prompt,ai_retry_attempts,enabled,created_at,updated_at)
VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
ON CONFLICT(user_id) DO UPDATE SET
keywords=excluded.keywords,directions=excluded.directions,selected_databases=excluded.selected_databases,delivery_method=excluded.delivery_method,
pushplus_token=excluded.pushplus_token,pushplus_template=excluded.pushplus_template,pushplus_topic=excluded.pushplus_topic,pushplus_channel=excluded.pushplus_channel,
sync_to_tracking_folder=excluded.sync_to_tracking_folder,ai_base_url=excluded.ai_base_url,ai_api_key=excluded.ai_api_key,ai_model=excluded.ai_model,ai_system_prompt=excluded.ai_system_prompt,
ai_backup_base_url=excluded.ai_backup_base_url,ai_backup_api_key=excluded.ai_backup_api_key,ai_backup_model=excluded.ai_backup_model,ai_backup_system_prompt=excluded.ai_backup_system_prompt,
ai_retry_attempts=excluded.ai_retry_attempts,enabled=excluded.enabled,updated_at=excluded.updated_at`

type notificationEncodedLists struct {
	keywords, directions, databases string
}

func encodeNotificationUpdateLists(update domain.NotificationSettingsUpdate) (notificationEncodedLists, error) {
	keywords, err := encodeNotificationStrings(update.Keywords)
	if err != nil {
		return notificationEncodedLists{}, err
	}
	directions, err := encodeNotificationStrings(update.Directions)
	if err != nil {
		return notificationEncodedLists{}, err
	}
	databases, err := encodeNotificationStrings(update.SelectedDatabases)
	if err != nil {
		return notificationEncodedLists{}, err
	}
	return notificationEncodedLists{keywords, directions, databases}, nil
}

type notificationPreparedSettings struct {
	primaryBase, backupBase, push, primary, backup string
}

func prepareNotificationSettingsWrite(ctx context.Context, connection *sql.Conn, codec *secrets.Codec, userId int64, update domain.NotificationSettingsUpdate) (notificationPreparedSettings, error) {
	allowed, err := settings.AiBaseUrlsInConnection(ctx, connection)
	if err != nil {
		return notificationPreparedSettings{}, err
	}
	primaryBase, err := selectedEndpoint(update.AiBaseUrl, allowed)
	if err != nil {
		return notificationPreparedSettings{}, err
	}
	backupBase, err := selectedEndpoint(update.AiBackupBaseUrl, allowed)
	if err != nil {
		return notificationPreparedSettings{}, err
	}
	existing, err := loadAuthenticatedNotificationSecrets(ctx, connection, codec, userId)
	if err != nil {
		return notificationPreparedSettings{}, err
	}
	push, err := resolveNotificationSecret(codec, userId, "pushplus_token", update.PushplusToken, string(existing.push))
	if err != nil {
		return notificationPreparedSettings{}, err
	}
	if err := validateNotificationWriteDependencies(ctx, connection, userId, update, push); err != nil {
		return notificationPreparedSettings{}, err
	}
	primary, err := resolveNotificationSecret(codec, userId, "ai_api_key", update.AiApiKey, string(existing.primary))
	if err != nil {
		return notificationPreparedSettings{}, err
	}
	backup, err := resolveNotificationSecret(codec, userId, "ai_backup_api_key", update.AiBackupApiKey, string(existing.backup))
	if err != nil {
		return notificationPreparedSettings{}, err
	}
	return notificationPreparedSettings{primaryBase, backupBase, push, primary, backup}, nil
}

type notificationExistingSecrets struct {
	push, primary, backup sqlite.Text
}

func loadAuthenticatedNotificationSecrets(ctx context.Context, connection *sql.Conn, codec *secrets.Codec, userId int64) (notificationExistingSecrets, error) {
	var currentPush, currentPrimary, currentBackup sqlite.Text
	err := connection.QueryRowContext(ctx, "SELECT pushplus_token,ai_api_key,ai_backup_api_key FROM notification_settings WHERE user_id=?", userId).Scan(&currentPush, &currentPrimary, &currentBackup)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return notificationExistingSecrets{}, err
	}
	if err == nil {
		for _, value := range []struct {
			field  string
			stored sqlite.Text
		}{{"pushplus_token", currentPush}, {"ai_api_key", currentPrimary}, {"ai_backup_api_key", currentBackup}} {
			if _, err := codec.Decrypt(string(value.stored), secrets.NotificationContext(userId, value.field)); err != nil {
				return notificationExistingSecrets{}, err
			}
		}
	}
	return notificationExistingSecrets{currentPush, currentPrimary, currentBackup}, nil
}

func validateNotificationWriteDependencies(ctx context.Context, connection *sql.Conn, userId int64, update domain.NotificationSettingsUpdate, push string) error {
	hasFolder := true
	if strings.TrimSpace(update.DeliveryMethod) == "folder" || update.SyncToTrackingFolder {
		if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM folders WHERE user_id=? AND is_tracking=1)", userId).Scan(&hasFolder); err != nil {
			return err
		}
	}
	var folder *int64
	if hasFolder {
		value := int64(1)
		folder = &value
	}
	if err := validateSubscriberDependencies(domain.Subscriber{DeliveryMethod: update.DeliveryMethod, PushplusToken: push, SyncToTrackingFolder: update.SyncToTrackingFolder, TrackingFolderId: folder}); err != nil {
		return err
	}
	return nil
}
