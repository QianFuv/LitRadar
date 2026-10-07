package settings

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	authstore "github.com/QianFuv/LitRadar/internal/storage/auth"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

func testSettings(t *testing.T) (*Repository, string) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	auth, err := authstore.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{7}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { auth.Close(); codec.Close() })
	return New(auth, codec), filename
}
func executeSettings(t *testing.T, repository *Repository, statement string, args ...any) {
	t.Helper()
	if err := repository.auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), statement, args...)
		return err
	}); err != nil {
		t.Fatal(err)
	}
}
func textValue(value string) *string { return &value }
func settingInfo(t *testing.T, items []Info, field string) Info {
	t.Helper()
	for _, item := range items {
		if item.Field == field {
			return item
		}
	}
	t.Fatal("missing setting", field)
	return Info{}
}

func TestSecretSettingsPreserveClearAndAuthenticatedPoolRemoval(t *testing.T) {
	repository, _ := testSettings(t)
	ctx := context.Background()
	field := "openalex_api_key_pool"
	initial, err := repository.Update(ctx, nil, map[string]*string{field: textValue(" first-secret;second-secret\nfirst-secret ")}, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	info := settingInfo(t, initial, field)
	assertInitialSecretProjection(t, info)
	originalReference := info.SecretItems[0].Reference
	updated, err := repository.Update(ctx, nil, map[string]*string{field: textValue("  ")}, map[string]PoolUpdate{field: {Remove: []string{originalReference, originalReference}, Add: []string{"third-secret;second-secret"}}}, nil)
	if err != nil {
		t.Fatal(err)
	}
	current, err := repository.LoadField(ctx, field)
	if err != nil || current.Value != "second-secret\nthird-secret" {
		t.Fatalf("pool update failed: %v", err)
	}
	for _, reference := range []string{originalReference, "plaintext"} {
		if _, err := repository.Update(ctx, nil, nil, map[string]PoolUpdate{field: {Remove: []string{reference}}}, nil); err == nil {
			t.Fatal("stale or unauthenticated removal accepted")
		}
	}
	reference := settingInfo(t, updated, field).SecretItems[0].Reference
	if _, err := repository.Update(ctx, nil, map[string]*string{"semantic_scholar_api_key_pool": textValue("second-secret")}, map[string]PoolUpdate{"semantic_scholar_api_key_pool": {Remove: []string{reference}}}, nil); err == nil {
		t.Fatal("cross-field authenticated reference accepted")
	}
	cleared, err := repository.Update(ctx, nil, map[string]*string{field: nil}, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	if settingInfo(t, cleared, field).HasValue {
		t.Fatal("null did not clear secret")
	}
}

// assertInitialSecretProjection preserves independent plaintext, presence and per-item masking checks.
func assertInitialSecretProjection(t *testing.T, info Info) {
	t.Helper()
	if info.Value != "" || !info.HasValue || info.MaskedValue != "••••" || len(info.SecretItems) != 2 || info.SecretItems[0].MaskedValue != "first*******" {
		t.Fatalf("invalid projection: %v", info)
	}
}

func TestSettingsRollbackAuthorityProxyAndAuditTogether(t *testing.T) {
	repository, _ := testSettings(t)
	ctx := context.Background()
	admin, err := repository.auth.Bootstrap(ctx, "admin", "hash", "", 1, nil)
	if err != nil {
		t.Fatal(err)
	}
	audit := domain.AuditEvent{Action: "runtime_settings_update", Outcome: "completed", OccurredAt: 2}
	if _, err := repository.Update(ctx, &admin.Id, map[string]*string{"provider_proxy_policy": textValue(`{"cnki":true}`)}, nil, &audit); err == nil {
		t.Fatal("enabled proxy without endpoint accepted")
	}
	if _, err := repository.Update(ctx, &admin.Id, map[string]*string{"provider_proxy_policy": textValue(`{"cnki":true}`), "provider_proxy_url": textValue("https://Proxy.Example:443")}, nil, &audit); err != nil {
		t.Fatal(err)
	}
	executeSettings(t, repository, `CREATE TRIGGER settings_audit_fault BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'sensitive fault'); END`)
	if _, err := repository.Update(ctx, &admin.Id, map[string]*string{"secure_cookies": textValue("true")}, nil, &audit); !errors.Is(err, domain.ErrAudit) {
		t.Fatalf("required audit did not fail closed: %v", err)
	}
	value, err := repository.LoadField(ctx, "secure_cookies")
	if err != nil || value.Value != "false" || value.Source != "default" {
		t.Fatal("failed audit retained setting")
	}
	executeSettings(t, repository, "UPDATE users SET is_admin=0 WHERE id=?", admin.Id)
	if _, err := repository.Update(ctx, &admin.Id, map[string]*string{"unknown": textValue("x")}, nil, nil); !errors.Is(err, domain.ErrAdminForbidden) {
		t.Fatal("validation preempted locked administrator check")
	}
}

func TestSettingsErrorsRetainCategoriesAndCorruptSecretsCannotBeOverwritten(t *testing.T) {
	repository, _ := testSettings(t)
	ctx := context.Background()
	for _, scenario := range []struct {
		field string
		value *string
		kind  ErrorKind
	}{{"missing", textValue("x"), UnknownSetting}, {"secure_cookies", nil, CannotClearNonSecret}, {"secure_cookies", textValue("bad"), InvalidSetting}} {
		_, err := repository.Update(ctx, nil, map[string]*string{scenario.field: scenario.value}, nil, nil)
		var typed Error
		if !errors.As(err, &typed) || typed.Kind != scenario.kind {
			t.Fatalf("lost error kind: %v", err)
		}
	}
	executeSettings(t, repository, "INSERT INTO runtime_settings VALUES('openalex_api_key_pool','litradarenc:v1:invalid',1)")
	if _, err := repository.Update(ctx, nil, map[string]*string{"openalex_api_key_pool": textValue("replacement")}, nil, nil); !errors.Is(err, secrets.ErrAuthentication) {
		t.Fatal("corrupt prior secret was overwritten")
	}
	if value, err := repository.LoadField(ctx, "audit_retention_days"); err != nil || value.Value != "180" {
		t.Fatal("unrelated secret blocked narrow loader")
	}
	if _, err := repository.List(ctx); err == nil {
		t.Fatal("full projection swallowed corrupt secret")
	}
}

func TestRuntimeRowsRejectCoercionIncludingUnknownFields(t *testing.T) {
	for _, statement := range []string{`INSERT INTO runtime_settings VALUES('secure_cookies',CAST('true' AS BLOB),1)`, `INSERT INTO runtime_settings VALUES('unknown','x','NaN')`, `INSERT INTO runtime_settings VALUES(CAST('unknown' AS BLOB),'x',1)`} {
		t.Run(statement, func(t *testing.T) {
			repository, _ := testSettings(t)
			executeSettings(t, repository, statement)
			if _, err := repository.Load(context.Background()); err == nil {
				t.Fatal("invalid storage class coerced into managed row")
			}
		})
	}
}

func TestLoggingBootstrapIsReadOnlyAndPreservesRawSettings(t *testing.T) {
	ctx := context.Background()
	assertMissingLoggingDefaults(t, ctx)
	repository, filename := testSettings(t)
	executeSettings(t, repository, `INSERT INTO runtime_settings VALUES('log_format','invalid raw format',1),('log_filter',' invalid raw filter ',2),('openalex_api_key_pool','invalid secret',3)`)
	if err := repository.auth.Close(); err != nil {
		t.Fatal(err)
	}
	before, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	logging, err := LoadLogging(ctx, filename)
	if err != nil || logging.LogFormat != "invalid raw format" || logging.LogFilter != " invalid raw filter " {
		t.Fatal("logging bootstrap validated or decrypted unrelated data", err)
	}
	after, err := os.ReadFile(filename)
	if err != nil || !bytes.Equal(before, after) {
		t.Fatal("readonly startup changed database")
	}
}

// assertMissingLoggingDefaults checks both default values and the absence of startup file creation.
func assertMissingLoggingDefaults(t *testing.T, ctx context.Context) {
	t.Helper()
	missing := filepath.Join(t.TempDir(), "absent", "auth.sqlite")
	if logging, err := LoadLogging(ctx, missing); err != nil || logging.LogFormat != "json" {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Dir(missing)); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("logging startup created directory")
	}
}

func TestSecretReferencesRemainPublicOnlyInExplicitJson(t *testing.T) {
	repository, _ := testSettings(t)
	items, err := repository.Update(context.Background(), nil, map[string]*string{"openalex_api_key_pool": textValue("secret-token")}, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	info := settingInfo(t, items, "openalex_api_key_pool")
	reference := info.SecretItems[0].Reference
	var logs bytes.Buffer
	logger := slog.New(slog.NewJSONHandler(&logs, nil))
	logger.Info("settings", "info", info, "item", info.SecretItems[0])
	rendered := fmt.Sprintf("%v %#v %v %#v", items, items, info.SecretItems[0], info.SecretItems[0]) + logs.String()
	if strings.Contains(rendered, reference) || strings.Contains(rendered, "secret-token") {
		t.Fatal("secret value or reference escaped debug redaction")
	}
	serialized, err := json.Marshal(info)
	if err != nil || !bytes.Contains(serialized, []byte(reference)) {
		t.Fatal("explicit administrator JSON lost removal reference")
	}
	for _, definition := range definitions {
		if secrets.IsRuntimeSecret(definition.Field) != definition.IsSecret {
			t.Fatal("secret maintenance registry drift", definition.Field)
		}
	}
}

func TestRatePolicyRejectsMalformedAndNonDominatingConfigurations(t *testing.T) {
	original := findDefinition("auth_rate_limit_policy").Default
	for _, mutated := range []string{strings.Replace(original, `"capacity":30`, `"capacity":30,"capacity":30`, 1), strings.Replace(original, `"capacity":30`, `"capacity":null`, 1), strings.Replace(original, `"capacity":30`, `"capacity":30.0`, 1), strings.Replace(original, `"capacity":1000`, `"capacity":30`, 1), strings.Replace(original, `"refill_tokens":100`, `"refill_tokens":1`, 1), strings.Replace(original, `"ip_key_limit":8192`, `"ip_key_limit":65537`, 1), strings.Replace(original, `"username":{`, `"extra":1,"username":{`, 1)} {
		if mutated == strings.Replace(original, `"refill_tokens":100`, `"refill_tokens":1`, 1) {
			mutated = strings.Replace(mutated, `"global_login":{"capacity":1000,"refill_tokens":1,"refill_seconds":1}`, `"global_login":{"capacity":1000,"refill_tokens":1,"refill_seconds":2}`, 1)
		}
		if _, err := ParseRateLimitPolicy(mutated); err == nil {
			t.Fatal("invalid rate policy accepted", mutated)
		}
	}
}
