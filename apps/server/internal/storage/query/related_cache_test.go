package query

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// TestRelatedCostCacheTracksDatabaseGeneration checks replacement even when size and
// timestamps are preserved, and bypasses uncheckpointed changes in SQLite journals.
func TestRelatedCostCacheTracksDatabaseGeneration(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "index.sqlite")
	if err := os.WriteFile(filename, []byte("first"), 0600); err != nil {
		t.Fatal(err)
	}
	first := relatedCostsForFile(filename)
	first.put("term", 42)
	if cost, found := relatedCostsForFile(filename).get("term"); !found || cost != 42 {
		t.Fatal("unchanged index did not reuse exact cost", cost, found)
	}
	info, err := os.Stat(filename)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Rename(filename, filename+".old"); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, []byte("other"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(filename, info.ModTime(), info.ModTime()); err != nil {
		t.Fatal(err)
	}
	if _, found := relatedCostsForFile(filename).get("term"); found {
		t.Fatal("replacement index reused stale costs")
	}
	for _, suffix := range []string{"-wal", "-journal"} {
		cache := relatedCostsForFile(filename)
		cache.put("term", 12)
		if err := os.WriteFile(filename+suffix, []byte("uncheckpointed"), 0600); err != nil {
			t.Fatal(err)
		}
		if relatedCostsForFile(filename) != nil {
			t.Fatal("journal content must bypass the process cache", suffix)
		}
		if err := os.Remove(filename + suffix); err != nil {
			t.Fatal(err)
		}
		if _, found := relatedCostsForFile(filename).get("term"); found {
			t.Fatal("journal invalidation retained old costs", suffix)
		}
	}
}

// TestRelatedParallelCountsMatchSingleConnection preserves phrase costs and tie order
// for Latin words, mixed alphanumeric tokens, and position-sensitive Han phrases.
func TestRelatedParallelCountsMatchSingleConnection(t *testing.T) {
	configuration, single := queryFixture(t)
	if _, err := single.Exec(`INSERT INTO articles(article_id,journal_id,title,abstract_text,authors_json,date) VALUES
 (4001,1,'Graph neural 数字经济 si7','数字经济 graph si7','[]','2024-01-01'),
 (4002,1,'Graph 数字经济 si7','neural networks','[]','2024-01-01'),
 (4003,1,'数字 经济 neural si7','graph','[]','2024-01-01');`); err != nil {
		t.Fatal(err)
	}
	rebuildQueryFixtureSearch(t, single)
	filename, err := configuration.ResolveIndexDBPath(nil)
	if err != nil {
		t.Fatal(err)
	}
	parallel, err := storage.Open(filename, false, relatedCountWorkers)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := parallel.Close(); err != nil {
			t.Error(err)
		}
	}()
	terms := relatedTerms("Graph neural 数字经济 si7")
	var expected []int
	for _, database := range []*sql.DB{single, parallel} {
		connection, err := database.Conn(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		costs, err := relatedTermCosts(t.Context(), database, connection, nil, terms, false)
		if closeError := connection.Close(); closeError != nil {
			t.Error(closeError)
		}
		if err != nil {
			t.Fatal(err)
		}
		if expected == nil {
			expected = costs
		} else if !reflect.DeepEqual(costs, expected) {
			t.Fatal("parallel costs changed phrase semantics", expected, costs)
		}
	}
	first, err := relatedArticleIds(context.Background(), single, filename, "Graph neural 数字经济 si7", 4001, 30)
	if err != nil {
		t.Fatal(err)
	}
	second, err := relatedArticleIds(context.Background(), parallel, filename, "Graph neural 数字经济 si7", 4001, 30)
	if err != nil || !reflect.DeepEqual(first, second) {
		t.Fatal("parallel ranking changed ordered IDs", first, second, err)
	}
}
