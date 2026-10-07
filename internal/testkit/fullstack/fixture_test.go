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

// TestUnmarkedOrPopulatedRootIsNeverSeeded proves both protections on the same owned root.
func TestUnmarkedOrPopulatedRootIsNeverSeeded(t *testing.T) {
	root := t.TempDir()
	sentinel := filepath.Join(root, "operator-data.txt")
	if err := os.WriteFile(sentinel, []byte("preserve"), 0600); err != nil {
		t.Fatal(err)
	}
	assertUnmarkedFixtureUntouched(t, root, sentinel)
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

// TestFixtureSeedsRealStorageAndRefreshesOriginalOverHttp retains real database owners across the local refresh.
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
	assertFixtureAccounts(t, ctx, accounts)
	content, err := indexstorage.OpenContent(ctx, filepath.Join(configuration.IndexDir, databaseName))
	if err != nil {
		t.Fatal(err)
	}
	defer content.Close()
	articleId := fixtureArticleIdentity(t, ctx, content)
	assertFixtureWeeklyManifest(t, root, articleId)
	repository, err := cfpstorage.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	assertInitialFixtureCfp(t, ctx, repository)
	output.Reset()
	if err := Run(ctx, []string{"--project-root", root, "--refresh-cfp"}, &output); err != nil {
		t.Fatal(err)
	}
	if output.String() != "{\"notices\":1,\"status\":\"cfp_updated\"}\n" {
		t.Fatal(output.String())
	}
	assertRefreshedFixtureCfp(t, ctx, repository)
}

// assertUnmarkedFixtureUntouched verifies missing-marker admission without changing operator data.
func assertUnmarkedFixtureUntouched(t *testing.T, root, sentinel string) {
	t.Helper()
	if _, err := seed(context.Background(), root); err == nil || !strings.Contains(err.Error(), "marker is missing") {
		t.Fatal("unmarked root admitted", err)
	}
	if content, err := os.ReadFile(sentinel); err != nil || string(content) != "preserve" {
		t.Fatal("sentinel changed", err)
	}
	if _, err := os.Stat(filepath.Join(root, "data")); !os.IsNotExist(err) {
		t.Fatal("unmarked root mutated", err)
	}
}

// assertFixtureAccounts retains the two-user storage expectation.
func assertFixtureAccounts(t *testing.T, ctx context.Context, accounts *authstorage.Repository) {
	t.Helper()
	users, err := accounts.ListUsers(ctx)
	if err != nil || len(users) != 2 {
		t.Fatal("fixture accounts missing", users, err)
	}
}

// fixtureArticleIdentity checks both articles and the first article's legacy authors.
func fixtureArticleIdentity(t *testing.T, ctx context.Context, content *indexstorage.Connection) int64 {
	t.Helper()
	var count int
	if err := content.QueryRowContext(ctx, "SELECT count(*) FROM articles").Scan(&count); err != nil || count != 2 {
		t.Fatal(count, err)
	}
	var authors string
	var articleId int64
	if err := content.QueryRowContext(ctx, "SELECT article_id,authors_json FROM articles WHERE doi=?", articleDoi).Scan(&articleId, &authors); err != nil || authors != `["Ada Lovelace","Grace Hopper"]` {
		t.Fatal("legacy author format not retained", authors, err)
	}
	return articleId
}

// assertFixtureWeeklyManifest checks one matching ID and a nonempty generation timestamp.
func assertFixtureWeeklyManifest(t *testing.T, root string, articleId int64) {
	t.Helper()
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
}

// assertInitialFixtureCfp checks the original notice before the refresh.
func assertInitialFixtureCfp(t *testing.T, ctx context.Context, repository *cfpstorage.Repository) {
	t.Helper()
	before, err := repository.LoadJournals(ctx)
	if err != nil || len(before) != 1 || len(before[0].Notices) != 1 || before[0].Notices[0].Title != "Initial original CFP" {
		t.Fatal("initial CFP missing", before, err)
	}
}

// assertRefreshedFixtureCfp checks publication followed by released source ownership.
func assertRefreshedFixtureCfp(t *testing.T, ctx context.Context, repository *cfpstorage.Repository) {
	t.Helper()
	after, err := repository.LoadJournals(ctx)
	if err != nil || len(after) != 1 || len(after[0].Notices) != 1 || after[0].Notices[0].Title != "Updated original CFP after backend refresh" {
		t.Fatal("real refresh did not publish", after, err)
	}
	if len(after[0].Sources) != 1 || after[0].Sources[0].Status != "success" || after[0].Sources[0].LeaseExpiresAt != nil {
		t.Fatal("refresh ownership not released", after[0].Sources)
	}
}
