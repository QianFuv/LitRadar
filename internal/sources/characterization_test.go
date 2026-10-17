package sources

import (
	"context"
	"errors"
	"math"
	"sync/atomic"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

// TestCnkiReturnsFirstOrdinalFailure preserves error selection and safe provider classification.
func TestCnkiReturnsFirstOrdinalFailure(t *testing.T) {
	for _, entry := range []struct {
		firstKind string
		laterKind string
		wantKind  provider.ErrorKind
	}{
		{"Parse", "Request", provider.InvalidResponse},
		{"Request", "Parse", provider.TemporarilyUnavailable},
	} {
		t.Run(entry.firstKind, func(t *testing.T) {
			index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A"), cnkiTestRow("B")}, 1)
			state.detail = func(url string) (any, error) {
				if url == "A" {
					return nil, &cnki.Error{Kind: entry.firstKind}
				}
				return nil, &cnki.Error{Kind: entry.laterKind}
			}
			var cache *cnkiPageCache
			attempts := []scholarly.Attempt{}
			_, err := index.fetchBatch(context.Background(), domain.JournalCatalogEntry{CatalogId: "J"}, domain.IndexFetchContext{}, &attempts, &cache)
			var failure *provider.Error
			if !errors.As(err, &failure) || failure.Kind != entry.wantKind || failure.Message != "domestic CNKI provider request failed" {
				t.Fatal("first ordinal failure was not preserved", err)
			}
		})
	}
}

// TestCnkiIssueAdmissionPrecedesDuplicateDetection preserves whole-tree validation priority.
func TestCnkiIssueAdmissionPrecedesDuplicateDetection(t *testing.T) {
	index, state := newCnkiIndexTest(t, nil, 1)
	state.issues = []any{state.issues[0], state.issues[0], map[string]any{"year_issue_id": "!"}}
	_, err := cnkiTestFetch(index, nil)
	if err == nil || err.Error() != "domestic CNKI issue payload omitted its stable year_issue_id" {
		t.Fatal("duplicate detection displaced issue admission", err)
	}
}

// TestCnkiRetryRetainsLaterSuccess preserves caching after an earlier ordinal failure.
func TestCnkiRetryRetainsLaterSuccess(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A"), cnkiTestRow("B")}, 2)
	var firstCalls, laterCalls atomic.Int32
	state.detail = func(url string) (any, error) {
		if url == "A" {
			if firstCalls.Add(1) == 1 {
				return nil, &cnki.Error{Kind: "Request"}
			}
		} else {
			laterCalls.Add(1)
		}
		return map[string]any{"title": url, "authors": "Author"}, nil
	}
	batch, err := cnkiTestFetch(index, nil)
	if err != nil || len(batch.Articles) != 2 || firstCalls.Load() != 2 || laterCalls.Load() != 1 {
		t.Fatal("later success was lost on retry", batch, err, firstCalls.Load(), laterCalls.Load())
	}
}

// TestCnkiDetailFailurePrecedesPageOverflow preserves detail work before progress admission.
func TestCnkiDetailFailurePrecedesPageOverflow(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A")}, 1)
	state.page = func(_ any, page uint64) cnki.IssueArticlePage {
		return cnki.IssueArticlePage{Articles: []any{cnkiTestRow("A")}, ArticleCount: 1, PageIndex: page, HasNextPage: true}
	}
	checkpoint := cnki.Checkpoint{Version: cnki.CheckpointVersion, CandidateHeadIssueId: "202601", CurrentIssueId: "202601", PageIndex: math.MaxUint64}
	encoded, err := checkpoint.Encode()
	if err != nil {
		t.Fatal(err)
	}
	state.detail = func(string) (any, error) { return nil, &cnki.Error{Kind: "Request"} }
	_, err = index.Fetch(context.Background(), domain.JournalCatalogEntry{CatalogId: "J"}, domain.IndexFetchContext{TraversalCheckpoint: &encoded})
	if err == nil || err.Error() != "domestic CNKI provider request failed" {
		t.Fatal("overflow displaced detail failure", err)
	}
	state.detail = func(url string) (any, error) { return map[string]any{"title": url, "authors": "Author"}, nil }
	_, err = index.Fetch(context.Background(), domain.JournalCatalogEntry{CatalogId: "J"}, domain.IndexFetchContext{TraversalCheckpoint: &encoded})
	if err == nil || err.Error() != "domestic CNKI page index overflowed" {
		t.Fatal("successful details did not reach overflow check", err)
	}
}
