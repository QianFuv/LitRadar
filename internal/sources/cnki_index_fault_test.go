package sources

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

type cnkiIndexTestState struct {
	clones, resets, journals, trees atomic.Int32
	issues                          []any
	page                            func(any, uint64) cnki.IssueArticlePage
	detail                          func(string) (any, error)
}
type cnkiIndexTestTransport struct {
	state      *cnkiIndexTestState
	generation int32
}

func (source *cnkiIndexTestTransport) Clone() *cnkiIndexTestTransport {
	source.state.clones.Add(1)
	return &cnkiIndexTestTransport{source.state, source.state.resets.Load()}
}
func (source *cnkiIndexTestTransport) ResetTransientState(context.Context) error {
	source.state.resets.Add(1)
	return nil
}
func (source *cnkiIndexTestTransport) ResolveJournal(context.Context, cnki.JournalLocator) (any, error) {
	source.state.journals.Add(1)
	return map[string]any{"pykm": "J"}, nil
}
func (source *cnkiIndexTestTransport) YearIssues(context.Context, any) ([]any, error) {
	source.state.trees.Add(1)
	return source.state.issues, nil
}
func (source *cnkiIndexTestTransport) IssueArticles(_ context.Context, _ any, issue any, page uint64) (cnki.IssueArticlePage, error) {
	return source.state.page(issue, page), nil
}
func (source *cnkiIndexTestTransport) ArticleDetail(_ context.Context, url string, _ *string) (any, error) {
	if source.generation != source.state.resets.Load() {
		return nil, errors.New("stale clone session")
	}
	return source.state.detail(url)
}
func (*cnkiIndexTestTransport) Attempts() []scholarly.Attempt      { return []scholarly.Attempt{} }
func (*cnkiIndexTestTransport) DrainAttempts() []scholarly.Attempt { return []scholarly.Attempt{} }

func newCnkiIndexTest(t *testing.T, rows []any, workers int) (*CnkiIndexProvider, *cnkiIndexTestState) {
	t.Helper()
	state := &cnkiIndexTestState{issues: []any{map[string]any{"year_issue_id": "202601", "year": int64(2026), "number": "1"}}}
	state.page = func(_ any, page uint64) cnki.IssueArticlePage {
		return cnki.IssueArticlePage{Articles: rows, ArticleCount: uint64(len(rows)), PageIndex: page}
	}
	state.detail = func(url string) (any, error) { return map[string]any{"title": url, "authors": "Author"}, nil }
	index, err := NewCnkiIndexProviderWithWorkers(&cnkiIndexTestTransport{state: state}, workers)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(index.Close)
	index.sleep = func(context.Context, time.Duration) error { return nil }
	return index, state
}
func cnkiTestRow(url string) any { return map[string]any{"article_url": url, "title": url} }
func cnkiTestFetch(index *CnkiIndexProvider, checkpoint *string) (domain.ProviderBatch, error) {
	return index.Fetch(context.Background(), domain.JournalCatalogEntry{CatalogId: "J", Title: "Journal"}, domain.IndexFetchContext{Mode: domain.Bootstrap, TraversalCheckpoint: checkpoint})
}

func TestCnkiRetryCacheRetainsSuccessAndFilterButRefetchesMissing(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A"), cnkiTestRow("B"), cnkiTestRow("C"), cnkiTestRow("D")}, 1)
	calls := map[string]int{}
	state.detail = func(url string) (any, error) {
		calls[url]++
		switch url {
		case "B":
			if calls[url] == 1 {
				return nil, &cnki.Error{Kind: "Request", Message: "temporary"}
			}
		case "C":
			return nil, &cnki.Error{Kind: "PermanentArticleMissing"}
		case "D":
			return map[string]any{"title": "filtered"}, nil
		}
		return map[string]any{"title": url, "authors": "Author"}, nil
	}
	delays := []time.Duration{}
	index.sleep = func(_ context.Context, delay time.Duration) error { delays = append(delays, delay); return nil }
	batch, err := cnkiTestFetch(index, nil)
	if err != nil || len(batch.Articles) != 2 || !reflect.DeepEqual(calls, map[string]int{"A": 1, "B": 2, "C": 2, "D": 1}) || state.resets.Load() != 1 || state.clones.Load() != 2 || !reflect.DeepEqual(delays, []time.Duration{time.Second}) {
		t.Fatalf("batch=%#v err=%v calls=%v reset=%d clones=%d delays=%v", batch, err, calls, state.resets.Load(), state.clones.Load(), delays)
	}
	if _, err := cnkiTestFetch(index, nil); err != nil {
		t.Fatal(err)
	}
	if calls["A"] != 2 || calls["D"] != 2 || state.trees.Load() != 2 {
		t.Fatal("cache escaped Fetch or completed snapshot survived", calls, state.trees.Load())
	}
}

