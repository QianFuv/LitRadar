package auth

import (
	"context"
	"database/sql"
	"fmt"
	"strings"
	"testing"
)

// TestTokenExpiryIndexPreservesRowsAndBoundary covers both fresh and version-20 databases.
func TestTokenExpiryIndexPreservesRowsAndBoundary(t *testing.T) {
	for _, version := range []int{0, 20} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			filename, connection := historicalDatabase(t, version)
			if version == 20 {
				seedTokenExpiryBoundary(t, connection)
			}
			if summary, err := Migrate(context.Background(), filename); err != nil || summary != (Summary{version, SchemaVersion}) {
				t.Fatalf("upgrade: %+v %v", summary, err)
			}
			if version == 0 {
				seedTokenExpiryBoundary(t, connection)
			}
			if count := scalar[int](t, connection, "SELECT count(*) FROM access_tokens"); count != 3 {
				t.Fatal("migration changed token rows", count)
			}
			if columns := scalar[string](t, connection, "SELECT coalesce(group_concat(name),'') FROM pragma_index_info('idx_access_tokens_expires')"); columns != "expires_at" {
				t.Fatal("missing expiry index", columns)
			}
			rows, err := connection.QueryContext(context.Background(), "EXPLAIN QUERY PLAN DELETE FROM access_tokens WHERE expires_at<=?", 100)
			if err != nil {
				t.Fatal(err)
			}
			hasExpirySearch := false
			planDetails := []string{}
			for rows.Next() {
				var identifier, parent, unused int
				var detail string
				if err := rows.Scan(&identifier, &parent, &unused, &detail); err != nil {
					rows.Close()
					t.Fatal(err)
				}
				hasExpirySearch = hasExpirySearch || strings.HasPrefix(detail, "SEARCH access_tokens ") && strings.Contains(detail, "idx_access_tokens_expires")
				planDetails = append(planDetails, detail)
			}
			if err := rows.Err(); err != nil {
				rows.Close()
				t.Fatal(err)
			}
			rows.Close()
			if !hasExpirySearch {
				t.Fatal("token cleanup did not use an expiry range search", planDetails)
			}
			mustExecute(t, connection, "DELETE FROM access_tokens WHERE expires_at<=?", 100)
			if remaining := scalar[string](t, connection, "SELECT group_concat(token_hash) FROM access_tokens"); remaining != "future" {
				t.Fatal("expiry boundary changed", remaining)
			}
			if summary, err := Migrate(context.Background(), filename); err != nil || summary != (Summary{SchemaVersion, SchemaVersion}) {
				t.Fatalf("repeat migration: %+v %v", summary, err)
			}
		})
	}
}

// TestTokenExpiryIndexFailureRetainsVersionAndRows protects the upgrade transaction.
func TestTokenExpiryIndexFailureRetainsVersionAndRows(t *testing.T) {
	filename, connection := historicalDatabase(t, 20)
	seedTokenExpiryBoundary(t, connection)
	mustExecute(t, connection, "CREATE TABLE idx_access_tokens_expires(marker TEXT)")
	if _, err := Migrate(context.Background(), filename); err == nil {
		t.Fatal("conflicting schema object accepted")
	}
	if version := scalar[int](t, connection, "PRAGMA user_version"); version != 20 {
		t.Fatal("failed migration changed schema version", version)
	}
	if count := scalar[int](t, connection, "SELECT count(*) FROM access_tokens"); count != 3 {
		t.Fatal("failed migration changed token rows", count)
	}
}

// seedTokenExpiryBoundary inserts synthetic tokens immediately before, at and after expiry.
func seedTokenExpiryBoundary(t *testing.T, connection *sql.Conn) {
	t.Helper()
	mustExecute(t, connection, `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'synthetic','hash','salt',1,1);
INSERT INTO access_tokens(user_id,token_hash,name,expires_at,created_at) VALUES
(1,'expired','personal',99.5,1),(1,'boundary','personal',100,1),(1,'future','personal',100.5,1)`)
}
