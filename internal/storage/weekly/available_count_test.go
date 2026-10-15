package weekly

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/config"
)

// writeAvailablePublication writes an independent publication with explicit source identity and membership.
func writeAvailablePublication(t testing.TB, configuration config.Config, database, run string, ids []int64) {
	t.Helper()
	directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	body, err := json.Marshal(map[string]any{"db_name": database, "run_id": run, "generated_at": "2026-10-03T12:00:00Z", "notifiable_article_ids": ids})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, run+".changes.json"), body, 0600); err != nil {
		t.Fatal(err)
	}
}

// availableCountFixture builds overlapping publications and listing rows whose orphan journals remain unavailable.
func availableCountFixture(t testing.TB, count int) (config.Config, Timestamp) {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, count, count+2)
	for index := range ids {
		ids[index] = int64(index + 1)
	}
	ids = append(ids, 1, int64(count+1))
	for _, name := range []string{"alpha", "beta"} {
		database, err := platform.Open(platform.Config{Filename: filepath.Join(configuration.IndexDir, name+".sqlite"), Mode: "rwc", MaxConnections: 1})
		if err != nil {
			t.Fatal(err)
		}
		if _, err := database.Exec("CREATE TABLE journals(journal_id INTEGER PRIMARY KEY); INSERT INTO journals VALUES(1); CREATE TABLE article_listing(article_id INTEGER PRIMARY KEY,journal_id INTEGER)"); err != nil {
			database.Close()
			t.Fatal(err)
		}
		_, err = database.Exec(`WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM ids WHERE id<?) INSERT INTO article_listing SELECT id,CASE WHEN id%10=0 THEN 999 ELSE 1 END FROM ids`, count)
		closeError := database.Close()
		if err != nil || closeError != nil {
			t.Fatal(err, closeError)
		}
		for publication := range 2 {
			writeAvailablePublication(t, configuration, name+".sqlite", fmt.Sprintf("%s-%d", name, publication), ids)
		}
	}
	end, _ := ParseTimestamp("2026-10-03T12:00:00Z")
	return configuration, end
}

// availableCountReference retains the original publication pruning and cross-publication pair deduplication.
func availableCountReference(ctx context.Context, configuration config.Config, end Timestamp, selected []string) (int, error) {
	manifests, err := LoadAvailable(ctx, configuration, end, selected)
	if err != nil {
		return 0, err
	}
	type reference struct {
		name string
		id   int64
	}
	unique := map[reference]bool{}
	for _, manifest := range manifests {
		for _, id := range manifest.ArticleIds {
			unique[reference{manifest.DbName, id}] = true
		}
	}
	return len(unique), nil
}

// TestCountAvailableRetainsDatabasePairIdentity checks overlap, selection, missing content and live availability.
func TestCountAvailableRetainsDatabasePairIdentity(t *testing.T) {
	configuration, end := availableCountFixture(t, 10)
	writeAvailablePublication(t, configuration, "missing.sqlite", "missing", []int64{1, 2})
	ctx := context.Background()
	for _, scenario := range []struct {
		selected []string
		expected int
	}{{nil, 18}, {[]string{"alpha.sqlite"}, 9}, {[]string{"alpha.sqlite", "alpha.sqlite"}, 9}, {[]string{"missing.sqlite"}, 0}, {[]string{"unpublished.sqlite"}, 0}} {
		actual, err := CountAvailable(ctx, configuration, end, scenario.selected)
		reference, referenceError := availableCountReference(ctx, configuration, end, scenario.selected)
		if err != nil || referenceError != nil || actual != scenario.expected || actual != reference {
			t.Fatal("available pair count changed", scenario.selected, actual, reference, err, referenceError)
		}
	}
	database, err := platform.Open(platform.Config{Filename: filepath.Join(configuration.IndexDir, "alpha.sqlite"), Mode: "rw", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("INSERT INTO journals VALUES(999)"); err != nil {
		database.Close()
		t.Fatal(err)
	}
	if err := database.Close(); err != nil {
		t.Fatal(err)
	}
	if actual, err := CountAvailable(ctx, configuration, end, nil); err != nil || actual != 19 {
		t.Fatal("count reused stale journal availability", actual, err)
	}
	if err := os.WriteFile(filepath.Join(configuration.ProjectRoot, "data", "push_state", "unselected.changes.json"), []byte("{broken"), 0600); err != nil {
		t.Fatal(err)
	}
	if actual, err := CountAvailable(ctx, configuration, end, []string{"alpha.sqlite"}); err == nil || actual != 0 {
		t.Fatal("selection bypassed malformed publication admission", actual, err)
	}
}

// BenchmarkCountAvailable measures complete file admission and live lookup with duplicate large publications.
func BenchmarkCountAvailable(b *testing.B) {
	for _, count := range []int{10000, 100000} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			configuration, end := availableCountFixture(b, count)
			b.ReportAllocs()
			for b.Loop() {
				actual, err := CountAvailable(context.Background(), configuration, end, nil)
				if err != nil || actual != 2*(count-count/10) {
					b.Fatal("weekly count changed", actual, err)
				}
			}
		})
	}
}
