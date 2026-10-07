package cfp

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"reflect"
	"sync"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

// preparationSource supplies a valid literal notice without normalizing its catalog or title identity.
func preparationSource(ids []string, title string) domain.Source {
	return domain.Source{CatalogIds: ids, JournalTitle: "Journal", Title: title, TypeText: "Special Issue", DateText: "Submission deadline: 31 December 2026", SourceUrl: "https://example.org/cfp", CheckedOn: "2026-09-15"}
}

// TestGenerationFenceSelectsWinningPayload protects last-good content from stale concurrent publications.
func TestGenerationFenceSelectsWinningPayload(t *testing.T) {
	ctx := context.Background()
	repository := preparationRepository(t)
	source := preparationSource([]string{"a"}, "Topic")
	importPreparationSource(t, repository, source)
	leases := make([]RefreshLease, 2)
	for index := range leases {
		lease, err := repository.BeginRefresh(ctx, "journal:a", 100, 20)
		if err != nil {
			t.Fatal(err)
		}
		if lease.SourceKey != "journal:a" || lease.ExpiresAt != 120 {
			t.Fatal("lease metadata changed", lease)
		}
		leases[index] = lease
	}
	failures := make([]error, 2)
	var workers sync.WaitGroup
	for index := range leases {
		workers.Go(func() {
			candidate := source
			candidate.Scope = []string{"stale original", "winning original"}[index]
			failures[index] = repository.PublishRefresh(ctx, leases[index], Publication{Capture: candidate.Scope, CaptureFormat: "html", SourceUrl: candidate.SourceUrl, Sources: []domain.Source{candidate}, ConfigVersion: 1}, 105)
		})
	}
	workers.Wait()
	if !errors.Is(failures[0], ErrStaleRefresh) || failures[1] != nil {
		t.Fatal("generation fence changed", failures)
	}
	assertWinningPreparationSnapshot(t, repository)
}

// assertWinningPreparationSnapshot requires the winning original and its matching publication revision.
func assertWinningPreparationSnapshot(t *testing.T, repository *Repository) {
	t.Helper()
	journals, err := repository.LoadJournals(context.Background())
	if err != nil || journals[0].Notices[0].Scope != "winning original" || journals[0].Sources[0].Revision != 2 {
		t.Fatal("stale content published", journals, err)
	}
}

// preparationRepository owns a migrated temporary database for publication characterization.
func preparationRepository(t *testing.T) *Repository {
	t.Helper()
	path := filepath.Join(t.TempDir(), "state.sqlite")
	if _, err := migration.Migrate(context.Background(), path); err != nil {
		t.Fatal(err)
	}
	repository, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	return repository
}

// importPreparationSource seeds one original before exercising generation-fenced publication.
func importPreparationSource(t *testing.T, repository *Repository, source domain.Source) {
	t.Helper()
	encoded, err := json.Marshal(domain.Seed{FormatVersion: 1, Sources: []domain.Source{source}})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.ImportSeed(context.Background(), "fixture", encoded); err != nil {
		t.Fatal(err)
	}
}

// TestPreparationFailureOrder protects populated-source rejection before empty statements and reviewed counts.
func TestPreparationFailureOrder(t *testing.T) {
	source := preparationSource(nil, "Invalid")
	seed := domain.Seed{FormatVersion: 1, Sources: []domain.Source{source}, EmptyJournals: []domain.EmptyJournal{{}}, ExpectedJournals: new(uint64(99))}
	prepared, err := prepare(seed)
	if err == nil || err.Error() != "Invalid CFP source: Journal / Invalid" || prepared != nil {
		t.Fatal("source error order changed", prepared, err)
	}
	seed.FormatVersion = 2
	if _, err := prepare(seed); err == nil || err.Error() != "Unsupported CFP input format" {
		t.Fatal("format error order changed", err)
	}
}

// TestPreparationRetainsFirstJournalAndLiteralAliases protects grouping order and non-normalized ownership.
func TestPreparationRetainsFirstJournalAndLiteralAliases(t *testing.T) {
	first := preparationSource([]string{"a", " alias ", "alias"}, "First")
	second := preparationSource([]string{"a", "alias"}, "Second")
	second.JournalTitle, second.CheckedOn = "Different Journal", "2026-10-07"
	prepared, err := prepare(domain.Seed{FormatVersion: 1, Sources: []domain.Source{first, second}})
	if err != nil {
		t.Fatal(err)
	}
	journal := prepared["a"]
	if journal.title != "Journal" || journal.checkedOn != "2026-10-07" || !reflect.DeepEqual(journal.aliases, []string{" alias ", "a", "alias"}) || journal.notices[0].source.Title != "First" || journal.notices[1].source.Title != "Second" {
		t.Fatal("literal grouping changed", journal)
	}
}

// TestPreparationAliasConflictPrecedesCounts protects sorted-owner conflict priority over count mismatch.
func TestPreparationAliasConflictPrecedesCounts(t *testing.T) {
	first := preparationSource([]string{"a", "z"}, "First")
	second := preparationSource([]string{"b", "z"}, "Second")
	prepared, err := prepare(domain.Seed{FormatVersion: 1, Sources: []domain.Source{second, first}, ExpectedNotices: new(uint64(99))})
	if err == nil || err.Error() != "Conflicting CFP alias ownership: z" || prepared != nil {
		t.Fatal("alias conflict order changed", prepared, err)
	}
}

// TestPreparationDuplicateCleanTitleAndEmptyStatements protects independent duplicate and empty conflict rules.
func TestPreparationDuplicateCleanTitleAndEmptyStatements(t *testing.T) {
	first := preparationSource([]string{"a"}, "Topic")
	second := preparationSource([]string{"a"}, " Topic ")
	if _, err := prepare(domain.Seed{FormatVersion: 1, Sources: []domain.Source{first, second}}); err == nil || err.Error() != "Duplicate CFP notice within a" {
		t.Fatal("clean title duplicate admitted", err)
	}
	empty := domain.EmptyJournal{CatalogIds: []string{"a"}, JournalTitle: "Journal", CheckedOn: "2026-10-07", SourceUrl: "https://example.org/cfp", SourceStatement: "No open calls"}
	if _, err := prepare(domain.Seed{FormatVersion: 1, EmptyJournals: []domain.EmptyJournal{empty, empty}}); err == nil || err.Error() != "A CFP journal cannot be both empty and populated in one source snapshot" {
		t.Fatal("duplicate empty statement admitted", err)
	}
	if _, err := prepare(domain.Seed{FormatVersion: 1, ExpectedJournals: new(uint64(0)), ExpectedNotices: new(uint64(0))}); err != nil {
		t.Fatal("reviewed empty counts rejected", err)
	}
}

// TestVerifiedEmptyDateGrammar protects signed years, calendar round trips and asymmetric whitespace.
func TestVerifiedEmptyDateGrammar(t *testing.T) {
	for _, scenario := range []struct {
		value    string
		expected bool
	}{
		{"0000-2-29", true}, {"+10000-1-1", true}, {"10000-1-1", false}, {"-262143-1-1", true}, {"+262142-12-31", true}, {"-262144-1-1", false}, {"+262143-1-1", false}, {"2024-2-29", true}, {"2026-2-29", false}, {"2026-2-31", false}, {" 2026- 1- 1", true}, {"2026 -1-1", false}, {"2026-1-1 ", false}, {"2026-01-001", false},
	} {
		if actual := validEmptyDate(scenario.value); actual != scenario.expected {
			t.Fatal(scenario, actual)
		}
	}
}
