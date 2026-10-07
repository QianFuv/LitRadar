package query

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"path/filepath"
	"reflect"
	"sync"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/mattn/go-sqlite3"
)

func candidateFixture(t *testing.T) string {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "candidates.sqlite")
	database, err := storage.OpenPlain(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	_, err = database.Exec(`CREATE TABLE journals(journal_id INTEGER PRIMARY KEY,title TEXT);
CREATE TABLE articles(article_id INTEGER PRIMARY KEY,journal_id INTEGER,issue_id INTEGER,title TEXT,abstract_text TEXT,date TEXT,open_access INTEGER,in_press INTEGER,doi TEXT);
INSERT INTO journals VALUES(1,'one'),(2,'two'),(50001,'large'),(65535,'inpress');
INSERT INTO articles VALUES(1,1,7,'first',NULL,'2026-10-01',0,0,NULL),(2,2,8,'second','abstract','2026-10-01',1,0,'doi'),(3,999,9,'orphan',NULL,'2026-10-05',0,0,NULL),(50001,50001,50001,'null-date',NULL,NULL,NULL,NULL,NULL),(65535,65535,NULL,'inpress',NULL,'2026-10-03',0,1,NULL),(65536,65535,NULL,'not-inpress',NULL,'2026-10-02',0,2,NULL);`)
	if err != nil {
		t.Fatal(err)
	}
	return filename
}

// TestLargeCandidateCancellationCleansPinnedConnection retains start/cancel/join/cleanup and reuse ordering.
func TestLargeCandidateCancellationCleansPinnedConnection(t *testing.T) {
	database, err := storage.OpenPlain(candidateFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	started := installCandidateCancellationSignal(t, connection)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := largeCandidateRows(ctx, connection, "a.article_id", []int64{1, 2}, "")
		done <- err
	}()
	select {
	case <-started:
	case <-ctx.Done():
		t.Fatal("candidate SELECT did not begin", <-done)
	}
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatal("executing SELECT did not report cancellation", err)
	}
	assertCanceledMembershipCleared(t, connection)
	if _, err := connection.ExecContext(context.Background(), "DROP VIEW journals; ALTER TABLE saved_journals RENAME TO journals"); err != nil {
		t.Fatal(err)
	}
	if _, err := largeCandidateRows(context.Background(), connection, "a.article_id", []int64{1, 1}, ""); err == nil {
		t.Fatal("fixture failed to induce a membership insert error")
	}
	assertCandidateConnectionReuse(t, connection)
}

func TestLargeCandidateMembershipPreservesAllThreeSelectors(t *testing.T) {
	filename := candidateFixture(t)
	for _, count := range []int{500, 501, 32766, 32767, 65536} {
		t.Run(fmt.Sprint(count), func(t *testing.T) {
			ids := make([]int64, 0, count+2)
			issueKeys, inpressKeys := []string{}, []string{}
			for id := 1; id <= count; id++ {
				ids = append(ids, int64(id))
				issueKeys = append(issueKeys, fmt.Sprintf("999:%d", id))
				inpressKeys = append(inpressKeys, fmt.Sprint(id))
			}
			ids = append(ids, 1, 1)
			issueKeys = append(issueKeys, "999:7", "999:7")
			inpressKeys = append(inpressKeys, "1", "1")
			original := append([]int64(nil), ids...)
			for _, selector := range []struct {
				name string
				run  func() ([]domain.ArticleCandidate, error)
				want []int64
			}{
				{"article", func() ([]domain.ArticleCandidate, error) {
					return FetchCandidatesForArticleIds(context.Background(), filename, ids)
				}, []int64{65535, 65536, 2, 1, 50001}},
				{"issue", func() ([]domain.ArticleCandidate, error) {
					return FetchCandidatesForIssueKeys(context.Background(), filename, issueKeys)
				}, []int64{2, 1, 50001}},
				{"inpress", func() ([]domain.ArticleCandidate, error) {
					return FetchCandidatesForInPressKeys(context.Background(), filename, inpressKeys)
				}, []int64{65535}},
			} {
				items, err := selector.run()
				if err != nil {
					t.Fatalf("%s: %v", selector.name, err)
				}
				want := []int64{}
				for _, id := range selector.want {
					if id <= int64(count) {
						want = append(want, id)
					}
				}
				actual := []int64{}
				for _, item := range items {
					actual = append(actual, item.ArticleId)
					if item.ArticleId == 50001 && (item.Date != nil || item.JournalTitle != "large" || item.Abstract != "" || item.OpenAccess) {
						t.Fatal("nullable candidate mapping changed", item)
					}
				}
				if !reflect.DeepEqual(actual, want) {
					t.Fatalf("%s order/membership: got %v want %v", selector.name, actual, want)
				}
			}
			if !reflect.DeepEqual(ids, original) {
				t.Fatal("caller identifiers were mutated")
			}
		})
	}
}

// installCandidateCancellationSignal preserves the callback lifetime and deliberately long-running SELECT.
func installCandidateCancellationSignal(t *testing.T, connection *sql.Conn) chan struct{} {
	t.Helper()
	started := make(chan struct{})
	var once sync.Once
	if err := connection.Raw(func(raw any) error {
		return raw.(*sqlite3.SQLiteConn).RegisterFunc("candidate_started", func() int { once.Do(func() { close(started) }); return 0 }, false)
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(context.Background(), `ALTER TABLE journals RENAME TO saved_journals;
CREATE VIEW journals AS WITH RECURSIVE numbers(value) AS (SELECT candidate_started() UNION ALL SELECT value+1 FROM numbers WHERE value<1000000000) SELECT 1 AS journal_id,CAST(sum(value) AS TEXT) AS title FROM numbers`); err != nil {
		t.Fatal(err)
	}
	return started
}

// assertCanceledMembershipCleared inspects TEMP state only after the canceled worker has joined.
func assertCanceledMembershipCleared(t *testing.T, connection *sql.Conn) {
	t.Helper()
	var tables int
	if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM sqlite_temp_master WHERE name='candidate_membership'").Scan(&tables); err != nil || tables != 0 {
		t.Fatal("cancellation left TEMP membership behind", tables, err)
	}
}

// assertCandidateConnectionReuse checks successive memberships after the deliberate insertion failure.
func assertCandidateConnectionReuse(t *testing.T, connection *sql.Conn) {
	t.Helper()
	for _, id := range []int64{50001, 1} {
		items, err := largeCandidateRows(context.Background(), connection, "a.article_id", []int64{id}, "")
		if err != nil || len(items) != 1 || items[0].ArticleId != id {
			t.Fatal("reused connection retained stale membership", items, err)
		}
	}
}
