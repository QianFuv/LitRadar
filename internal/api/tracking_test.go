package api

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	storage "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func trackingFixture(t *testing.T) (*trackingHandlers, *authHandlers, *http.ServeMux, string, string) {
	t.Helper()
	auth, router, token, _, member := adminFixture(t)
	var databasePath, name string
	var sequence int
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		return connection.QueryRowContext(context.Background(), "PRAGMA database_list").Scan(&sequence, &name, &databasePath)
	}); err != nil {
		t.Fatal(err)
	}
	repository, err := storage.Open(databasePath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	handlers := &trackingHandlers{repository: repository, favorites: favorites.New(auth.repository), settings: settings.New(auth.repository, codec), codec: codec, authenticator: auth.authenticator, pool: auth.pool}
	handlers.storage.IndexDir = filepath.Join(t.TempDir(), "index")
	for _, route := range handlers.routes() {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	return handlers, auth, router, token, member
}

func TestTrackingSettingsPreserveSecretIntentAndMaskedProjection(t *testing.T) {
	handlers, _, router, token, _ := trackingFixture(t)
	response := authRequest(router, "GET", "/api/tracking/notification-settings", "", token)
	if response.Code != 200 || response.Body.String() != "null" {
		t.Fatal(response.Code, response.Body.String())
	}
	if _, err := handlers.favorites.CreateFolder(context.Background(), 1, "Tracking", true); err != nil {
		t.Fatal(err)
	}
	for _, body := range []string{
		`{"pushplus_token":" private-push ","ai_api_key":" private-ai ","ai_backup_api_key":" private-backup ","keywords":[" x ","","x"],"pushplus_template":" "}`,
		`{}`,
		`{"pushplus_token":" ","ai_api_key":"","ai_backup_api_key":" "}`,
		`{"pushplus_token":null,"ai_api_key":null,"ai_backup_api_key":null}`,
	} {
		response := authRequest(router, "PUT", "/api/tracking/notification-settings", body, token)
		if response.Code != 200 || strings.Contains(response.Body.String(), "private-") {
			t.Fatal(response.Code, response.Body.String())
		}
		var value domain.NotificationSettingsResponse
		if err := json.Unmarshal(response.Body.Bytes(), &value); err != nil {
			t.Fatal(err)
		}
		shouldHaveSecret := !strings.Contains(body, "null")
		if value.HasPushplusToken != shouldHaveSecret || value.HasAiApiKey != shouldHaveSecret || value.HasAiBackupApiKey != shouldHaveSecret {
			t.Fatal(response.Body.String())
		}
		stored, err := handlers.repository.GetNotificationSettings(context.Background(), handlers.codec, 1)
		if err != nil {
			t.Fatal(err)
		}
		if shouldHaveSecret && (stored.PushplusToken != "private-push" || stored.AiApiKey != "private-ai" || stored.AiBackupApiKey != "private-backup") {
			t.Fatal("secret intent changed")
		}
		if strings.Contains(body, "keywords") && (strings.Join(value.Keywords, ",") != "x,x" || value.PushplusTemplate != "markdown") {
			t.Fatal(response.Body.String())
		}
	}
}

func TestTrackingValidationOrderAndDatabaseNormalization(t *testing.T) {
	handlers, auth, router, token, _ := trackingFixture(t)
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "DELETE FROM folders WHERE user_id=1")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(handlers.storage.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(handlers.storage.IndexDir, "known.sqlite"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct {
		body, bearer string
		status       int
		detail       string
	}{
		{`{"enabled":null}`, "", 422, "expected a boolean"},
		{`{"ai_retry_attempts":0}`, "", 401, "Authentication required"},
		{`{"ai_retry_attempts":0}`, token, 400, "ai_retry_attempts must be between"},
		{`{"selected_databases":["absent","absent"],"delivery_method":"bad"}`, token, 400, "Unknown databases: absent.sqlite"},
		{`{"delivery_method":" folder "}`, token, 400, "delivery_method must be one of"},
		{`{"enabled":false}`, token, 400, "A tracking folder is required when"},
		{`{"delivery_method":"pushplus","sync_to_tracking_folder":true}`, token, 400, "pushplus_token is required"},
		{`{"delivery_method":"pushplus","pushplus_token":"test","sync_to_tracking_folder":true}`, token, 400, "before enabling PushPlus sync"},
		{`{"delivery_method":"pushplus","pushplus_token":"test","ai_base_url":"https://unapproved.example/"}`, token, 400, "AI endpoint is not available"},
		{`{"delivery_method":"pushplus","pushplus_token":"test","selected_databases":["known","known.sqlite"]}`, token, 200, `"selected_databases":[]`},
	} {
		response := authRequest(router, "PUT", "/api/tracking/notification-settings", scenario.body, scenario.bearer)
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.detail) {
			t.Fatal(scenario.body, response.Code, response.Body.String())
		}
	}
	allowed := "https://ai.example/v1/"
	if _, err := handlers.settings.Update(context.Background(), nil, map[string]*string{"ai_allowed_base_urls": &allowed}, nil, nil); err != nil {
		t.Fatal(err)
	}
	response := authRequest(router, "PUT", "/api/tracking/notification-settings", `{"delivery_method":"pushplus","ai_base_url":" https://AI.EXAMPLE/v1 "}`, token)
	if response.Code != 200 || !strings.Contains(response.Body.String(), `"ai_base_url":"https://ai.example/v1/"`) {
		t.Fatal(response.Code, response.Body.String())
	}
}

func TestManualPushDurableAdmissionOwnershipAndCancellation(t *testing.T) {
	handlers, _, router, token, member := trackingFixture(t)
	response := authRequest(router, "GET", "/api/tracking/push-weekly/status", "", token)
	if response.Code != 200 || !strings.Contains(response.Body.String(), `"status":"idle"`) {
		t.Fatal(response.Code, response.Body.String())
	}
	var first manualPushStatus
	for attempt := 0; attempt < 2; attempt++ {
		response = authRequest(router, "POST", "/api/tracking/push-weekly", "", token)
		var current manualPushStatus
		if response.Code != 202 || json.Unmarshal(response.Body.Bytes(), &current) != nil || current.JobId == nil || len(*current.JobId) != 32 || current.Status != "pending" || !current.CanCancel {
			t.Fatal(response.Code, response.Body.String())
		}
		if attempt == 0 {
			first = current
		} else if *first.JobId != *current.JobId {
			t.Fatal("duplicate admission created another job")
		}
	}
	record, err := handlers.repository.LoadLatestManualRun(context.Background(), 1)
	if err != nil || record.Status != storage.RunStatusQueued || record.DeadlineAt == nil || *record.DeadlineAt-record.CreatedAt != 600 {
		t.Fatal(record, err)
	}
	path := "/api/tracking/push-weekly/runs/" + *first.JobId
	if response := authRequest(router, "GET", path, "", member); response.Code != 404 {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "POST", path+"/cancel", "", token)
	if response.Code != 200 || !strings.Contains(response.Body.String(), `"status":"cancelled"`) || !strings.Contains(response.Body.String(), `"can_retry":true`) {
		t.Fatal(response.Code, response.Body.String())
	}
	if response := authRequest(router, "POST", path+"/cancel", "", token); response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "POST", "/api/tracking/push-weekly", "", member)
	var second manualPushStatus
	if response.Code != 202 || json.Unmarshal(response.Body.Bytes(), &second) != nil || second.JobId == nil {
		t.Fatal(response.Code, response.Body.String())
	}
	if response := authRequest(router, "POST", "/api/tracking/push-weekly/runs/"+*second.JobId+"/cancel", "", token); response.Code != 200 {
		t.Fatal("admin cannot cancel another owner", response.Code, response.Body.String())
	}
	for _, bearer := range []string{"", token} {
		response := authRequest(router, "GET", "/api/tracking/push-weekly/runs/invalid", "", bearer)
		expected := 404
		if bearer == "" {
			expected = 401
		}
		if response.Code != expected {
			t.Fatal(response.Code, response.Body.String())
		}
	}
}

