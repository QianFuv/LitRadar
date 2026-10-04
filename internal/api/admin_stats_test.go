package api

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestAdminStatsPreservesPartialIndexAndLegacyPushSemantics(t *testing.T) {
	auth, _, _ := authFixture(t)
	ctx := context.Background()
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	database, err := sqlite.OpenPlain(filepath.Join(configuration.IndexDir, "good.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = database.ExecContext(ctx, "CREATE TABLE articles(id INTEGER);INSERT INTO articles VALUES(1),(2);CREATE TABLE journals(id INTEGER);INSERT INTO journals VALUES(1)")
	database.Close()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(configuration.IndexDir, "bad.sqlite"), []byte("not sqlite"), 0600); err != nil {
		t.Fatal(err)
	}
	directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	for name, content := range map[string]string{"good.json": `{"status":"completed","last_completed_run_at":"stamp","run":{"delivered_article_ids":[1,2],"user_results":[]}}`, "scalar.json": "null", "broken.json": "{", "ignore.changes.json": "{}", "upper.JSON": "{}"} {
		if err := os.WriteFile(filepath.Join(directory, name), []byte(content), 0600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Mkdir(filepath.Join(directory, "directory.json"), 0700); err != nil {
		t.Fatal(err)
	}
	err = auth.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "UPDATE access_tokens SET expires_at=0")
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	result, err := readAdminStats(ctx, auth.repository, configuration)
	if err != nil {
		t.Fatal(err)
	}
	if result.Auth.TotalUsers != 1 || result.Auth.AdminCount != 1 || result.Auth.ActiveTokens != 0 {
		t.Fatal(result.Auth)
	}
	if len(result.Index.Databases) != 2 || result.Index.TotalArticles != 2 || result.Index.TotalJournals != 1 || result.Index.Databases[0].Error == nil || result.Index.Databases[1].Error != nil || result.Index.Databases[1].Issues != 0 {
		t.Fatal(result.Index)
	}
	if len(result.Push) != 4 || result.Push[0].Status != "error" || result.Push[1].Status != "error" || result.Push[2].Status != "completed" || result.Push[3].Status != "unknown" {
		t.Fatal(result.Push)
	}
	if result.Push[2].DeliveredCount == nil || *result.Push[2].DeliveredCount != 2 || result.Push[2].UserResults == nil || *result.Push[2].UserResults != 0 {
		t.Fatal(result.Push[2])
	}
	encoded, err := json.Marshal(result.Push[3])
	if err != nil || strings.Contains(string(encoded), "delivered_count") || !strings.Contains(string(encoded), `"last_completed":null`) {
		t.Fatal(string(encoded), err)
	}
	var count int64
	err = auth.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		return connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM access_tokens").Scan(&count)
	})
	if err != nil || count != 0 {
		t.Fatal("expired tokens not cleaned", count, err)
	}
}

func TestAdminStatsCleanupSurvivesLaterFailure(t *testing.T) {
	auth, _, _ := authFixture(t)
	ctx := context.Background()
	configuration := config.FromProjectRoot(t.TempDir())
	err := auth.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "UPDATE access_tokens SET expires_at=0;DROP TABLE announcements")
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := readAdminStats(ctx, auth.repository, configuration); err == nil {
		t.Fatal("missing table ignored")
	}
	var count int64
	err = auth.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		return connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM access_tokens").Scan(&count)
	})
	if err != nil || count != 0 {
		t.Fatal("cleanup incorrectly rolled back", count, err)
	}
}
