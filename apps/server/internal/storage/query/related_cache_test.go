package query

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	native "modernc.org/sqlite"
)

// TestRelatedCostCacheTracksDatabaseGeneration checks replacement even when size and
// timestamps are preserved, and bypasses uncheckpointed changes in SQLite journals.
func TestRelatedCostCacheTracksDatabaseGeneration(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "index.sqlite")
	if err := os.WriteFile(filename, []byte("first"), 0600); err != nil {
		t.Fatal(err)
	}
	first := relatedCostsForFile(filename)
	first.put("term", relatedRecallBudget, 42)
	key := relatedResultKey{title: "title", id: 1, limit: 30}
	first.putResult(key, []int64{2, 3})
	if cost, found := relatedCostsForFile(filename).get("term", relatedRecallBudget); !found || cost != 42 {
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
	if _, found := relatedCostsForFile(filename).get("term", relatedRecallBudget); found {
		t.Fatal("replacement index reused stale costs")
	}
	if _, found := relatedCostsForFile(filename).getResult(key); found {
		t.Fatal("replacement index reused stale candidates")
	}
	for _, suffix := range []string{"-wal", "-journal"} {
		cache := relatedCostsForFile(filename)
		cache.put("term", relatedRecallBudget, 12)
		cache.putResult(key, []int64{2, 3})
		if err := os.WriteFile(filename+suffix, []byte("uncheckpointed"), 0600); err != nil {
			t.Fatal(err)
		}
		if relatedCostsForFile(filename) != nil {
			t.Fatal("journal content must bypass the process cache", suffix)
		}
		if err := os.Remove(filename + suffix); err != nil {
			t.Fatal(err)
		}
		if _, found := relatedCostsForFile(filename).get("term", relatedRecallBudget); found {
			t.Fatal("journal invalidation retained old costs", suffix)
		}
		if _, found := relatedCostsForFile(filename).getResult(key); found {
			t.Fatal("journal invalidation retained old candidates", suffix)
		}
	}
}

// TestRelatedCandidateCacheOwnsIDsAndInputs protects rankings from aliases and key collisions.
func TestRelatedCandidateCacheOwnsIDsAndInputs(t *testing.T) {
	cache := &relatedDatabaseCosts{results: map[relatedResultKey][]int64{}}
	key := relatedResultKey{title: "graph", id: 1, limit: 30}
	ids := []int64{2, 3}
	cache.putResult(key, ids)
	ids[0] = 99
	first, found := cache.getResult(key)
	if !found || !reflect.DeepEqual(first, []int64{2, 3}) {
		t.Fatal("caller input altered cached candidates", first)
	}
	first[0] = 88
	second, _ := cache.getResult(key)
	if !reflect.DeepEqual(second, []int64{2, 3}) {
		t.Fatal("caller output altered cached candidates", second)
	}
	for _, other := range []relatedResultKey{{title: "changed", id: 1, limit: 30}, {title: "graph", id: 2, limit: 30}, {title: "graph", id: 1, limit: 60}} {
		if _, found := cache.getResult(other); found {
			t.Fatal("different ranking input reused cached candidates", other)
		}
	}
}

// TestRelatedCandidateCacheAvoidsRanking verifies the warm path without permitting FTS reads.
func TestRelatedCandidateCacheAvoidsRanking(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec(`INSERT INTO article_search(rowid,title,abstract_text) VALUES (8001,'cacheable graph',''),(8002,'cacheable neural graph',''); PRAGMA wal_checkpoint(TRUNCATE)`); err != nil {
		t.Fatal(err)
	}
	filename, err := configuration.ResolveIndexDBPath(nil)
	if err != nil {
		t.Fatal(err)
	}
	first, err := relatedArticleIds(t.Context(), database, filename, "cacheable graph", 8001, 30)
	if err != nil || !reflect.DeepEqual(first, []int64{8002}) {
		t.Fatal("initial ranking failed", first, err)
	}
	connection, err := database.Conn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if err := connection.Raw(func(raw any) error {
		return raw.(native.AuthorizerRegisterer).RegisterAuthorizer(func(operation native.AuthorizerActionCode, table, column, database, trigger string) native.AuthorizerReturnCode {
			if operation == native.AuthRead && strings.HasPrefix(table, "article_search") {
				return native.AuthorizerDeny
			}
			return native.AuthorizerOK
		})
	}); err != nil {
		t.Fatal(err)
	}
	if err := connection.Close(); err != nil {
		t.Fatal(err)
	}
	second, err := relatedArticleIds(t.Context(), database, filename, "cacheable graph", 8001, 30)
	if err != nil || !reflect.DeepEqual(first, second) {
		t.Fatal("warm cache attempted FTS ranking or changed candidates", second, err)
	}
	if _, err := relatedArticleIds(t.Context(), database, filename, "cacheable graph", 8001, 60); err == nil {
		t.Fatal("changed candidate limit must require a new ranking")
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if _, err := relatedArticleIds(ctx, database, filename, "cacheable graph", 8001, 30); !errors.Is(err, context.Canceled) {
		t.Fatal("cache hit ignored canceled request", err)
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
