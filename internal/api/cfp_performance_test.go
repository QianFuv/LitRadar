package api

import (
	"context"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	assets "github.com/QianFuv/LitRadar/assets/cfp"
	acquisition "github.com/QianFuv/LitRadar/internal/cfp"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/index"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

// TestCfpCatalogRetainsPerJournalCapabilities compares shared catalog projection with individual summaries.
func TestCfpCatalogRetainsPerJournalCapabilities(t *testing.T) {
	registrations := acquisition.Registry()
	entries := []sources.JournalCatalogEntry{{CatalogId: "unadapted", Title: "Unadapted"}}
	snapshots := []storage.JournalSnapshot{}
	seen := map[bool]bool{}
	for _, registration := range registrations {
		canRefresh := registration.CanRefresh()
		if seen[canRefresh] {
			continue
		}
		seen[canRefresh] = true
		entries = append(entries, sources.JournalCatalogEntry{CatalogId: registration.CatalogIds[0], Title: registration.JournalTitle})
		snapshots = append(snapshots, storage.JournalSnapshot{JournalKey: registration.CatalogIds[0], CatalogIds: registration.CatalogIds, CheckedOn: "2026-09-15", Sources: []storage.SourceStatus{{SourceKey: registration.SourceKey, Status: "success", Revision: 2}}})
	}
	if len(seen) != 2 {
		t.Fatal("registry lost automatic or snapshot-only coverage")
	}
	const now = int64(100)
	result, failure := cfpCatalog("fixture.sqlite", nil, entries, snapshots, now)
	if failure != nil || len(result.Items) != 3 || result.Summary.AdaptedJournals != 2 {
		t.Fatal(result, failure)
	}
	for _, entry := range entries {
		snapshot, failure := cfpFindSnapshot(entry, snapshots)
		if failure != nil {
			t.Fatal(failure)
		}
		expected := cfpSummary(entry, snapshot, now, now)
		hasItem := false
		for _, item := range result.Items {
			if item.CatalogId == entry.CatalogId {
				hasItem = true
				if !reflect.DeepEqual(item, expected) {
					t.Fatal("shared registry changed journal capability or freshness", item, expected)
				}
			}
		}
		if !hasItem {
			t.Fatal("catalog lost a journal", entry.CatalogId)
		}
	}
	query := "Unadapted"
	filtered, failure := cfpCatalog("fixture.sqlite", &query, entries, snapshots, now)
	if failure != nil || len(filtered.Items) != 1 || filtered.Items[0].CatalogId != "unadapted" || filtered.Summary != result.Summary {
		t.Fatal("search changed global totals or returned unrelated journals", filtered, failure)
	}
}

// cfpBenchmarkCatalog loads the maintained English catalog and actual reviewed seed outside measured work.
func cfpBenchmarkCatalog(t testing.TB) ([]sources.JournalCatalogEntry, []storage.JournalSnapshot) {
	t.Helper()
	entries, err := index.ReadCatalogCsv(filepath.Join("..", "..", "assets", "meta", "english_journals.csv"))
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(ctx, filename); err != nil {
		t.Fatal(err)
	}
	repository, err := storage.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	if _, err := repository.ImportSeed(ctx, assets.SeedId, assets.Seed()); err != nil {
		t.Fatal(err)
	}
	snapshots, err := repository.LoadJournals(ctx)
	if err != nil {
		t.Fatal(err)
	}
	return entries, snapshots
}

// BenchmarkCfpCatalog measures complete catalog projection with real adapted and unadapted journals.
func BenchmarkCfpCatalog(b *testing.B) {
	entries, snapshots := cfpBenchmarkCatalog(b)
	now := time.Date(2026, 10, 16, 0, 0, 0, 0, time.UTC).Unix()
	for _, scenario := range []struct {
		name      string
		query     *string
		snapshots []storage.JournalSnapshot
	}{
		{"all", nil, snapshots},
		{"english-search", new("journal"), snapshots},
		{"unmatched-search", new("no-such-reviewed-cfp-title"), snapshots},
		{"unadapted", nil, nil},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			var result *cfpCatalogResponse
			b.ReportAllocs()
			for b.Loop() {
				var failure *apiError
				result, failure = cfpCatalog("english_journals.sqlite", scenario.query, entries, scenario.snapshots, now)
				if failure != nil {
					b.Fatal(failure)
				}
			}
			b.StopTimer()
			if result.Summary.Journals != len(entries) || scenario.name == "unmatched-search" && len(result.Items) != 0 || scenario.name == "all" && result.Summary.AdaptedJournals == 0 {
				b.Fatal("maintained catalog projection changed", result.Summary, len(result.Items))
			}
		})
	}
}
