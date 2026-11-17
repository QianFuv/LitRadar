package query

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"errors"
	"fmt"
	"testing"
	"time"

	native "modernc.org/sqlite"
)

// relatedCancellationAttempts bounds how many samples may absorb runner scheduling stalls.
const relatedCancellationAttempts = 3

// TestRelatedCancellationInterruptsActiveRanking waits until bm25 reads its first
// document length, then checks that SQLite stops and releases the physical connection. Each
// interruption is compared with a complete ranking on the same connection rather than a fixed
// bound. A single sample on a loaded runner can stall for longer than a whole ranking, so the
// test passes once any of a few attempts shows the interruption; a broken cancellation runs every
// ranking to completion and still fails all of them.
func TestRelatedCancellationInterruptsActiveRanking(t *testing.T) {
	_, database := queryFixture(t)
	if _, err := database.Exec(`WITH RECURSIVE documents(id) AS (
 VALUES(1) UNION ALL SELECT id+1 FROM documents WHERE id<100000
)
INSERT INTO article_search(rowid,title,abstract_text)
 SELECT 100000+id,'common','common common' FROM documents;`); err != nil {
		t.Fatal(err)
	}
	samples := make([]string, 0, relatedCancellationAttempts)
	for attempt := range int64(relatedCancellationAttempts) {
		interrupted, complete := measureRelatedCancellation(t, database, attempt+1)
		if interrupted <= complete/2 {
			t.Logf("active ranking interrupted in %s; complete ranking takes %s; earlier samples %v", interrupted, complete, samples)
			return
		}
		samples = append(samples, fmt.Sprintf("interrupted %s, complete %s", interrupted, complete))
	}
	t.Fatal("SQLite continued ranking after cancellation in every attempt", samples)
}

// measureRelatedCancellation cancels one active ranking on a fresh physical connection and
// times a complete one for the same source identifier, which keeps every attempt outside the
// related result cache. FTS5 caches its document-length statement per connection, so only a
// new connection prepares it again and signals the start of ranking.
func measureRelatedCancellation(t *testing.T, database *sql.DB, id int64) (time.Duration, time.Duration) {
	t.Helper()
	started := make(chan struct{})
	isSignaled := false
	connection, err := database.Conn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if err := connection.Raw(func(raw any) error {
		return raw.(native.AuthorizerRegisterer).RegisterAuthorizer(func(operation native.AuthorizerActionCode, table, column, database, trigger string) native.AuthorizerReturnCode {
			if operation == native.AuthRead && table == "article_search_docsize" && !isSignaled {
				isSignaled = true
				close(started)
			}
			return native.AuthorizerOK
		})
	}); err != nil {
		t.Fatal(err)
	}
	if err := connection.Close(); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := relatedArticleIds(ctx, database, "", "common", id, 30)
		done <- err
	}()
	select {
	case <-started:
	case err := <-done:
		t.Fatal("ranking finished before the in-flight cancellation signal", err)
	case <-ctx.Done():
		t.Fatal("ranking did not start before its deadline", <-done)
	}
	stopped := time.Now()
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatal("active bm25 query did not report cancellation", err)
	}
	interrupted := time.Since(stopped)
	ranked := time.Now()
	if _, err := relatedArticleIds(t.Context(), database, "", "common", id, 30); err != nil {
		t.Fatal("interrupted query retained or poisoned the only connection", err)
	}
	complete := time.Since(ranked)
	connection, err = database.Conn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if err := connection.Raw(func(any) error { return driver.ErrBadConn }); !errors.Is(err, driver.ErrBadConn) {
		t.Fatal("measured connection was not discarded", err)
	}
	return interrupted, complete
}
