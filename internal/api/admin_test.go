package api

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func adminFixture(t *testing.T) (*authHandlers, *http.ServeMux, string, domain.User, string) {
	t.Helper()
	auth, router, token := authFixture(t)
	invite, err := auth.service.IssueInvite(context.Background(), 1, false, nil)
	if err != nil {
		t.Fatal(err)
	}
	user, err := auth.service.Register(context.Background(), "member", "long enough password", &invite.Code, nil)
	if err != nil {
		t.Fatal(err)
	}
	memberToken, err := auth.service.CreateTrustedToken(context.Background(), user.Id, "member", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	var databasePath, databaseName string
	var sequence int
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		return connection.QueryRowContext(context.Background(), "PRAGMA database_list").Scan(&sequence, &databaseName, &databasePath)
	}); err != nil {
		t.Fatal(err)
	}
	tasks, err := scheduler.Open(databasePath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { tasks.Close() })
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	handlers := adminHandlers{auth: auth, storage: config.FromProjectRoot(t.TempDir()), scheduler: tasks, settings: settings.New(auth.repository, codec)}
	for _, route := range handlers.routes() {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	return auth, router, token, user, memberToken.Token
}

func TestAdministratorRoutesEnforceRolesSelfProtectionAndNumericIds(t *testing.T) {
	_, router, token, user, memberToken := adminFixture(t)
	target := strconv.FormatInt(int64(user.Id), 10)
	for _, scenario := range []struct {
		method, path, body, bearer string
		status                     int
		contains                   string
	}{
		{"GET", "/api/admin/users", "", memberToken, 403, "Admin access required"},
		{"GET", "/api/admin/users", "", token, 200, `"id":1`},
		{"PUT", "/api/admin/users/1/admin", `{"is_admin":false}`, token, 400, "Cannot revoke own admin status"},
		{"DELETE", "/api/admin/users/1", "", token, 400, "Cannot delete yourself"},
		{"POST", "/api/admin/users/99999/reset-password", `{"new_password":"short"}`, token, 400, "Password must be"},
		{"POST", "/api/admin/users/99999/reset-password", `{"new_password":"long enough password"}`, token, 404, "User not found"},
		{"PUT", "/api/admin/users/" + target + "/admin", `{"is_admin":true}`, token, 200, `"ok":true`},
		{"GET", "/api/admin/stats", "", memberToken, 200, `"admin_count":2`},
		{"PUT", "/api/admin/users/" + target + "/admin", `{"is_admin":false}`, token, 200, `"ok":true`},
		{"GET", "/api/admin/stats", "", memberToken, 403, "Admin access required"},
		{"POST", "/api/admin/users/" + target + "/reset-password", `{"new_password":"replacement password"}`, token, 200, `"ok":true`},
		{"GET", "/api/auth/me", "", memberToken, 401, "Invalid or expired token"},
		{"GET", "/api/admin/invite-codes", "", token, 200, `"created_by":1`},
		{"DELETE", "/api/admin/users/" + target, "", token, 200, `"ok":true`},
		{"DELETE", "/api/admin/users/" + target, "", token, 404, "User not found"},
	} {
		response := authRequest(router, scenario.method, scenario.path, scenario.body, scenario.bearer)
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.contains) {
			t.Fatal(scenario.method, scenario.path, response.Code, response.Body.String())
		}
	}
}

func TestAdministratorInviteOptionalBodyAndRevocation(t *testing.T) {
	_, router, token, _, _ := adminFixture(t)
	request := httptest.NewRequest("POST", "/api/admin/invite-codes", strings.NewReader("ignored without content type"))
	request.Header.Set("Authorization", "Bearer "+token)
	response := httptest.NewRecorder()
	router.ServeHTTP(response, request)
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	var invite struct {
		Id        int64
		CreatedBy *int64 `json:"created_by"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &invite); err != nil || invite.Id == 0 || invite.CreatedBy != nil {
		t.Fatal(response.Body.String(), err)
	}
	for _, status := range []int{200, 404} {
		response = authRequest(router, "DELETE", "/api/admin/invite-codes/"+strconv.FormatInt(invite.Id, 10), "", token)
		if response.Code != status {
			t.Fatal(response.Code, response.Body.String())
		}
	}
	response = authRequest(router, "POST", "/api/admin/invite-codes", `{"max_uses":0}`, token)
	if response.Code != 400 {
		t.Fatal(response.Code, response.Body.String())
	}
}

func TestAdministratorAnnouncementsValidateBeforeLookupAndAuditChanges(t *testing.T) {
	_, router, token, _, member := adminFixture(t)
	for _, scenario := range []struct {
		method, path, body, bearer string
		status                     int
		contains                   string
	}{
		{"POST", "/api/admin/announcements", `{"title":"test","message":"test"}`, member, 403, "Admin access required"},
		{"PUT", "/api/admin/announcements/999", `{"title":" "}`, token, 400, "Title must be"},
		{"PUT", "/api/admin/announcements/999", `{"priority":"urgent"}`, token, 400, "Priority must be"},
		{"PUT", "/api/admin/announcements/999", `{}`, token, 404, "Announcement not found"},
		{"POST", "/api/admin/announcements", `{"title":" title ","message":" message ","priority":" HIGH "}`, token, 200, `"priority":"high"`},
		{"GET", "/api/admin/announcements", "", token, 200, `"title":"title"`},
		{"PUT", "/api/admin/announcements/1", `{"enabled":false,"title":null}`, token, 200, `"enabled":false`},
		{"DELETE", "/api/admin/announcements/1", "", token, 200, `"ok":true`},
		{"DELETE", "/api/admin/announcements/1", "", token, 404, "Announcement not found"},
	} {
		response := authRequest(router, scenario.method, scenario.path, scenario.body, scenario.bearer)
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.contains) {
			t.Fatal(scenario.method, scenario.path, response.Code, response.Body.String())
		}
	}
}

func TestAdministratorCompletionAuditFailureRollsBackWithoutSecondAudit(t *testing.T) {
	auth, router, token, user, _ := adminFixture(t)
	ctx := context.Background()
	err := auth.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "CREATE TRIGGER reject_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT, 'private failure'); END")
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	before := storage.AuditFailureCount()
	response := authRequest(router, "PUT", "/api/admin/users/"+strconv.FormatInt(int64(user.Id), 10)+"/admin", `{"is_admin":true}`, token)
	if response.Code != 503 || storage.AuditFailureCount() != before+1 {
		t.Fatal(response.Code, response.Body.String(), storage.AuditFailureCount()-before)
	}
	users, err := auth.repository.ListUsers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range users {
		if row.Id == user.Id && row.IsAdmin {
			t.Fatal("failed audit committed role change")
		}
	}
	before = storage.AuditFailureCount()
	response = authRequest(router, "DELETE", "/api/admin/users/1", "", token)
	if response.Code != 503 || storage.AuditFailureCount() != before+1 {
		t.Fatal("rejection audit did not override self-delete failure", response.Code, response.Body.String())
	}
}
