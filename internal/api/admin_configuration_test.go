package api

import (
	"context"
	"database/sql"
	"encoding/json"
	"strings"
	"testing"

	scheduled "github.com/QianFuv/LitRadar/internal/domain/scheduler"
)

func TestAdministratorScheduledTaskValidationAndLifecycle(t *testing.T) {
	_, router, token, _, member := adminFixture(t)
	for _, scenario := range []struct {
		method, path, body, bearer string
		status                     int
		contains                   string
	}{
		{"GET", "/api/admin/scheduled-tasks", "", member, 403, "Admin access required"},
		{"POST", "/api/admin/scheduled-tasks", `{"name":" job ","job":{"kind":"index"},"cron":" * * * * * "}`, token, 200, `"timeout_seconds":3600`},
		{"GET", "/api/admin/scheduled-tasks", "", token, 200, `"name":"job"`},
		{"PUT", "/api/admin/scheduled-tasks/999", `{"name":" "}`, token, 400, "Task name must not be empty"},
		{"PUT", "/api/admin/scheduled-tasks/999", `{"cron":"bad","job":{"kind":"notify","max_candidates":0}}`, token, 400, "Cron expression must contain exactly five fields"},
		{"PUT", "/api/admin/scheduled-tasks/999", `{"job":{"kind":"notify","max_candidates":0},"timeout_seconds":0}`, token, 400, "max_candidates must be between"},
		{"PUT", "/api/admin/scheduled-tasks/999", `{"timezone":"invalid-zone","timeout_seconds":0}`, token, 400, "timezone must be a valid IANA name"},
		{"PUT", "/api/admin/scheduled-tasks/999", `{"timeout_seconds":0}`, token, 400, "timeout_seconds must be between"},
		{"PUT", "/api/admin/scheduled-tasks/999", `{}`, token, 404, "Scheduled task not found"},
		{"PUT", "/api/admin/scheduled-tasks/1", `{"job":["notify","fixture.sqlite",10],"enabled":false}`, token, 200, `"kind":"notify"`},
		{"GET", "/api/admin/scheduler/status", "", token, 200, `"recent_runs":[]`},
		{"DELETE", "/api/admin/scheduled-tasks/1", "", token, 200, `"ok":true`},
		{"DELETE", "/api/admin/scheduled-tasks/1", "", token, 404, "Scheduled task not found"},
	} {
		response := authRequest(router, scenario.method, scenario.path, scenario.body, scenario.bearer)
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.contains) {
			t.Fatal(scenario.path, response.Code, response.Body.String())
		}
	}
}

func TestAdministratorTaskAuditFailureRollsBackCreate(t *testing.T) {
	auth, router, token, _, _ := adminFixture(t)
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "CREATE TRIGGER reject_task_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'private'); END")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	response := authRequest(router, "POST", "/api/admin/scheduled-tasks", `{"name":"job","job":{"kind":"push"},"cron":"* * * * *"}`, token)
	if response.Code != 503 || strings.Contains(response.Body.String(), "private") {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "GET", "/api/admin/scheduled-tasks", "", token)
	var tasks []scheduled.Task
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &tasks) != nil || len(tasks) != 0 {
		t.Fatal("audit failure retained task", response.Code, response.Body.String())
	}
}

func TestAdministratorRuntimeSettingsValidateRawCapabilitiesAndMaskSecrets(t *testing.T) {
	_, router, token, _, member := adminFixture(t)
	for _, scenario := range []struct {
		method, path, body, bearer string
		status                     int
		contains                   string
	}{
		{"GET", "/api/admin/provider-catalog", "", member, 403, "Admin access required"},
		{"GET", "/api/admin/provider-catalog", "", token, 200, `"name":"zjlib"`},
		{"GET", "/api/admin/runtime-settings", "", token, 200, `"field":"article_abstract_provider_orders"`},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"log_format":" compact "}}`, token, 400, "Invalid LitRadar log format"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"index_provider_routes":"{\"fixture\":\"zjlib\"}"}}`, token, 400, "does not support the configured capability"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"article_abstract_provider_orders":"{\"default\":[\"CNKI\"],\"catalogs\":{}}"}}`, token, 400, "Provider orders must contain lowercase ASCII names"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"article_fulltext_provider_orders":"{\"default\":[\"zjlib_cnki\"],\"catalogs\":{}}"}}`, token, 400, "Unknown Provider: zjlib_cnki"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"article_fulltext_provider_orders":"{\"default\":[\"zjlib_cnki\"],\"catalogs\":{}}","log_format":"invalid"}}`, token, 400, "Invalid LitRadar log format"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"article_fulltext_provider_orders":"{\"default\":[\"scholarly\"],\"catalogs\":{}}"}}`, token, 400, "does not support the configured capability"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"provider_proxy_policy":"{\"unknown\":true}"}}`, token, 400, "Unknown Provider: unknown"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"unknown":"x"}}`, token, 400, "Unknown runtime setting: unknown"},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"cnki_captcha_token":"private-secret-token","log_format":"compact"}}`, token, 200, `"has_value":true`},
		{"PUT", "/api/admin/runtime-settings", `{"values":{"cnki_captcha_token":null}}`, token, 200, `"value":"compact"`},
	} {
		response := authRequest(router, scenario.method, scenario.path, scenario.body, scenario.bearer)
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.contains) || strings.Contains(response.Body.String(), "private-secret-token") {
			t.Fatal(scenario.path, scenario.body, response.Code, response.Body.String())
		}
	}
}
