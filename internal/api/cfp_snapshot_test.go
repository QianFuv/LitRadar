package api

import (
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
)

// TestCfpSnapshotIndexRetainsAliasOwnership compares exact pointer identity and ambiguity with scanning.
func TestCfpSnapshotIndexRetainsAliasOwnership(t *testing.T) {
	for _, scenario := range []struct {
		name      string
		entry     sources.JournalCatalogEntry
		snapshots []storage.JournalSnapshot
	}{
		{"empty", sources.JournalCatalogEntry{CatalogId: "a"}, nil},
		{"canonical", sources.JournalCatalogEntry{CatalogId: "a"}, []storage.JournalSnapshot{{CatalogIds: []string{"a"}}}},
		{"aliases", sources.JournalCatalogEntry{CatalogId: "a", CatalogAliases: []string{"alias", "other", "alias"}}, []storage.JournalSnapshot{{CatalogIds: []string{"alias", "other", "alias"}}}},
		{"shared-alias", sources.JournalCatalogEntry{CatalogId: "a"}, []storage.JournalSnapshot{{CatalogIds: []string{"a"}}, {CatalogIds: []string{"a"}}}},
		{"different-alias-owners", sources.JournalCatalogEntry{CatalogId: "a", CatalogAliases: []string{"b"}}, []storage.JournalSnapshot{{CatalogIds: []string{"a"}}, {CatalogIds: []string{"b"}}}},
		{"unrelated-conflict", sources.JournalCatalogEntry{CatalogId: "a"}, []storage.JournalSnapshot{{CatalogIds: []string{"a"}}, {CatalogIds: []string{"x"}}, {CatalogIds: []string{"x"}}}},
		{"literal-case", sources.JournalCatalogEntry{CatalogId: "Alias"}, []storage.JournalSnapshot{{CatalogIds: []string{"alias"}}}},
		{"journal-key-only", sources.JournalCatalogEntry{CatalogId: "a"}, []storage.JournalSnapshot{{JournalKey: "a", CatalogIds: []string{"b"}}}},
		{"blank-alias", sources.JournalCatalogEntry{CatalogId: ""}, []storage.JournalSnapshot{{CatalogIds: []string{""}}}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			expected, expectedFailure := cfpFindSnapshot(scenario.entry, scenario.snapshots)
			actual, failure := cfpFindIndexedSnapshot(scenario.entry, cfpIndexSnapshots(scenario.snapshots))
			if actual != expected || (failure == nil) != (expectedFailure == nil) || failure != nil && failure.status != 500 {
				t.Fatal("indexed snapshot ownership changed", actual, expected, failure, expectedFailure)
			}
		})
	}
	query := "never matches"
	snapshots := []storage.JournalSnapshot{{CatalogIds: []string{"a"}}, {CatalogIds: []string{"a"}}}
	if result, failure := cfpCatalog("fixture.sqlite", &query, []sources.JournalCatalogEntry{{CatalogId: "a", Title: "Journal"}}, snapshots, 100); result != nil || failure == nil || failure.status != 500 {
		t.Fatal("search filtering bypassed snapshot ambiguity", result, failure)
	}
}

// FuzzCfpSnapshotIndex compares indexed ownership with the original ordered scan for arbitrary alias collisions.
func FuzzCfpSnapshotIndex(f *testing.F) {
	for _, data := range [][]byte{{}, {0}, {1, 2, 3, 4, 5, 6}, {5, 3, 2, 2, 2, 1, 2, 2, 2}} {
		f.Add(data)
	}
	f.Fuzz(func(t *testing.T, data []byte) {
		if len(data) > 256 {
			t.Skip()
		}
		position := 0
		next := func() byte {
			if position == len(data) {
				return 0
			}
			value := data[position]
			position++
			return value
		}
		words := []string{"", "a", "b", "Alias", "alias", "中文", " a "}
		word := func() string { return words[int(next())%len(words)] }
		entry := sources.JournalCatalogEntry{CatalogId: word()}
		for range int(next() % 4) {
			entry.CatalogAliases = append(entry.CatalogAliases, word())
		}
		snapshots := make([]storage.JournalSnapshot, int(next()%6))
		for index := range snapshots {
			snapshots[index].JournalKey = word()
			for range int(next() % 5) {
				snapshots[index].CatalogIds = append(snapshots[index].CatalogIds, word())
			}
		}
		expected, expectedFailure := cfpFindSnapshot(entry, snapshots)
		actual, failure := cfpFindIndexedSnapshot(entry, cfpIndexSnapshots(snapshots))
		if actual != expected || (failure == nil) != (expectedFailure == nil) {
			t.Fatal("indexed snapshot ownership changed", entry, snapshots, failure, expectedFailure)
		}
	})
}
