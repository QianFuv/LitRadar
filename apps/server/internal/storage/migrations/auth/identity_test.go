package auth

import (
	"context"
	"testing"
)

// TestVersionTwentyThreeIdentityConstraints preserves old credentials and enforces identity ownership.
func TestVersionTwentyThreeIdentityConstraints(t *testing.T) {
	filename, connection := historicalDatabase(t, 22)
	mustExecute(t, connection, "INSERT INTO users(username,password_hash,salt,created_at,updated_at) VALUES('first','old-hash','old-salt',1,1),('second','hash','salt',1,1)")
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	if kind := scalar[string](t, connection, "SELECT auth_kind FROM users WHERE id=1"); kind != "local" {
		t.Fatal(kind)
	}
	if hash := scalar[string](t, connection, "SELECT password_hash FROM users WHERE id=1"); hash != "old-hash" {
		t.Fatal("legacy credentials changed")
	}
	mustExecute(t, connection, "UPDATE users SET email='first@example.com' WHERE id=1")
	for _, statement := range []string{"UPDATE users SET email='first@example.com' WHERE id=2", "UPDATE users SET auth_kind='other' WHERE id=2"} {
		if err := execute(context.Background(), connection, statement); err == nil {
			t.Fatal("invalid account accepted")
		}
	}
	mustExecute(t, connection, "INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(1,'https://issuer','subject',1,1)")
	for _, statement := range []string{
		"INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(2,'https://issuer','subject',1,1)",
		"INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(1,'https://issuer','other',1,1)",
		"INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(999,'https://issuer','orphan',1,1)",
	} {
		if err := execute(context.Background(), connection, statement); err == nil {
			t.Fatal("invalid identity accepted")
		}
	}
	mustExecute(t, connection, "DELETE FROM users WHERE id=1")
	if count := scalar[int](t, connection, "SELECT count(*) FROM user_identities"); count != 0 {
		t.Fatal("identity did not cascade")
	}
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal("migration is not idempotent", err)
	}
}
