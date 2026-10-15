package api

import (
	"slices"
	"testing"

	acquisition "github.com/QianFuv/LitRadar/internal/cfp"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
)

// TestCfpRefreshLookupPreservesFirstRegistration keeps false capabilities authoritative and aliases literal.
func TestCfpRefreshLookupPreservesFirstRegistration(t *testing.T) {
	registrations := []acquisition.SourceConfig{
		{CatalogIds: []string{"shared", "snapshot"}, Adapter: acquisition.SnapshotOnly},
		{CatalogIds: []string{"shared", "automatic", "Alias", ""}, Adapter: acquisition.SpringerCollections},
	}
	lookup := cfpRefreshLookup(registrations)
	registrations[0].Adapter = acquisition.SpringerCollections
	registrations[1].CatalogIds[1] = "mutated"
	for journalKey, expected := range map[string]bool{
		"shared": false, "snapshot": false, "automatic": true, "Alias": true,
		"alias": false, "": true, "missing": false, "mutated": false,
	} {
		if lookup(journalKey) != expected {
			t.Fatal("capability or registry ownership changed", journalKey, expected)
		}
	}
}

// TestCfpPackagedCapabilitiesMatchRegistry compares every bundled alias with the original ordered scan.
func TestCfpPackagedCapabilitiesMatchRegistry(t *testing.T) {
	registrations := acquisition.Registry()
	lookup := cfpRefreshCapability()
	for _, registration := range registrations {
		for _, alias := range registration.CatalogIds {
			expected := false
			for _, candidate := range registrations {
				if slices.Contains(candidate.CatalogIds, alias) {
					expected = candidate.CanRefresh()
					break
				}
			}
			if lookup(alias) != expected {
				t.Fatal("packaged capability differs from registry", alias)
			}
		}
	}
	if lookup("unregistered-cfp-journal") {
		t.Fatal("unknown journal became refreshable")
	}
}

// TestCfpCapabilityReuseKeepsSnapshotStatusLive prevents fixed capability reuse from freezing mutable refresh data.
func TestCfpCapabilityReuseKeepsSnapshotStatusLive(t *testing.T) {
	registration := acquisition.Registry()[0]
	entry := sources.JournalCatalogEntry{CatalogId: registration.CatalogIds[0]}
	lease := int64(100)
	snapshot := storage.JournalSnapshot{JournalKey: entry.CatalogId, Sources: []storage.SourceStatus{{Status: "refreshing", LeaseExpiresAt: &lease}}}
	first := cfpSummary(entry, &snapshot, 100, 100)
	expired := cfpSummary(entry, &snapshot, 100, 101)
	if first.RefreshStatus != "refreshing" || expired.RefreshStatus != "failed" || expired.LastError == nil {
		t.Fatal("capability reuse froze the lease clock", first, expired)
	}
	snapshot.Sources[0].Status = "success"
	snapshot.JournalKey = "unregistered-cfp-journal"
	changed := cfpSummary(entry, &snapshot, 100, 102)
	if changed.RefreshStatus != "success" || changed.CanRefresh || changed.LastError != nil {
		t.Fatal("capability reuse froze snapshot contents", changed)
	}
}

// BenchmarkCfpRefreshCapabilityInitialization includes the one-time bundled decoding and validation cost.
func BenchmarkCfpRefreshCapabilityInitialization(b *testing.B) {
	b.ReportAllocs()
	for b.Loop() {
		lookup := cfpRefreshLookup(acquisition.Registry())
		if lookup("unregistered-cfp-journal") {
			b.Fatal("unknown journal became refreshable")
		}
	}
}
