package weekly

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

func writeManifest(t *testing.T, filename string, ids []int64) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(map[string]any{"db_name": "catalog.sqlite", "generated_at": "2026-10-03T12:00:00.123456789Z", "run_id": "source", "notifiable_article_ids": ids})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, data, 0600); err != nil {
		t.Fatal(err)
	}
}

func TestCacheArticleBudgetEvictsAndDoesNotRetainOversizedPublication(t *testing.T) {
	directory := t.TempDir()
	ids := make([]int64, cacheArticleLimit+1)
	for index := range ids {
		ids[index] = int64(index + 1)
	}
	first := filepath.Join(directory, "first.changes.json")
	second := filepath.Join(directory, "second.changes.json")
	large := filepath.Join(directory, "large.changes.json")
	writeManifest(t, first, ids[:600000])
	writeManifest(t, second, ids[:600000])
	writeManifest(t, large, ids)
	cache := Cache{}
	now := time.Now()
	for _, filename := range []string{first, second} {
		if _, err := cache.readAt(filename, now); err != nil {
			t.Fatal(err)
		}
	}
	if stats := cache.statsAt(now); stats.Entries != 1 || stats.ArticleIds != 600000 {
		t.Fatal(stats)
	}
	assertOversizedPublicationUncached(t, &cache, large, now)
}

func TestCacheExpiresFromLoadTimeAndReturnsIndependentCopies(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "catalog.changes.json")
	writeManifest(t, filename, []int64{1, 2})
	cache := Cache{}
	now := time.Now()
	first, err := cache.readAt(filename, now)
	if err != nil {
		t.Fatal(err)
	}
	first.ArticleIds[0] = 999
	*first.RunId = "mutated"
	second, err := cache.readAt(filename, now.Add(59*time.Second))
	if err != nil || second.ArticleIds[0] != 1 || *second.RunId != "source" {
		t.Fatalf("cached data mutated: %+v %v", second, err)
	}
	if stats := cache.statsAt(now.Add(59 * time.Second)); stats.ParseAttempts != 1 || stats.Entries != 1 || stats.ArticleIds != 2 {
		t.Fatalf("warm stats=%+v", stats)
	}
	if _, err := cache.readAt(filename, now.Add(60*time.Second)); err != nil {
		t.Fatal(err)
	}
	if stats := cache.statsAt(now.Add(60 * time.Second)); stats.ParseAttempts != 2 {
		t.Fatalf("cache hit extended TTL: %+v", stats)
	}
}

