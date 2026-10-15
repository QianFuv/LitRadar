package query

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

// writeBucketPublication writes independent publication identity, time and overlapping membership.
func writeBucketPublication(t testing.TB, configuration config.Config, file, database, run, generated string, ids []int64) {
	t.Helper()
	directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	body, err := json.Marshal(map[string]any{"db_name": database, "run_id": run, "generated_at": generated, "notifiable_article_ids": ids})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, file+".changes.json"), body, 0600); err != nil {
		t.Fatal(err)
	}
}

// TestWeeklyBucketsRetainPrivateMembership checks cached and uncached single-publication output ownership.
func TestWeeklyBucketsRetainPrivateMembership(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	writeBucketPublication(t, configuration, "single", "alpha", "latest", "2026-10-03T12:00:00Z", []int64{3, 1, 3, 2})
	end, _ := weekly.ParseTimestamp("2026-10-04T00:00:00Z")
	for _, cache := range []*weekly.Cache{nil, {}} {
		for iteration := range 3 {
			buckets, err := loadWeeklyBuckets(configuration, end, cache)
			if err != nil || len(buckets) != 1 {
				t.Fatal("single publication changed", buckets, err)
			}
			bucket := buckets["alpha.sqlite"]
			if bucket == nil || !reflect.DeepEqual(bucket.ids, []int64{3, 1, 2}) || bucket.runId == nil || *bucket.runId != "latest" || bucket.generated.Format(false) != "2026-10-03T12:00:00Z" {
				t.Fatal("publication identity or membership changed", iteration, bucket)
			}
			bucket.ids[0] = 99
			bucket.ids = append(bucket.ids, 100)
			*bucket.runId = "mutated"
		}
		if cache != nil && cache.Stats().ParseAttempts != 1 {
			t.Fatal("ownership check did not exercise cached clones", cache.Stats())
		}
	}
}

// TestWeeklyBucketsMergeInPublicationOrder preserves newest metadata and first-occurrence ID order per database.
func TestWeeklyBucketsMergeInPublicationOrder(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	writeBucketPublication(t, configuration, "latest", "alpha", "latest", "2026-10-03T12:00:00Z", []int64{5, 1, 5})
	writeBucketPublication(t, configuration, "duplicate", "alpha", "latest", "2026-10-03T12:00:00Z", []int64{5, 1})
	writeBucketPublication(t, configuration, "older", "alpha", "older", "2026-10-02T12:00:00Z", []int64{1, 2, 9, 2})
	writeBucketPublication(t, configuration, "oldest", "alpha", "oldest", "2026-10-01T12:00:00Z", []int64{9, 4, 5})
	writeBucketPublication(t, configuration, "other", "beta", "other", "2026-10-03T12:00:00Z", []int64{1, 0, -1, 1})
	end, _ := weekly.ParseTimestamp("2026-10-04T00:00:00Z")
	cache := &weekly.Cache{}
	for iteration := range 3 {
		buckets, err := loadWeeklyBuckets(configuration, end, cache)
		if err != nil || len(buckets) != 2 {
			t.Fatal("overlapping publication set changed", buckets, err)
		}
		first, other := buckets["alpha.sqlite"], buckets["beta.sqlite"]
		if first == nil || other == nil || !reflect.DeepEqual(first.ids, []int64{5, 1, 2, 9, 4}) || !reflect.DeepEqual(other.ids, []int64{1, 0, -1}) || first.runId == nil || *first.runId != "latest" {
			t.Fatal("publication merge order or identity changed", iteration, first, other)
		}
		first.ids[0], other.ids[0] = 99, 100
		*first.runId = "mutated"
	}
}
