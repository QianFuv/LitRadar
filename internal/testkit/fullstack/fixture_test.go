package fullstack

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	cfpstorage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	indexstorage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func TestUnmarkedOrPopulatedRootIsNeverSeeded(t *testing.T) {
	root := t.TempDir()
	sentinel := filepath.Join(root, "operator-data.txt")
	if err := os.WriteFile(sentinel, []byte("preserve"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := seed(context.Background(), root); err == nil || !strings.Contains(err.Error(), "marker is missing") {
		t.Fatal("unmarked root admitted", err)
	}
	if content, err := os.ReadFile(sentinel); err != nil || string(content) != "preserve" {
		t.Fatal("sentinel changed", err)
	}
	if _, err := os.Stat(filepath.Join(root, "data")); !os.IsNotExist(err) {
		t.Fatal("unmarked root mutated", err)
	}
	if err := os.WriteFile(filepath.Join(root, markerFile), []byte(markerContent), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(root, "data"), 0700); err != nil {
		t.Fatal(err)
	}
	if _, err := seed(context.Background(), root); err == nil || !strings.Contains(err.Error(), "already exists") {
		t.Fatal("existing data admitted", err)
	}
}

func TestFixtureSeedsRealStorageAndRefreshesOriginalOverHttp(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, markerFile), []byte(markerContent), 0600); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	var output bytes.Buffer
	if err := Run(ctx, []string{"--project-root", root}, &output); err != nil {
		t.Fatal(err)
	}
	if output.String() != "{\"article_count\":2,\"database\":\"full-stack.sqlite\",\"status\":\"seeded\",\"user_count\":2,\"weekly_article_count\":1}\n" {
		t.Fatal(output.String())
	}
	configuration := config.FromProjectRoot(root)
	accounts, err := authstorage.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer accounts.Close()
	users, err := accounts.ListUsers(ctx)
	if err != nil || len(users) != 2 {
		t.Fatal("fixture accounts missing", users, err)
	}
	content, err := indexstorage.OpenContent(ctx, filepath.Join(configuration.IndexDir, databaseName))
	if err != nil {
		t.Fatal(err)
	}
	defer content.Close()
	var count int
	if err := content.QueryRowContext(ctx, "SELECT count(*) FROM articles").Scan(&count); err != nil || count != 2 {
		t.Fatal(count, err)
	}
	var authors string
	var articleId int64
	if err := content.QueryRowContext(ctx, "SELECT article_id,authors_json FROM articles WHERE doi=?", articleDoi).Scan(&articleId, &authors); err != nil || authors != `["Ada Lovelace","Grace Hopper"]` {
		t.Fatal("legacy author format not retained", authors, err)
	}
	manifest, err := os.ReadFile(filepath.Join(root, "data", "push_state", "full-stack.changes.json"))
	if err != nil {
		t.Fatal(err)
	}
	var weekly struct {
		Ids         []int64 `json:"notifiable_article_ids"`
		GeneratedAt string  `json:"generated_at"`
	}
	if err := json.Unmarshal(manifest, &weekly); err != nil || len(weekly.Ids) != 1 || weekly.Ids[0] != articleId || weekly.GeneratedAt == "" {
		t.Fatal("weekly fixture changed", weekly, err)
	}
	repository, err := cfpstorage.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	before, err := repository.LoadJournals(ctx)
	if err != nil || len(before) != 1 || len(before[0].Notices) != 1 || before[0].Notices[0].Title != "Initial original CFP" {
		t.Fatal("initial CFP missing", before, err)
	}
	output.Reset()
	if err := Run(ctx, []string{"--project-root", root, "--refresh-cfp"}, &output); err != nil {
		t.Fatal(err)
	}
	if output.String() != "{\"notices\":1,\"status\":\"cfp_updated\"}\n" {
		t.Fatal(output.String())
	}
	after, err := repository.LoadJournals(ctx)
	if err != nil || len(after) != 1 || len(after[0].Notices) != 1 || after[0].Notices[0].Title != "Updated original CFP after backend refresh" {
		t.Fatal("real refresh did not publish", after, err)
	}
	if len(after[0].Sources) != 1 || after[0].Sources[0].Status != "success" || after[0].Sources[0].LeaseExpiresAt != nil {
		t.Fatal("refresh ownership not released", after[0].Sources)
	}
}