func TestCacheNeverMasksDamageAndRecoversAfterRepair(t *testing.T) {
	directory := t.TempDir()
	filename := filepath.Join(directory, "catalog.changes.json")
	other := filepath.Join(directory, "other.changes.json")
	writeManifest(t, filename, []int64{1})
	writeManifest(t, other, []int64{2})
	cache := Cache{}
	if _, err := cache.read(filename); err != nil {
		t.Fatal(err)
	}
	if _, err := cache.read(other); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, []byte("{broken"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := cache.read(filename); err == nil {
		t.Fatal("returned stale publication after corruption")
	}
	if stats := cache.Stats(); stats.Entries != 1 || stats.ArticleIds != 1 || stats.ParseAttempts != 3 {
		t.Fatalf("failure cached: %+v", stats)
	}
	assertCacheRepairAndMissingSource(t, &cache, filename)
}

func TestCacheBoundsLruAndRetainsValidIgnoredPublications(t *testing.T) {
	directory := t.TempDir()
	cache := Cache{}
	now := time.Now()
	fillPublicationCache(t, &cache, directory, now)
	first := filepath.Join(directory, "0.changes.json")
	if _, err := cache.readAt(first, now); err != nil {
		t.Fatal(err)
	}
	extra := filepath.Join(directory, "extra.changes.json")
	writeManifest(t, extra, []int64{})
	if value, err := cache.readAt(extra, now); err != nil || value != nil {
		t.Fatalf("empty publication=%+v %v", value, err)
	}
	if stats := cache.statsAt(now); stats.Entries != 64 || stats.ArticleIds != 63 {
		t.Fatalf("bounded stats=%+v", stats)
	}
	if _, err := cache.readAt(first, now); err != nil {
		t.Fatal(err)
	}
	if _, err := cache.readAt(extra, now); err != nil {
		t.Fatal(err)
	}
	if stats := cache.statsAt(now); stats.ParseAttempts != 65 {
		t.Fatalf("recent/empty entry evicted: %+v", stats)
	}
	assertOldestPublicationEvicted(t, &cache, directory, now)
}

func TestDiscoveryAndPublicationDedupUseImmutableSourceIdentity(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
	latest := filepath.Join(directory, "catalog.changes.json")
	writeManifest(t, latest, []int64{1, 2, 1})
	writeManifest(t, filepath.Join(directory, "history", "catalog", strings.Repeat("a", 64)+".changes.json"), []int64{1, 2})
	writeManifest(t, filepath.Join(directory, "history", "catalog", strings.Repeat("b", 64)+".changes.json"), []int64{2, 1})
	for _, filename := range []string{filepath.Join(directory, "ignored.json"), filepath.Join(directory, "history", "catalog", strings.Repeat("C", 64)+".changes.json"), filepath.Join(directory, "history", "catalog", "bad.changes.json")} {
		if err := os.WriteFile(filename, []byte("{broken"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	end, _ := ParseTimestamp("2026-10-03T12:00:00.123456789Z")
	cache := Cache{}
	assertImmutablePublicationDedup(t, configuration, &cache, end)
	before := end
	before.Nanoseconds--
	if manifests, err := LoadManifests(configuration, before.WindowStart(), before, &cache); err != nil || len(manifests) != 0 {
		t.Fatalf("nanosecond boundary=%+v %v", manifests, err)
	}
	if err := os.WriteFile(filepath.Join(directory, "unrelated.changes.json"), []byte("{broken"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := LoadManifests(configuration, end.WindowStart(), end, &cache); err == nil {
		t.Fatal("unrelated malformed publication swallowed")
	}
}

func TestConcurrentCacheReadersCannotMutateSharedPublication(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "catalog.changes.json")
	writeManifest(t, filename, []int64{1, 2, 3})
	cache := Cache{}
	var workers sync.WaitGroup
	for range 8 {
		workers.Go(func() {
			for range 10 {
				value, err := cache.read(filename)
				if err != nil {
					t.Error(err)
					return
				}
				if value.ArticleIds[0] != 1 {
					t.Error("shared slice mutated")
				}
				value.ArticleIds[0] = 999
			}
		})
	}
	workers.Wait()
	if stats := cache.Stats(); stats.Entries != 1 || stats.ArticleIds != 3 {
		t.Fatalf("concurrent accounting=%+v", stats)
	}
}

// assertOversizedPublicationUncached checks repeated untruncated reads without replacing retained publications.
func assertOversizedPublicationUncached(t *testing.T, cache *Cache, large string, now time.Time) {
	t.Helper()
	for range 2 {
		manifest, err := cache.readAt(large, now)
		if err != nil || len(manifest.ArticleIds) != cacheArticleLimit+1 {
			t.Fatalf("oversized publication was truncated: %v", err)
		}
	}
	if stats := cache.statsAt(now); stats.Entries != 1 || stats.ArticleIds != 600000 || stats.ParseAttempts != 4 {
		t.Fatal(stats)
	}
}

// assertCacheRepairAndMissingSource checks repair followed by global clearing after failed canonicalization.
func assertCacheRepairAndMissingSource(t *testing.T, cache *Cache, filename string) {
	t.Helper()
	writeManifest(t, filename, []int64{3})
	if value, err := cache.read(filename); err != nil || value.ArticleIds[0] != 3 {
		t.Fatalf("repair=%+v %v", value, err)
	}
	if err := os.Remove(filename); err != nil {
		t.Fatal(err)
	}
	if _, err := cache.read(filename); err == nil {
		t.Fatal("missing manifest accepted")
	}
	if stats := cache.Stats(); stats.Entries != 0 || stats.ArticleIds != 0 {
		t.Fatalf("canonicalization failure did not clear cache: %+v", stats)
	}
}

// fillPublicationCache loads all entries before testing access chronology.
func fillPublicationCache(t *testing.T, cache *Cache, directory string, now time.Time) {
	t.Helper()
	for index := range cacheCapacity {
		filename := filepath.Join(directory, strconv.Itoa(index)+".changes.json")
		writeManifest(t, filename, []int64{int64(index)})
		if _, err := cache.readAt(filename, now); err != nil {
			t.Fatal(err)
		}
	}
}

// assertOldestPublicationEvicted checks the original LRU victim after warm and ignored-publication hits.
func assertOldestPublicationEvicted(t *testing.T, cache *Cache, directory string, now time.Time) {
	t.Helper()
	if _, err := cache.readAt(filepath.Join(directory, "1.changes.json"), now); err != nil {
		t.Fatal(err)
	}
	if stats := cache.statsAt(now); stats.ParseAttempts != 66 {
		t.Fatalf("oldest entry retained: %+v", stats)
	}
}

// assertImmutablePublicationDedup checks exact source order under both cold and cached discovery.
func assertImmutablePublicationDedup(t *testing.T, configuration config.Config, cache *Cache, end Timestamp) {
	t.Helper()
	for _, selectedCache := range []*Cache{nil, cache} {
		manifests, err := LoadManifests(configuration, end.WindowStart(), end, selectedCache)
		if err != nil {
			t.Fatal(err)
		}
		if len(manifests) != 2 || !reflect.DeepEqual(manifests[0].ArticleIds, []int64{1, 2}) || !reflect.DeepEqual(manifests[1].ArticleIds, []int64{2, 1}) {
			t.Fatalf("dedup/order=%+v", manifests)
		}
	}
}