func TestManualUnknownAcknowledgementRemainsOwnerOnlyAndAtomic(t *testing.T) {
	handlers, auth, router, token, member := trackingFixture(t)
	response := authRequest(router, "POST", "/api/tracking/push-weekly", "", member)
	var status manualPushStatus
	if response.Code != 202 || json.Unmarshal(response.Body.Bytes(), &status) != nil || status.JobId == nil {
		t.Fatal(response.Code, response.Body.String())
	}
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "UPDATE delivery_runs SET status='unknown',finished_at=?,revision=revision+1 WHERE external_id=?", currentTimestamp(), *status.JobId)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if response := authRequest(router, "POST", "/api/tracking/push-weekly", "", member); response.Code != 409 {
		t.Fatal(response.Code, response.Body.String())
	}
	path := "/api/tracking/push-weekly/runs/" + *status.JobId + "/acknowledge"
	if response := authRequest(router, "POST", path, "", token); response.Code != 404 {
		t.Fatal("admin acquired owner-only acknowledgement", response.Code, response.Body.String())
	}
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "CREATE TRIGGER reject_manual_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'private'); END")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if response := authRequest(router, "POST", path, "", member); response.Code != 500 {
		t.Fatal("manual audit failure must be500", response.Code, response.Body.String())
	}
	latest, err := handlers.repository.LoadLatestManualRun(context.Background(), 2)
	if err != nil || latest == nil || latest.Status != storage.RunStatusUnknown {
		t.Fatal("audit failure admitted replacement", latest, err)
	}
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "DROP TRIGGER reject_manual_audit")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if response := authRequest(router, "POST", path, "", member); response.Code != 202 {
		t.Fatal(response.Code, response.Body.String())
	}
	if response := authRequest(router, "POST", path, "", member); response.Code != 409 {
		t.Fatal(response.Code, response.Body.String())
	}
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		var count, actor, target int64
		var action, outcome, correlation string
		if err := connection.QueryRowContext(context.Background(), "SELECT count(*),actor_id,target_id,action,outcome,request_id FROM security_audit_events WHERE action='manual_push_unknown_acknowledge'").Scan(&count, &actor, &target, &action, &outcome, &correlation); err != nil {
			return err
		}
		if count != 1 || actor != 2 || target != latest.Id || action != "manual_push_unknown_acknowledge" || outcome != "completed" || correlation != "fixture-request" {
			t.Errorf("acknowledgement audit identity: %d %d %d %s %s %s", count, actor, target, action, outcome, correlation)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestManualPublicOutcomeRequiresStrictDecodedState(t *testing.T) {
	for _, encoded := range []string{`{}`, `{"status":"completed","message":"private","pushed":null,"selected":1,"summary":"x"}`, `{"status":"invented","message":"private","pushed":1,"selected":1,"summary":"x"}`, `{"status":"completed","status":"completed","message":"private","pushed":1,"selected":1,"summary":"x"}`, `{"status":{"completed":null,"completed":null},"message":"private","pushed":1,"selected":1,"summary":"x"}`} {
		value := manualStatus(&storage.RunRecord{Status: storage.RunStatusCompleted, ResultJson: &encoded})
		if value.Message != "Manual push completed" || value.Pushed != 0 || value.Summary != "" {
			t.Fatal("invalid result disclosed", encoded, value)
		}
	}
	encoded := `{"status":{"completed":null},"message":"Public result","pushed":7,"selected":8,"summary":"Public summary"}`
	for _, state := range []storage.RunStatus{storage.RunStatusCompleted, storage.RunStatusRunning, storage.RunStatusUnknown} {
		value := manualStatus(&storage.RunRecord{Status: state, ResultJson: &encoded})
		if value.Pushed != 7 || value.Selected != 8 || value.Summary != "Public summary" {
			t.Fatal(value)
		}
		if state == storage.RunStatusCompleted && value.Message != "Public result" || state != storage.RunStatusCompleted && value.Message == "Public result" {
			t.Fatal(value)
		}
		if state == storage.RunStatusUnknown && value.CanRetry {
			t.Fatal("ambiguous run can retry without acknowledgement")
		}
	}
}

func TestTrackingNormalizedEndpointLengthRemainsClientError(t *testing.T) {
	handlers, _, router, token, _ := trackingFixture(t)
	prefix := "https://ai.example/"
	endpoint := prefix + strings.Repeat("a", 2048-len(prefix))
	if _, err := handlers.settings.Update(context.Background(), nil, map[string]*string{"ai_allowed_base_urls": &endpoint}, nil, nil); err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"ai_base_url", "ai_backup_base_url"} {
		body, _ := json.Marshal(map[string]string{field: endpoint})
		response := authRequest(router, "PUT", "/api/tracking/notification-settings", string(body), token)
		if response.Code != 400 || !strings.Contains(response.Body.String(), field+" must be at most 2048 characters") {
			t.Fatal(field, response.Code, response.Body.String())
		}
	}
}
