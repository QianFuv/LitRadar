package api

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"testing"
)

func requireCNKIProblem(t *testing.T, response *httptest.ResponseRecorder, status int, code, phase string) {
	t.Helper()
	problem := requireProblem(t, response, status, "")
	locations := map[string]string{}
	for _, detail := range problem.Errors {
		locations[detail.Location] = detail.Message
	}
	if locations["code"] != code || locations["phase"] != phase {
		t.Fatal("CNKI problem classifications changed", locations)
	}
}

// TestTypedDefaultsPreserveExplicitFalse checks that defaulting never re-enables disabled resources.
func TestTypedDefaultsPreserveExplicitFalse(t *testing.T) {
	_, router, token, _, _ := adminFixture(t)
	for _, scenario := range []struct {
		path, body string
		fields     []string
	}{
		{"/api/admin/announcements", `{"title":"Disabled","message":"Message","enabled":false}`, []string{"enabled"}},
		{"/api/admin/scheduled-tasks", `{"name":"Disabled","cron":"* * * * *","job":{"kind":"index"},"enabled":false,"coalesce":false}`, []string{"enabled", "coalesce"}},
	} {
		response := requireSuccess(t, router, "POST", scenario.path, scenario.body, token)
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(response.Body.Bytes(), &fields); err != nil {
			t.Fatal(err)
		}
		for _, name := range scenario.fields {
			if string(fields[name]) != "false" {
				t.Fatalf("%s default overwrote explicit false: %s", name, response.Body.String())
			}
		}
	}
	handlers, _, tracking, credential, _ := trackingFixture(t)
	if _, err := handlers.favorites.CreateFolder(context.Background(), 1, "Tracking", true); err != nil {
		t.Fatal(err)
	}
	response := requireSuccess(t, tracking, "PUT", "/api/tracking/notification-settings", `{"enabled":false}`, credential)
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(response.Body.Bytes(), &fields); err != nil {
		t.Fatal(err)
	}
	if string(fields["enabled"]) != "false" {
		t.Fatal("notification default overwrote explicit false", response.Body.String())
	}
}
