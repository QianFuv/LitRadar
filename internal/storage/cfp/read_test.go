package cfp

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"reflect"
	"slices"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
)

// TestDecodedNoticesMatchUncachedSnapshots protects payloads and caller ownership on cold and warm reads.
func TestDecodedNoticesMatchUncachedSnapshots(t *testing.T) {
	repository := preparationRepository(t)
	source := preparationSource([]string{"a", "alias"}, "Original topic")
	source.Title, source.TypeText = "2026年度选题", "General"
	source.TimeZone, source.StatusText = new("UTC"), new("Invitation only")
	source.DateText = "Submission deadline: 31 December 2026"
	seed := domain.Seed{FormatVersion: 1, Sources: []domain.Source{source}, EmptyJournals: []domain.EmptyJournal{{CatalogIds: []string{"z"}, JournalTitle: "Empty Journal", CheckedOn: "2026-09-15", SourceUrl: source.SourceUrl, SourceStatement: "No open calls"}}}
	encoded, err := json.Marshal(seed)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.ImportSeed(context.Background(), "fixture", encoded); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.database.Exec("UPDATE cfp_sources SET last_attempt=100,last_success=90,last_error='warning',lease_expires_at=120"); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	expected, err := repository.loadJournals(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	for range 3 {
		actual, err := repository.LoadJournals(ctx)
		if err != nil || !reflect.DeepEqual(actual, expected) {
			t.Fatal("decoded cache changed a snapshot", actual, expected, err)
		}
		actual[0].CatalogIds[0] = "changed alias"
		actual[0].Sources[0].Revision = 100
		for index := range actual {
			journal := &actual[index]
			if journal.SourceUrl != nil {
				*journal.SourceUrl = "changed url"
			}
			if journal.SourceStatement != nil {
				*journal.SourceStatement = "changed statement"
			}
			for index := range journal.Sources {
				status := &journal.Sources[index]
				*status.LastAttempt, *status.LastSuccess, *status.LeaseExpiresAt, *status.LastError = 0, 0, 0, "changed error"
			}
		}
		actual[0].Notices[0].Title = "changed title"
		notice := &actual[0].Notices[0]
		if len(notice.Dates) == 0 || notice.TimeZone == nil || notice.SourceStatus == nil || notice.TopicYear == nil {
			t.Fatal("fixture must exercise every mutable notice field", notice)
		}
		notice.Dates[0].Date = "1900-01-01"
		*notice.TimeZone, *notice.SourceStatus, *notice.TopicYear = "changed", domain.Closed, 1900
	}
}

// TestSnapshotCacheConnectionLifecycle prevents reuse after a failed or replaced version-check connection.
func TestSnapshotCacheConnectionLifecycle(t *testing.T) {
	repository := preparationRepository(t)
	ctx := context.Background()
	if _, err := repository.LoadJournals(ctx); err != nil {
		t.Fatal(err)
	}
	if err := repository.snapshotConnection.Close(); err != nil {
		t.Fatal(err)
	}
	importPreparationSource(t, repository, preparationSource([]string{"a"}, "New topic"))
	if _, err := repository.LoadJournals(ctx); !errors.Is(err, sql.ErrConnDone) {
		t.Fatal("failed connection unexpectedly served cached data", err)
	}
	if journals, err := repository.LoadJournals(ctx); err != nil || len(journals) != 1 {
		t.Fatal("replacement connection reused the old version identity", journals, err)
	}
	if err := repository.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.LoadJournals(ctx); err == nil {
		t.Fatal("closed repository served a cached snapshot")
	}
}

// TestSnapshotCacheObservesAnotherProcess prevents process-local invalidation from hiding CLI imports.
func TestSnapshotCacheObservesAnotherProcess(t *testing.T) {
	const pathVariable = "LITRADAR_CFP_CACHE_TEST_DATABASE"
	if filename := os.Getenv(pathVariable); filename != "" {
		repository, err := Open(filename)
		if err != nil {
			t.Fatal(err)
		}
		defer repository.Close()
		importPreparationSource(t, repository, preparationSource([]string{"external"}, "Imported by another process"))
		return
	}
	repository := preparationRepository(t)
	ctx := context.Background()
	if journals, err := repository.LoadJournals(ctx); err != nil || len(journals) != 0 {
		t.Fatal(journals, err)
	}
	var filename string
	if err := repository.database.QueryRow("SELECT file FROM pragma_database_list WHERE name='main'").Scan(&filename); err != nil {
		t.Fatal(err)
	}
	command := exec.Command(os.Args[0], "-test.run=^TestSnapshotCacheObservesAnotherProcess$", "-test.timeout=30s")
	command.Env = append(os.Environ(), pathVariable+"="+filename)
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatal("external import failed", string(output), err)
	}
	journals, err := repository.LoadJournals(ctx)
	if err != nil || len(journals) != 1 || journals[0].Notices[0].Title != "Imported by another process" {
		t.Fatal("external import remained cached", journals, err)
	}
}

