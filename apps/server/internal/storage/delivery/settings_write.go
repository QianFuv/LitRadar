package delivery

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"slices"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/notificationdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

var ErrEndpointNotAllowed = errors.New("Outbound endpoint is not allowed")

// UpsertNotificationSettings authenticates existing secrets and checks endpoint and folder policy in the same write transaction.
func (repository *Repository) UpsertNotificationSettings(ctx context.Context, codec *secrets.Codec, userId int64, update domain.NotificationSettingsUpdate) (resultValue1 *domain.NotificationSettings, resultError error) {
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
	defer func() {
		if closeError := connection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return nil, err
	}
	hasTransaction := true
	defer func() {
		if hasTransaction {
			if _, rollbackError := connection.ExecContext(context.Background(), "ROLLBACK"); rollbackError != nil {
				slog.Error("storage.rollback.failed", "error", rollbackError)
			}
		}
	}()
	prepared, err := prepareNotificationSettingsWrite(ctx, connection, codec, userId, update)
	if err != nil {
		return nil, err
	}
	now := float64(time.Now().UnixNano()) / 1e9
	_, err = queries.StoreNotificationSettings(ctx, connection, deliverydb.StoreNotificationSettingsParams{UserID: sqlite.Integer(userId), Keywords: sqlite.Text(encoded.keywords), Directions: sqlite.Text(encoded.directions), SelectedDatabases: sqlite.Text(encoded.databases), DeliveryMethod: sqlite.Text(update.DeliveryMethod), PushplusToken: sqlite.Text(prepared.push), PushplusTemplate: sqlite.Text(update.PushplusTemplate), PushplusTopic: sqlite.Text(update.PushplusTopic), PushplusChannel: sqlite.Text(update.PushplusChannel), SyncToTrackingFolder: sqlite.Integer(dbtypes.Boolean(update.SyncToTrackingFolder)), AIBaseURL: sqlite.Text(prepared.primaryBase), AIAPIKey: sqlite.Text(prepared.primary), AIModel: sqlite.Text(update.AiModel), AISystemPrompt: sqlite.Text(update.AiSystemPrompt), AIBackupBaseURL: sqlite.Text(prepared.backupBase), AIBackupAPIKey: sqlite.Text(prepared.backup), AIBackupModel: sqlite.Text(update.AiBackupModel), AIBackupSystemPrompt: sqlite.Text(update.AiBackupSystemPrompt), AIRetryAttempts: sqlite.Integer(update.AiRetryAttempts), Enabled: sqlite.Integer(dbtypes.Boolean(update.Enabled)), CreatedAt: sqlite.Number(now), UpdatedAt: sqlite.Number(now)})
	if err != nil {
		return nil, err
	}
	row, err := notificationQueries.NotificationSettings(ctx, connection, notificationdb.NotificationSettingsParams{UserID: sqlite.Integer(userId)})
	stored, err := decodeNotificationSettings(row, err, codec)
	if err != nil {
		return nil, err
	}
	if stored == nil {
		return nil, sql.ErrNoRows
	}
	if _, err := connection.ExecContext(ctx, "COMMIT"); err != nil {
		return nil, err
	}
	hasTransaction = false
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
	err := func() error {
		queryResult, err := queries.NotificationSecrets(ctx, connection, deliverydb.NotificationSecretsParams{UserID: sqlite.Integer(userId)})
		if err != nil {
			return err
		}
		currentPush = queryResult.PushplusToken
		currentPrimary = queryResult.AIAPIKey
		currentBackup = queryResult.AIBackupAPIKey
		return nil
	}()
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
		if err := func() error {
			queryResult, err := queries.HasTrackingFolder(ctx, connection, deliverydb.HasTrackingFolderParams{UserID: sqlite.Integer(userId)})
			if err != nil {
				return err
			}
			hasFolder = queryResult
			return nil
		}(); err != nil {
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
