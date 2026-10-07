package delivery

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

func TestNotificationSettingsKeepClearAndReplaceCredentials(t *testing.T) {
	repository := testRepository(t)
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	ctx := context.Background()
	if _, err := repository.database.Exec("INSERT INTO runtime_settings(key,value,updated_at) VALUES('ai_allowed_base_urls','https://api.example.test/v1/',1)"); err != nil {
		t.Fatal(err)
	}
	update := domain.DefaultNotificationSettingsUpdate()
	update.DeliveryMethod = "pushplus"
	update.PushplusToken = domain.SecretUpdate{IsPresent: true, Value: pointer(" private-token ")}
	update.AiApiKey = domain.SecretUpdate{IsPresent: true, Value: pointer("private-key")}
	update.AiBaseUrl = " https://API.example.test/v1 "
	stored, err := repository.UpsertNotificationSettings(ctx, codec, 1, update)
	if err != nil {
		t.Fatal(err)
	}
	encrypted := assertEncryptedNotificationCredentials(t, repository, stored)
	assertNotificationSecretsArePreserved(t, repository, ctx, codec, &update, stored, encrypted)
	assertNotificationSecretClearAndPublicMask(t, repository, ctx, codec, &update)
	assertRequiredNotificationSecretClearRollsBack(t, repository, ctx, codec, &update)

}

func TestNotificationSettingsRejectRevokedEndpointsAndCorruptExistingSecrets(t *testing.T) {
	repository := testRepository(t)
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	ctx := context.Background()
	if _, err := repository.database.Exec("INSERT INTO folders(user_id,name,is_tracking,created_at,updated_at) VALUES(1,'tracking',1,1,1)"); err != nil {
		t.Fatal(err)
	}
	update := domain.DefaultNotificationSettingsUpdate()
	update.AiBaseUrl = "https://revoked.example/v1"
	if _, err := repository.UpsertNotificationSettings(ctx, codec, 1, update); !errors.Is(err, ErrEndpointNotAllowed) {
		t.Fatalf("unapproved endpoint accepted: %v", err)
	}
	update.AiBaseUrl = ""
	if _, err := repository.UpsertNotificationSettings(ctx, codec, 1, update); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.database.Exec("UPDATE notification_settings SET ai_backup_api_key='plaintext-secret'"); err != nil {
		t.Fatal(err)
	}
	update.AiBackupApiKey = domain.SecretUpdate{IsPresent: true}
	if _, err := repository.UpsertNotificationSettings(ctx, codec, 1, update); !errors.Is(err, secrets.ErrLegacyPlaintext) {
		t.Fatalf("corrupt old secret silently cleared: %v", err)
	}
	var raw string
	if err := repository.database.QueryRow("SELECT ai_backup_api_key FROM notification_settings").Scan(&raw); err != nil || raw != "plaintext-secret" {
		t.Fatal("failed secret authentication changed row")
	}
}

func TestNotificationSettingsRequiredReadbackRollsBackWrite(t *testing.T) {
	repository := testRepository(t)
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	if _, err := repository.database.Exec("INSERT INTO folders(user_id,name,is_tracking,created_at,updated_at) VALUES(1,'tracking',1,1,1); CREATE TRIGGER remove_settings AFTER INSERT ON notification_settings BEGIN DELETE FROM notification_settings WHERE id=NEW.id; END;"); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.UpsertNotificationSettings(context.Background(), codec, 1, domain.DefaultNotificationSettingsUpdate()); err == nil {
		t.Fatal("missing write readback committed")
	}
	var count int
	if err := repository.database.QueryRow("SELECT COUNT(*) FROM notification_settings").Scan(&count); err != nil || count != 0 {
		t.Fatal("failed transaction persisted")
	}
}

func assertEncryptedNotificationCredentials(t *testing.T, repository *Repository, stored *domain.NotificationSettings) string {
	t.Helper()
	if stored.AiBaseUrl != "https://api.example.test/v1/" || stored.PushplusToken != "private-token" || stored.AiApiKey != "private-key" {
		t.Fatal(stored)
	}
	var encrypted string
	if err := repository.database.QueryRow("SELECT pushplus_token FROM notification_settings WHERE user_id=1").Scan(&encrypted); err != nil {
		t.Fatal(err)
	}
	if encrypted == "private-token" || !strings.HasPrefix(encrypted, "litradarenc:v1:") {
		t.Fatal("credential stored in plaintext")
	}
	return encrypted
}

func assertNotificationSecretsArePreserved(t *testing.T, repository *Repository, ctx context.Context, codec *secrets.Codec, update *domain.NotificationSettingsUpdate, stored *domain.NotificationSettings, encrypted string) {
	t.Helper()
	update.PushplusToken = domain.SecretUpdate{}
	update.AiApiKey = domain.SecretUpdate{IsPresent: true, Value: pointer("  ")}
	changed, err := repository.UpsertNotificationSettings(ctx, codec, 1, *update)
	if err != nil {
		t.Fatal(err)
	}
	if changed.CreatedAt != stored.CreatedAt || changed.AiApiKey != "private-key" {
		t.Fatal(changed)
	}
	var preserved string
	if err := repository.database.QueryRow("SELECT pushplus_token FROM notification_settings WHERE user_id=1").Scan(&preserved); err != nil || preserved != encrypted {
		t.Fatal("omitted secret was re-encrypted")
	}
}

func assertNotificationSecretClearAndPublicMask(t *testing.T, repository *Repository, ctx context.Context, codec *secrets.Codec, update *domain.NotificationSettingsUpdate) {
	t.Helper()
	update.AiApiKey = domain.SecretUpdate{IsPresent: true}
	changed, err := repository.UpsertNotificationSettings(ctx, codec, 1, *update)
	if err != nil || changed.AiApiKey != "" {
		t.Fatal(changed, err)
	}
	public, err := json.Marshal(changed.Public())
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(public), "private-token") || strings.Contains(string(public), "private-key") || !strings.Contains(string(public), "••••") {
		t.Fatal("public projection leaked credentials or lost fixed masks")
	}
}

func assertRequiredNotificationSecretClearRollsBack(t *testing.T, repository *Repository, ctx context.Context, codec *secrets.Codec, update *domain.NotificationSettingsUpdate) {
	t.Helper()
	update.PushplusToken = domain.SecretUpdate{IsPresent: true}
	if _, err := repository.UpsertNotificationSettings(ctx, codec, 1, *update); err == nil {
		t.Fatal("cleared required PushPlus token accepted")
	}
	unchanged, err := repository.GetNotificationSettings(ctx, codec, 1)
	if err != nil || unchanged.PushplusToken != "private-token" {
		t.Fatal("failed dependency update persisted", err)
	}
}
