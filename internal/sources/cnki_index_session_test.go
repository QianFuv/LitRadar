package sources

import (
	"context"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
)

// TestCnkiRetryRefreshesNavigationPreservesFrozenIssuesAndDetails exercises session-bound source handles.
func TestCnkiRetryRefreshesNavigationPreservesFrozenIssuesAndDetails(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A"), cnkiTestRow("B")}, 1)
	state.requiresSession = true
	state.issues = append(state.issues, map[string]any{"year_issue_id": "202512", "year": int64(2025), "number": "12"})
	calls := map[string]int{}
	state.detail = func(url string) (any, error) {
		calls[url]++
		if url == "B" && calls[url] == 1 {
			state.issues = []any{map[string]any{"year_issue_id": "202602", "year": int64(2026), "number": "2"}, state.issues[1], state.issues[0]}
			return nil, &cnki.Error{Kind: "Request", Message: "temporary"}
		}
		return map[string]any{"title": url, "authors": "Author"}, nil
	}
	first, err := cnkiTestFetch(index, nil)
	if err != nil || len(first.Articles) != 2 || state.journals.Load() != 2 || state.trees.Load() != 2 || calls["A"] != 1 || calls["B"] != 2 {
		t.Fatalf("session refresh failed: err=%v journals=%d trees=%d calls=%v", err, state.journals.Load(), state.trees.Load(), calls)
	}
	checkpoint, err := decodeCnkiCheckpoint(*first.Progress.Checkpoint)
	if err != nil || checkpoint.CandidateHeadIssueId != "202601" || checkpoint.CurrentIssueId != "202512" || checkpoint.PageIndex != 0 {
		t.Fatalf("retry changed frozen traversal: %#v %v", checkpoint, err)
	}
	last, err := cnkiTestFetch(index, first.Progress.Checkpoint)
	if err != nil || last.Progress.State != domain.Complete || state.trees.Load() != 2 {
		t.Fatalf("refreshed snapshot was not reusable: %#v %v", last.Progress, err)
	}
	anchor, err := decodeCnkiAnchor(*last.Progress.NextAnchor)
	if err != nil || anchor.YearIssueId != "202601" {
		t.Fatalf("frozen head changed: %#v %v", anchor, err)
	}
}

// TestCnkiRetryKeepsResumedPage ensures refreshed tokens cannot move an existing checkpoint.
func TestCnkiRetryKeepsResumedPage(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A")}, 1)
	state.requiresSession = true
	state.issues = append(state.issues, map[string]any{"year_issue_id": "202512", "year": int64(2025), "number": "12"})
	pages := 0
	state.page = func(issue any, page uint64) cnki.IssueArticlePage {
		pages++
		if providerField(issue, "year_issue_id") != "202512" || page != 2 {
			t.Error("resumed page moved")
		}
		return cnki.IssueArticlePage{Articles: []any{cnkiTestRow("A")}, ArticleCount: 1, PageIndex: page, HasNextPage: true}
	}
	calls := 0
	state.detail = func(string) (any, error) {
		calls++
		if calls == 1 {
			return nil, &cnki.Error{Kind: "Request", Message: "temporary"}
		}
		return map[string]any{"title": "A", "authors": "Author"}, nil
	}
	encoded, err := (cnki.Checkpoint{Version: cnki.CheckpointVersion, CandidateHeadIssueId: "202601", CurrentIssueId: "202512", PageIndex: 2}).Encode()
	if err != nil {
		t.Fatal(err)
	}
	batch, err := index.Fetch(context.Background(), domain.JournalCatalogEntry{CatalogId: "J", Title: "Journal"}, domain.IndexFetchContext{Mode: domain.Bootstrap, TraversalCheckpoint: &encoded})
	if err != nil {
		t.Fatal(err)
	}
	checkpoint, err := decodeCnkiCheckpoint(*batch.Progress.Checkpoint)
	if err != nil || checkpoint.CandidateHeadIssueId != "202601" || checkpoint.CurrentIssueId != "202512" || checkpoint.PageIndex != 3 || pages != 2 {
		t.Fatalf("resumed checkpoint changed: %#v %v pages=%d", checkpoint, err, pages)
	}
}

// TestCnkiSessionRefreshRejectsChangedCoverage keeps failed refreshes recoverable without skipping issues.
func TestCnkiSessionRefreshRejectsChangedCoverage(t *testing.T) {
	for _, scenario := range []struct {
		name string
		ids  []string
	}{
		{"missing-head", []string{"202512"}},
		{"missing-later-issue", []string{"202601"}},
		{"duplicate", []string{"202601", "202512", "202512"}},
		{"invalid", []string{"202601", "202512", "bad/id"}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A")}, 1)
			state.requiresSession = true
			state.issues = append(state.issues, map[string]any{"year_issue_id": "202512", "year": int64(2025), "number": "12"})
			original := copyCnkiJson(state.issues).([]any)
			pages := 0
			state.page = func(_ any, page uint64) cnki.IssueArticlePage {
				pages++
				return cnki.IssueArticlePage{Articles: []any{cnkiTestRow("A")}, ArticleCount: 1, PageIndex: page}
			}
			calls := 0
			state.detail = func(string) (any, error) {
				calls++
				if calls == 1 {
					state.issues = nil
					for _, id := range scenario.ids {
						state.issues = append(state.issues, map[string]any{"year_issue_id": id, "year": int64(2026)})
					}
					return nil, &cnki.Error{Kind: "Request", Message: "temporary"}
				}
				return map[string]any{"title": "A", "authors": "Author"}, nil
			}
			batch, err := cnkiTestFetch(index, nil)
			if err == nil || pages != 1 || batch.Progress.Checkpoint != nil || batch.Progress.NextAnchor != nil {
				t.Fatalf("invalid refresh advanced traversal: %#v %v pages=%d", batch.Progress, err, pages)
			}
			snapshot := index.snapshots["J"]
			if !snapshot.shouldRefresh || providerField(snapshot.issues[0], "year_issue") != int32(0) || len(snapshot.issues) != 2 {
				t.Fatal("failed refresh replaced the frozen snapshot")
			}
			state.issues = original
			if _, err := cnkiTestFetch(index, nil); err != nil || pages != 2 {
				t.Fatalf("valid refresh could not recover: %v pages=%d", err, pages)
			}
		})
	}
}

// TestCnkiSessionRefreshClearsAbsentTokens preserves stable-ID fallback and frozen issue metadata.
func TestCnkiSessionRefreshClearsAbsentTokens(t *testing.T) {
	frozen := cnkiJournalSnapshot{journal: map[string]any{"time": "old"}, issues: []any{map[string]any{"year_issue_id": "202601", "year_issue": "old-token", "year": int64(2026), "number": "1"}}}
	fresh := cnkiJournalSnapshot{journal: map[string]any{"time": "new"}, issues: []any{map[string]any{"year_issue_id": "202601", "year": int64(2027), "number": "changed"}}}
	refreshed, err := refreshCnkiSnapshot(frozen, fresh)
	if err != nil || providerField(refreshed.journal, "time") != "new" || providerField(refreshed.issues[0], "year_issue") != nil || providerField(refreshed.issues[0], "year") != int64(2026) || providerField(refreshed.issues[0], "number") != "1" || providerField(frozen.issues[0], "year_issue") != "old-token" {
		t.Fatalf("refresh lost frozen metadata or retained a stale token: %v", err)
	}
}
