package api

import (
	"context"
	"encoding/json"
	"fmt"
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

// TestAdminSchemaValidationPrecedesAuthentication checks constraints shared with generated clients.
func TestAdminSchemaValidationPrecedesAuthentication(t *testing.T) {
	_, router, token, _, _ := adminFixture(t)
	for _, scenario := range []struct {
		name, method, path, body, location string
	}{
		{"priority", "POST", "/api/admin/announcements", `{"title":"Notice","message":"Message","priority":" HIGH "}`, "body.priority"},
		{"task name", "POST", "/api/admin/scheduled-tasks", `{"name":"","cron":"* * * * *","job":{"kind":"index"}}`, "body.name"},
		{"task cron", "PUT", "/api/admin/scheduled-tasks/1", `{"cron":""}`, "body.cron"},
		{"create task timezone", "POST", "/api/admin/scheduled-tasks", `{"name":"Task","cron":"* * * * *","timezone":"","job":{"kind":"index"}}`, "body.timezone"},
		{"task timezone", "PUT", "/api/admin/scheduled-tasks/1", `{"timezone":""}`, "body.timezone"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			for _, bearer := range []string{"", token} {
				response := authRequest(router, scenario.method, scenario.path, scenario.body, bearer)
				problem := requireProblem(t, response, 422, "validation failed")
				if len(problem.Errors) != 1 || problem.Errors[0].Location != scenario.location {
					t.Fatal("missing field validation detail", response.Body.String())
				}
			}
		})
	}
}

// TestFavoriteQueryDefaults treats omitted and empty query values consistently with Huma defaults.
func TestFavoriteQueryDefaults(t *testing.T) {
	_, handlers, router, token := favoriteFixture(t)
	folder, err := handlers.repository.CreateFolder(context.Background(), 1, "Defaults", false)
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct{ suffix, query string }{
		{"articles", "?limit=&offset="},
		{"articles/page", "?limit="},
		{"export", "?format="},
	} {
		path := fmt.Sprintf("/api/favorites/folders/%d/%s", folder.ID, scenario.suffix)
		omitted := authRequest(router, "GET", path, "", token)
		empty := authRequest(router, "GET", path+scenario.query, "", token)
		if omitted.Code != 200 || empty.Code != 200 || omitted.Body.String() != empty.Body.String() || omitted.Header().Get("Content-Type") != empty.Header().Get("Content-Type") {
			t.Fatalf("query defaults differ for %s: %d %s; %d %s", path, omitted.Code, omitted.Body.String(), empty.Code, empty.Body.String())
		}
	}
}
