package cfp

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"sync"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func TestConcurrentSeedAndGenerationFencing(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "state.sqlite")
	if _, err := migration.Migrate(ctx, path); err != nil {
		t.Fatal(err)
	}
	source := domain.Source{CatalogIds: []string{"a"}, JournalTitle: "Original Journal", Title: "Original topic", TypeText: "Special Issue", DateText: "Submission deadline: 31 December 2026", SourceUrl: "https://example.org/cfp", CheckedOn: "2026-09-15"}
	payload := concurrentSeedPayload(t, source)
	repositories := []*Repository{}
	for index := 0; index < 8; index++ {
		repository, err := Open(path)
		if err != nil {
			t.Fatal(err)
		}
		defer repository.Close()
		repositories = append(repositories, repository)
	}
	var workers sync.WaitGroup
	results := make([]ImportResult, len(repositories))
	failures := make([]error, len(repositories))
	for index, repository := range repositories {
		workers.Go(func() { results[index], failures[index] = repository.ImportSeed(ctx, "fixture", payload) })
	}
	workers.Wait()
	assertSingleConcurrentImport(t, results, failures)
	leases := make([]RefreshLease, len(repositories))
	for index, repository := range repositories {
		workers.Go(func() { leases[index], failures[index] = repository.BeginRefresh(ctx, "journal:a", 100, 20) })
	}
	workers.Wait()
	assertUniqueConcurrentGenerations(t, leases, failures)
	publication := Publication{Capture: "complete capture", CaptureFormat: "html", SourceUrl: source.SourceUrl, ConfigVersion: 1, Sources: []domain.Source{source}, EmptyJournals: []domain.EmptyJournal{}}
	for index, repository := range repositories {
		workers.Go(func() { failures[index] = repository.PublishRefresh(ctx, leases[index], publication, 105) })
	}
	workers.Wait()
	assertCurrentGenerationPublished(t, leases, failures)
	journals, err := repositories[0].LoadJournals(ctx)
	if err != nil || journals[0].Sources[0].Revision != 2 {
		t.Fatal(journals, err)
	}
}

func TestReadersSeeOnePublicationSnapshot(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "state.sqlite")
	if _, err := migration.Migrate(ctx, path); err != nil {
		t.Fatal(err)
	}
	repository, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	source := domain.Source{CatalogIds: []string{"a"}, JournalTitle: "Journal", Title: "Topic", TypeText: "Special Issue", DateText: "Submission deadline: 31 December 2026", SourceUrl: "https://example.org/cfp", CheckedOn: "2026-09-15", Scope: "revision-1"}
	payload, _ := json.Marshal(domain.Seed{FormatVersion: 1, Sources: []domain.Source{source}, EmptyJournals: []domain.EmptyJournal{}})
	if _, err = repository.ImportSeed(ctx, "fixture", payload); err != nil {
		t.Fatal(err)
	}
	finished := make(chan error, 1)
	go func() {
		for revision := 2; revision <= 20; revision++ {
			lease, err := repository.BeginRefresh(ctx, "journal:a", 100, 20)
			if err != nil {
				finished <- err
				return
			}
			source.Scope = fmt.Sprintf("revision-%d", revision)
			err = repository.PublishRefresh(ctx, lease, Publication{Capture: source.Scope, CaptureFormat: "html", SourceUrl: source.SourceUrl, ConfigVersion: 1, Sources: []domain.Source{source}, EmptyJournals: []domain.EmptyJournal{}}, 105)
			if err != nil {
				finished <- err
				return
			}
		}
		finished <- nil
	}()
	for index := 0; index < 60; index++ {
		journals, err := repository.LoadJournals(ctx)
		if err != nil {
			t.Fatal(err)
		}
		journal := journals[0]
		if journal.Notices[0].Scope != fmt.Sprintf("revision-%d", journal.Sources[0].Revision) {
			t.Fatal("reader mixed header and content revisions", journal)
		}
	}
	if err := <-finished; err != nil {
		t.Fatal(err)
	}
}

// assertSingleConcurrentImport requires every worker to succeed but only one import marker to be created.
func assertSingleConcurrentImport(t *testing.T, results []ImportResult, failures []error) {
	t.Helper()
	imports := 0
	for index, result := range results {
		if failures[index] != nil {
			t.Fatal(failures[index])
		}
		if result.DidImport {
			imports++
		}
	}
	if imports != 1 {
		t.Fatal("seed imported more than once", imports)
	}
}

// assertUniqueConcurrentGenerations requires every admitted refresh to own a distinct generation.
func assertUniqueConcurrentGenerations(t *testing.T, leases []RefreshLease, failures []error) {
	t.Helper()
	seen := map[int64]bool{}
	for index, lease := range leases {
		if failures[index] != nil {
			t.Fatal(failures[index])
		}
		if seen[lease.Generation] {
			t.Fatal("duplicate generation")
		}
		seen[lease.Generation] = true
	}
}

// assertCurrentGenerationPublished admits only the eighth generation and requires stale errors for every predecessor.
func assertCurrentGenerationPublished(t *testing.T, leases []RefreshLease, failures []error) {
	t.Helper()
	for index, err := range failures {
		if leases[index].Generation == 8 {
			if err != nil {
				t.Fatal(err)
			}
		} else if !errors.Is(err, ErrStaleRefresh) {
			t.Fatal("old generation published", err)
		}
	}
}

// concurrentSeedPayload preserves the exact original seed fixture for all concurrent repositories.
func concurrentSeedPayload(t *testing.T, source domain.Source) []byte {
	t.Helper()
	payload, err := json.Marshal(domain.Seed{FormatVersion: 1, Sources: []domain.Source{source}, EmptyJournals: []domain.EmptyJournal{}})
	if err != nil {
		t.Fatal(err)
	}
	return payload
}
