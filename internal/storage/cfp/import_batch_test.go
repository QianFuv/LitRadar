package cfp

import (
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
)

// batchSources creates distinct notices with literal fields that must survive publication unchanged.
func batchSources(count int) []domain.Source {
	sources := make([]domain.Source, count)
	for index := range sources {
		sources[index] = preparationSource([]string{"a", "alias"}, fmt.Sprintf("Topic %04d", count-index))
		sources[index].Scope = fmt.Sprintf("Original 中文 scope %d\n\"quoted\"\u2028", index)
	}
	return sources
}

// preparedBatch validates one reusable seed before entering the measured import transaction.
func preparedBatch(t testing.TB, sources []domain.Source) *PreparedSeed {
	t.Helper()
	encoded, err := json.Marshal(domain.Seed{FormatVersion: 1, Sources: sources})
	if err != nil {
		t.Fatal(err)
	}
	prepared, err := PrepareSeed(encoded)
	if err != nil {
		t.Fatal(err)
	}
	return prepared
}

// assertBatchPublication checks literal ordering and every normalized notice through public readers.
func assertBatchPublication(t testing.TB, repository *Repository, sources []domain.Source) {
	t.Helper()
	originals, err := repository.LoadOriginals(context.Background(), "journal:a")
	if err != nil || !reflect.DeepEqual(originals, sources) {
		t.Fatal("literal notice ordering or fields changed", originals, err)
	}
	journals, err := repository.LoadJournals(context.Background())
	if err != nil || len(journals) != 1 || len(journals[0].Notices) != len(sources) {
		t.Fatal("normalized publication is incomplete", journals, err)
	}
	for index, source := range sources {
		if expected := domain.ParseSource(source); expected == nil || !reflect.DeepEqual(journals[0].Notices[index], *expected) {
			t.Fatal("normalized notice ordering or fields changed", index, journals[0].Notices[index], expected)
		}
	}
}

// TestNoticeBatchFailureIsAtomic preserves imports and last-good refreshes after a later insert fails.
func TestNoticeBatchFailureIsAtomic(t *testing.T) {
	for _, isRefresh := range []bool{false, true} {
		t.Run(fmt.Sprintf("refresh=%t", isRefresh), func(t *testing.T) {
			repository := preparationRepository(t)
			ctx := context.Background()
			sources := batchSources(3)
			prepared := preparedBatch(t, sources)
			publication := Publication{Capture: "new capture", CaptureFormat: "html", SourceUrl: sources[0].SourceUrl, Sources: sources, ConfigVersion: 1}
			var lease RefreshLease
			var before []JournalSnapshot
			var oldSources []domain.Source
			var capture string
			if isRefresh {
				oldSources = batchSources(1)
				importPreparationSource(t, repository, oldSources[0])
				var err error
				lease, err = repository.BeginRefresh(ctx, "journal:a", 100, 20)
				if err != nil {
					t.Fatal(err)
				}
				before, err = repository.LoadJournals(ctx)
				if err != nil {
					t.Fatal(err)
				}
				if err := repository.database.QueryRowContext(ctx, "SELECT capture FROM cfp_sources WHERE source_key='journal:a'").Scan(&capture); err != nil {
					t.Fatal(err)
				}
			}
			if _, err := repository.database.ExecContext(ctx, "CREATE TRIGGER reject_notice BEFORE INSERT ON cfp_notices WHEN NEW.display_order=1 BEGIN SELECT RAISE(ABORT,'notice rejected'); END"); err != nil {
				t.Fatal(err)
			}
			if isRefresh {
				if err := repository.PublishRefresh(ctx, lease, publication, 105); err == nil {
					t.Fatal("later notice failure was ignored")
				}
				after, err := repository.LoadJournals(ctx)
				if err != nil || !reflect.DeepEqual(before, after) {
					t.Fatal("failed refresh changed publication or lease", after, err)
				}
				assertBatchPublication(t, repository, oldSources)
				var afterCapture string
				if err := repository.database.QueryRowContext(ctx, "SELECT capture FROM cfp_sources WHERE source_key='journal:a'").Scan(&afterCapture); err != nil || capture != afterCapture {
					t.Fatal("failed refresh changed source capture", afterCapture, err)
				}
			} else {
				if _, err := repository.ImportPrepared(ctx, "batch", prepared); err == nil {
					t.Fatal("later notice failure was ignored")
				}
				for _, table := range []string{"cfp_notices", "cfp_source_journals", "cfp_journal_aliases", "cfp_journals", "cfp_sources", "cfp_seed_imports"} {
					var count int
					if err := repository.database.QueryRowContext(ctx, "SELECT COUNT(*) FROM "+table).Scan(&count); err != nil || count != 0 {
						t.Fatal("failed import published partial state", table, count, err)
					}
				}
			}
			if _, err := repository.database.ExecContext(ctx, "DROP TRIGGER reject_notice"); err != nil {
				t.Fatal(err)
			}
			if isRefresh {
				if err := repository.PublishRefresh(ctx, lease, publication, 105); err != nil {
					t.Fatal("failed refresh could not retry the same lease", err)
				}
			} else if result, err := repository.ImportPrepared(ctx, "batch", prepared); err != nil || !result.DidImport || result.Notices != 3 {
				t.Fatal("failed import could not retry the same seed", result, err)
			}
			assertBatchPublication(t, repository, sources)
		})
	}
}

// BenchmarkNoticeBatch measures complete imports and refreshes without accumulating stored notices.
func BenchmarkNoticeBatch(b *testing.B) {
	for _, count := range []int{1, 20, 200} {
		for _, isRefresh := range []bool{false, true} {
			b.Run(fmt.Sprintf("notices=%d/refresh=%t", count, isRefresh), func(b *testing.B) {
				repository := preparationRepository(b)
				ctx := context.Background()
				sources := batchSources(count)
				prepared := preparedBatch(b, sources)
				publication := Publication{Capture: "complete capture", CaptureFormat: "html", SourceUrl: sources[0].SourceUrl, Sources: sources, ConfigVersion: 1}
				if isRefresh {
					importPreparationSource(b, repository, sources[0])
				}
				b.ReportAllocs()
				for b.Loop() {
					if isRefresh {
						b.StopTimer()
						lease, err := repository.BeginRefresh(ctx, "journal:a", 100, 20)
						if err != nil {
							b.Fatal(err)
						}
						b.StartTimer()
						if err := repository.PublishRefresh(ctx, lease, publication, 105); err != nil {
							b.Fatal(err)
						}
					} else {
						if result, err := repository.ImportPrepared(ctx, "batch", prepared); err != nil || !result.DidImport || result.Notices != uint64(count) {
							b.Fatal(result, err)
						}
					}
					b.StopTimer()
					if !isRefresh {
						for _, table := range []string{"cfp_notices", "cfp_source_journals", "cfp_journal_aliases", "cfp_journals", "cfp_sources", "cfp_seed_imports"} {
							if _, err := repository.database.ExecContext(ctx, "DELETE FROM "+table); err != nil {
								b.Fatal(err)
							}
						}
					}
					b.StartTimer()
				}
				b.StopTimer()
				if isRefresh {
					assertBatchPublication(b, repository, sources)
				}
			})
		}
	}
}
