package api

import (
	"context"
	"database/sql"
	"encoding/json"
	"testing"
)

// TestAdministratorUserListIncludesAccountIdentity protects the email and SSO controls.
func TestAdministratorUserListIncludesAccountIdentity(t *testing.T) {
	auth, router, token, member, _ := adminFixture(t)
	ctx := context.Background()
	err := auth.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		if _, err := connection.ExecContext(ctx, "UPDATE users SET email='sso@example.test',auth_kind='oidc' WHERE id=?", member.Id); err != nil {
			return err
		}
		_, err := connection.ExecContext(ctx, "INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(?,'https://identity.example.test','member',1,1)", member.Id)
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	response := authRequest(router, "GET", "/api/admin/users", "", token)
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	var users []map[string]json.RawMessage
	if err := json.Unmarshal(response.Body.Bytes(), &users); err != nil {
		t.Fatal(err)
	}
	hasMember := false
	for _, user := range users {
		for _, field := range []string{"email", "auth_kind", "is_oidc_linked"} {
			if _, exists := user[field]; !exists {
				t.Fatalf("administrator user row omits %s: %s", field, response.Body.String())
			}
		}
		if string(user["username"]) == `"member"` {
			hasMember = true
			if string(user["email"]) != `"sso@example.test"` || string(user["auth_kind"]) != `"oidc"` || string(user["is_oidc_linked"]) != "true" {
				t.Fatal("account identity does not match storage", string(user["email"]), string(user["auth_kind"]))
			}
		}
	}
	if !hasMember {
		t.Fatal("member is absent from administrator user list")
	}
}