// TestDecodedNoticesObserveIndependentWriters requires fresh imports, leases, failures and publications.
func TestDecodedNoticesObserveIndependentWriters(t *testing.T) {
	reader := preparationRepository(t)
	var filename string
	if err := reader.database.QueryRow("SELECT file FROM pragma_database_list WHERE name='main'").Scan(&filename); err != nil {
		t.Fatal(err)
	}
	writer, err := Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	ctx := context.Background()
	assertFresh := func() []JournalSnapshot {
		t.Helper()
		actual, err := reader.LoadJournals(ctx)
		if err != nil {
			t.Fatal(err)
		}
		expected, err := writer.loadJournals(ctx, nil)
		if err != nil || !reflect.DeepEqual(actual, expected) {
			t.Fatal("reader retained data across an independent write", actual, expected, err)
		}
		return actual
	}
	assertFresh()
	source := preparationSource([]string{"a", "alias"}, "Original topic")
	importPreparationSource(t, writer, source)
	if actual := assertFresh(); len(actual) != 1 || len(actual[0].Notices) != 1 {
		t.Fatal("import not visible", actual)
	}
	lease, err := writer.BeginRefresh(ctx, "journal:a", 100, 20)
	if err != nil {
		t.Fatal(err)
	}
	assertFresh()
	if err := writer.FailRefresh(ctx, lease, "failed capture", false); err != nil {
		t.Fatal(err)
	}
	assertFresh()
	lease, err = writer.BeginRefresh(ctx, "journal:a", 130, 20)
	if err != nil {
		t.Fatal(err)
	}
	assertFresh()
	source.Scope, source.CheckedOn = "updated original", "2026-10-20"
	publication := Publication{Capture: "updated capture", CaptureFormat: "html", SourceUrl: source.SourceUrl, Sources: []domain.Source{source}, ConfigVersion: 1}
	if err := writer.PublishFullText(ctx, lease, publication, 135, 0); err != nil {
		t.Fatal(err)
	}
	if actual := assertFresh(); actual[0].Notices[0].Scope != source.Scope || actual[0].Sources[0].Revision != 2 {
		t.Fatal("publication not visible", actual)
	}
	lease, err = writer.BeginRefresh(ctx, "journal:a", 160, 20)
	if err != nil {
		t.Fatal(err)
	}
	publication.Sources = nil
	publication.EmptyJournals = []domain.EmptyJournal{{CatalogIds: source.CatalogIds, JournalTitle: source.JournalTitle, CheckedOn: source.CheckedOn, SourceUrl: source.SourceUrl, SourceStatement: "No open calls"}}
	if err := writer.PublishRefresh(ctx, lease, publication, 165); err != nil {
		t.Fatal(err)
	}
	if actual := assertFresh(); len(actual[0].Notices) != 0 || actual[0].SourceStatement == nil {
		t.Fatal("deleted notices retained", actual)
	}
}

