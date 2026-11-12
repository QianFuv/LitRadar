package query

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	native "modernc.org/sqlite"
)

// TestRelatedCancellationInterruptsActiveRanking waits until bm25 reads its first
// document length, then checks that SQLite stops and releases the physical connection. The
// interruption is compared with a complete ranking on the same connection rather than a fixed
// bound, so slow runners such as race-enabled Windows CI cannot fail it.
func TestRelatedCancellationInterruptsActiveRanking(t *testing.T) {
	_, database := queryFixture(t)
	if _, err := database.Exec(`WITH RECURSIVE documents(id) AS (
 VALUES(1) UNION ALL SELECT id+1 FROM documents WHERE id<100000
)
INSERT INTO article_search(rowid,title,abstract_text)
 SELECT 100000+id,'common','common common' FROM documents;`); err != nil {
		t.Fatal(err)
	}
	connection, err := database.Conn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	started := make(chan struct{})
	var once sync.Once
	if err := connection.Raw(func(raw any) error {
		return raw.(native.AuthorizerRegisterer).RegisterAuthorizer(func(operation native.AuthorizerActionCode, table, column, database, trigger string) native.AuthorizerReturnCode {
			if operation == native.AuthRead && table == "article_search_docsize" {
				once.Do(func() { close(started) })
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
		_, err := relatedArticleIds(ctx, database, "", "common", 1, 30)
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
	if _, err := relatedArticleIds(t.Context(), database, "", "common", 1, 30); err != nil {
		t.Fatal("interrupted query retained or poisoned the only connection", err)
	}
	complete := time.Since(ranked)
	if interrupted > complete/2 {
		t.Fatal("SQLite continued ranking after cancellation", interrupted, "complete ranking", complete)
	}
	t.Logf("active ranking interrupted in %s; complete ranking takes %s", interrupted, complete)
}