func TestCnkiRetryCacheDetectsInPlacePageMutation(t *testing.T) {
	rows := []any{cnkiTestRow("A"), cnkiTestRow("B")}
	index, state := newCnkiIndexTest(t, rows, 1)
	pages := 0
	calls := map[string]int{}
	state.page = func(_ any, page uint64) cnki.IssueArticlePage {
		pages++
		if pages == 2 {
			rows[0].(map[string]any)["article_url"] = "A2"
		}
		return cnki.IssueArticlePage{Articles: rows, ArticleCount: 2, PageIndex: page}
	}
	state.detail = func(url string) (any, error) {
		calls[url]++
		if url == "B" && calls[url] == 1 {
			return nil, &cnki.Error{Kind: "Request", Message: "temporary"}
		}
		return map[string]any{"title": url, "authors": "Author"}, nil
	}
	batch, err := cnkiTestFetch(index, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(batch.Articles) != 2 || batch.Articles[0].Title != "A2" || calls["A2"] != 1 {
		t.Fatalf("stale cached article: %#v calls=%v", batch.Articles, calls)
	}
}

func TestCnkiSnapshotOwnsFrozenIssueTree(t *testing.T) {
	index, state := newCnkiIndexTest(t, nil, 1)
	state.issues = append(state.issues, map[string]any{"year_issue_id": "202512", "year": int64(2025), "number": "12"})
	first, err := cnkiTestFetch(index, nil)
	if err != nil {
		t.Fatal(err)
	}
	state.issues[1].(map[string]any)["year_issue_id"] = "changed"
	last, err := cnkiTestFetch(index, first.Progress.Checkpoint)
	if err != nil || last.Progress.State != domain.Complete || state.trees.Load() != 1 {
		t.Fatalf("frozen snapshot changed: %#v %v", last, err)
	}
}

func TestCnkiInvalidCheckpointRetriesBeforeNetwork(t *testing.T) {
	index, state := newCnkiIndexTest(t, nil, 1)
	delays := []time.Duration{}
	index.sleep = func(_ context.Context, delay time.Duration) error { delays = append(delays, delay); return nil }
	_, err := cnkiTestFetch(index, new("{}"))
	if err == nil || state.journals.Load() != 0 || state.resets.Load() != 2 || state.clones.Load() != 3 || !reflect.DeepEqual(delays, []time.Duration{time.Second, 2 * time.Second}) {
		t.Fatalf("err=%v journals=%d resets=%d clones=%d delays=%v", err, state.journals.Load(), state.resets.Load(), state.clones.Load(), delays)
	}
}

func TestCnkiMissingUrlPreventsEveryDetailSubmission(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A"), map[string]any{"title": "missing"}}, 2)
	var calls atomic.Int32
	state.detail = func(string) (any, error) { calls.Add(1); return nil, nil }
	_, err := cnkiTestFetch(index, nil)
	if err == nil || calls.Load() != 0 {
		t.Fatal("partial task submission", err, calls.Load())
	}
}

func TestCnkiBoundedWorkersPreserveArticleOrderAndJoin(t *testing.T) {
	rows := []any{}
	for ordinal := range 8 {
		rows = append(rows, cnkiTestRow(fmt.Sprint(ordinal)))
	}
	index, state := newCnkiIndexTest(t, rows, 3)
	var started atomic.Int32
	ready := make(chan struct{})
	release := make(chan struct{})
	var once sync.Once
	defer once.Do(func() { close(release) })
	state.detail = func(url string) (any, error) {
		if started.Add(1) == 3 {
			close(ready)
		}
		<-release
		return map[string]any{"title": url, "authors": "Author"}, nil
	}
	result := make(chan error, 1)
	go func() {
		batch, err := cnkiTestFetch(index, nil)
		if err == nil {
			if len(batch.Articles) != 8 {
				err = fmt.Errorf("missing articles")
			} else {
				for ordinal, article := range batch.Articles {
					if article.Title != fmt.Sprint(ordinal) {
						err = fmt.Errorf("unordered articles")
					}
				}
			}
		}
		result <- err
	}()
	select {
	case <-ready:
	case <-time.After(3 * time.Second):
		t.Fatal("workers did not reach concurrency")
	}
	once.Do(func() { close(release) })
	select {
	case err := <-result:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("workers did not join")
	}
	if index.pool.peak.Load() != 3 || index.pool.active.Load() != 0 || state.clones.Load() != 3 {
		t.Fatal("pool accounting", index.pool.peak.Load(), index.pool.active.Load(), state.clones.Load())
	}
}

func TestCnkiWorkerPanicIsContainedAndPoolRemainsUsable(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A")}, 1)
	state.detail = func(string) (any, error) { panic("private upstream data") }
	_, err := cnkiTestFetch(index, nil)
	var failure *provider.Error
	if !errors.As(err, &failure) || failure.Kind != provider.Internal || failure.Message != "domestic CNKI detail worker panicked" || state.resets.Load() != 0 || index.pool.active.Load() != 0 {
		t.Fatal(err)
	}
	state.detail = func(url string) (any, error) { return map[string]any{"title": url, "authors": "Author"}, nil }
	if _, err := cnkiTestFetch(index, nil); err != nil {
		t.Fatal(err)
	}
}

func TestCnkiWorkerCountValidatedBeforeClone(t *testing.T) {
	for _, count := range []int{0, 33} {
		state := &cnkiIndexTestState{}
		if _, err := NewCnkiIndexProviderWithWorkers(&cnkiIndexTestTransport{state: state}, count); err == nil || state.clones.Load() != 0 {
			t.Fatal("invalid count cloned transport", count, err)
		}
	}
}