// TestSelectedJournalSnapshotsMatchFullReads keeps literal aliases, all owners and source ordering intact.
func TestSelectedJournalSnapshotsMatchFullReads(t *testing.T) {
	repository := preparationRepository(t)
	seed := domain.Seed{FormatVersion: 1, Sources: []domain.Source{
		preparationSource([]string{"a", "alias", " a "}, "First"),
		preparationSource([]string{"a", "alias", " a "}, "Second"),
		preparationSource([]string{"b", "other"}, "Other"),
	}}
	encoded, err := json.Marshal(seed)
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	if _, err := repository.ImportSeed(ctx, "fixture", encoded); err != nil {
		t.Fatal(err)
	}
	all, err := repository.loadJournals(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, aliases := range [][]string{nil, {}, {"a"}, {"alias", "alias"}, {"a", "alias"}, {"a", "other"}, {" a "}, {"Alias"}, {"missing"}, {"' OR 1=1 --"}} {
		expected := []JournalSnapshot{}
		for _, journal := range all {
			if slices.ContainsFunc(journal.CatalogIds, func(alias string) bool { return slices.Contains(aliases, alias) }) {
				expected = append(expected, journal)
			}
		}
		actual, err := repository.LoadJournalsForCatalogIds(ctx, aliases)
		if err != nil || !reflect.DeepEqual(actual, expected) {
			t.Fatal("selected read changed alias owners", aliases, actual, expected, err)
		}
	}
}

// TestDecodedNoticesValidateChangedPayloads prevents cached data from masking a malformed external update.
func TestDecodedNoticesValidateChangedPayloads(t *testing.T) {
	repository := preparationRepository(t)
	importPreparationSource(t, repository, preparationSource([]string{"a"}, "Topic"))
	ctx := context.Background()
	if _, err := repository.LoadJournals(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.database.Exec("UPDATE cfp_notices SET normalized_json='{}'"); err != nil {
		t.Fatal(err)
	}
	for _, load := range []func() ([]JournalSnapshot, error){
		func() ([]JournalSnapshot, error) { return repository.LoadJournals(ctx) },
		func() ([]JournalSnapshot, error) { return repository.LoadJournalsForCatalogIds(ctx, []string{"a"}) },
	} {
		_, err := load()
		var payloadError *PayloadError
		if !errors.As(err, &payloadError) {
			t.Fatal("changed payload bypassed validation", err)
		}
	}
}

// BenchmarkCfpSnapshotReads compares the former full decoder with warm catalogs and selected notices.
func BenchmarkCfpSnapshotReads(b *testing.B) {
	repository := preparationRepository(b)
	seed := domain.Seed{FormatVersion: 1}
	for journal := range 470 {
		for notice := range 3 + journal%2 {
			source := preparationSource([]string{fmt.Sprintf("journal-%03d", journal)}, fmt.Sprintf("Topic %d", notice))
			source.Scope = strings.Repeat("Original source content. ", 60)
			seed.Sources = append(seed.Sources, source)
		}
	}
	encoded, err := json.Marshal(seed)
	if err != nil {
		b.Fatal(err)
	}
	ctx := context.Background()
	if _, err := repository.ImportSeed(ctx, "benchmark", encoded); err != nil {
		b.Fatal(err)
	}
	for _, scenario := range []struct {
		name  string
		load  func() ([]JournalSnapshot, error)
		count int
	}{
		{"before-both-endpoints", func() ([]JournalSnapshot, error) { return repository.loadJournals(ctx, nil) }, 470},
		{"after-catalog-warm", func() ([]JournalSnapshot, error) { return repository.LoadJournals(ctx) }, 470},
		{"after-notices", func() ([]JournalSnapshot, error) {
			return repository.LoadJournalsForCatalogIds(ctx, []string{"journal-235"})
		}, 1},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			if _, err := scenario.load(); err != nil {
				b.Fatal(err)
			}
			b.ReportAllocs()
			for b.Loop() {
				result, err := scenario.load()
				if err != nil || len(result) != scenario.count {
					b.Fatal(len(result), err)
				}
			}
		})
	}
}
