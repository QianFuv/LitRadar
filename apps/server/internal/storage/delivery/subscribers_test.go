package delivery

import (
	"context"
	"errors"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
)

func TestSubscriberProjectionAuthenticatesSecretsAndPreservesPreferences(t *testing.T) {
	repository := testRepository(t)
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	backup := prepareSubscriberCredentials(t, repository, codec)
	item, err := repository.GetSubscriber(context.Background(), codec, 1)
	if err != nil {
		t.Fatal(err)
	}
	assertSubscriberCredentialProjection(t, item)
	items, err := repository.ListSubscribers(context.Background(), codec)
	if err != nil || len(items) != 1 {
		t.Fatalf("list: %v %d", err, len(items))
	}
	assertSubscriberLegacyRetryMinimum(t, repository, codec)
	assertSubscriberSecretContextAndDisabledAdmission(t, repository, codec, backup)

}

// TestSubscriberCorruptionKeepsOriginalFieldErrorPrecedence exercises ordered validation through generated queries.
func TestSubscriberCorruptionKeepsOriginalFieldErrorPrecedence(t *testing.T) {
	repository := testRepository(t)
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	if _, err := repository.database.Exec(`PRAGMA ignore_check_constraints=ON; DROP TRIGGER notification_settings_json_strings_insert; INSERT INTO notification_settings(user_id,keywords,ai_retry_attempts,created_at,updated_at) VALUES(1,'[1]',x'6261642d7265747279',1,1)`); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.GetSubscriber(context.Background(), codec, 1); !errors.Is(err, ErrNotificationList) {
		t.Fatalf("later malformed column hid earlier list error: %v", err)
	}
	if _, err := repository.database.Exec("UPDATE notification_settings SET keywords='[]',pushplus_token='old-plaintext'"); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.GetSubscriber(context.Background(), codec, 1); !errors.Is(err, secrets.ErrLegacyPlaintext) {
		t.Fatalf("later malformed column hid earlier credential error: %v", err)
	}
}

func TestSubscriberDependenciesFailClosedWithoutDroppingEnabledRows(t *testing.T) {
	repository := testRepository(t)
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	if _, err := repository.database.Exec("INSERT INTO notification_settings(user_id,created_at,updated_at) VALUES(1,1,1)"); err != nil {
		t.Fatal(err)
	}
	_, err = repository.ListSubscribers(context.Background(), codec)
	if err == nil || err.Error() != "a tracking folder is required when delivery_method is 'folder'" {
		t.Fatalf("missing folder skipped: %v", err)
	}
	if _, err := repository.database.Exec("UPDATE notification_settings SET delivery_method='pushplus'"); err != nil {
		t.Fatal(err)
	}
	_, err = repository.ListSubscribers(context.Background(), codec)
	if err == nil || err.Error() != "pushplus_token is required when delivery_method is 'pushplus'" {
		t.Fatalf("missing token skipped: %v", err)
	}
}

func TestNotificationListsRejectCoercionAndPreserveEmptyStrings(t *testing.T) {
	for _, raw := range []string{`null`, `{}`, `[null]`, `[1]`, `["\ud800"]`, `["x",]`} {
		if _, err := notificationStrings(raw); !errors.Is(err, ErrNotificationList) {
			t.Fatalf("invalid state accepted: %s %v", raw, err)
		}
	}
	values, err := notificationStrings(`[""," spaced ","same","same"]`)
	if err != nil || !reflect.DeepEqual(values, []string{"", " spaced ", "same", "same"}) {
		t.Fatal(values, err)
	}
}

func prepareSubscriberCredentials(t *testing.T, repository *Repository, codec *secrets.Codec) string {
	t.Helper()
	push, err := codec.Encrypt(" token ", secrets.NotificationContext(1, "pushplus_token"))
	if err != nil {
		t.Fatal(err)
	}
	primary, err := codec.Encrypt(" primary ", secrets.NotificationContext(1, "ai_api_key"))
	if err != nil {
		t.Fatal(err)
	}
	backup, err := codec.Encrypt(" backup ", secrets.NotificationContext(1, "ai_backup_api_key"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = repository.database.Exec(`INSERT INTO notification_settings(user_id,keywords,directions,selected_databases,delivery_method,pushplus_token,ai_api_key,ai_backup_api_key,ai_model,ai_system_prompt,created_at,updated_at) VALUES(1,'[" Rust ","Rust"]','["systems"]','["db.sqlite"]','pushplus',?,?,?,' model ','  ',1,1)`, push, primary, backup)
	if err != nil {
		t.Fatal(err)
	}
	return backup
}

func assertSubscriberCredentialProjection(t *testing.T, item *domain.Subscriber) {
	t.Helper()
	if item == nil || item.SubscriberID != "1" || !hasExpectedSubscriberSecrets(item) || *item.AiModel != "model" || item.AiSystemPrompt != nil || !reflect.DeepEqual(item.Keywords, []string{" Rust ", "Rust"}) {
		t.Fatalf("projection: %+v", item)
	}
}

func assertSubscriberLegacyRetryMinimum(t *testing.T, repository *Repository, codec *secrets.Codec) {
	t.Helper()
	if _, err := repository.database.Exec("PRAGMA ignore_check_constraints=ON; UPDATE notification_settings SET ai_retry_attempts=0; PRAGMA ignore_check_constraints=OFF;"); err != nil {
		t.Fatal(err)
	}
	item, err := repository.GetSubscriber(context.Background(), codec, 1)
	if err != nil || item.AiRetryAttempts != 1 {
		t.Fatalf("legacy retry minimum: %+v %v", item, err)
	}
}

func assertSubscriberSecretContextAndDisabledAdmission(t *testing.T, repository *Repository, codec *secrets.Codec, backup string) {
	t.Helper()
	if _, err := repository.database.Exec("UPDATE notification_settings SET ai_api_key=?", backup); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.GetSubscriber(context.Background(), codec, 1); !errors.Is(err, secrets.ErrAuthentication) {
		t.Fatalf("cross-field credential accepted: %v", err)
	}
	if _, err := repository.database.Exec("UPDATE notification_settings SET enabled=0"); err != nil {
		t.Fatal(err)
	}
	if item, err := repository.GetSubscriber(context.Background(), codec, 1); err != nil || item != nil {
		t.Fatalf("disabled corrupted row was read: %+v %v", item, err)
	}
}

func hasExpectedSubscriberSecrets(item *domain.Subscriber) bool {
	return item.PushplusToken == " token " && item.AiAPIKey != nil && *item.AiAPIKey == "primary" && item.AiBackupAPIKey != nil && *item.AiBackupAPIKey == "backup"
}
