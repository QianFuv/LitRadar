package api

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
)

// TestCfpOptimizedResponsesMatchFullSnapshots compares HTTP bytes with the original full-snapshot projections.
func TestCfpOptimizedResponsesMatchFullSnapshots(t *testing.T) {
	handlers, router, token := cfpFixture(t)
	ctx := context.Background()
	baseline, err := storage.Open(handlers.storage.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer baseline.Close()
	snapshots, err := baseline.LoadJournals(ctx)
	if err != nil {
		t.Fatal(err)
	}
	entries, failure := handlers.catalogMembers("fixture.sqlite")
	if failure != nil {
		t.Fatal(failure)
	}
	for range 2 {
		for _, query := range []string{"", "Example", "unmatched"} {
			response := authRequest(router, "GET", "/api/cfp/journals?db=fixture.sqlite&q="+query, "", token)
			var actual cfpCatalogResponse
			if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &actual) != nil {
				t.Fatal(response.Code, response.Body.String())
			}
			expected, failure := cfpCatalog("fixture.sqlite", &query, entries, snapshots, actual.EvaluatedAt)
			if failure != nil {
				t.Fatal(failure)
			}
			encoded := httptest.NewRecorder()
			writeResponse(encoded, expected)
			if response.Body.String() != encoded.Body.String() {
				t.Fatal("catalog response bytes changed", response.Body.String(), encoded.Body.String())
			}
		}
	}
	for _, id := range []string{"cfp-fixture", "cfp-alias", "unadapted"} {
		for _, includeClosed := range []bool{false, true} {
			flag := "false"
			if includeClosed {
				flag = "true"
			}
			response := authRequest(router, "GET", "/api/cfp/journals/"+id+"/notices?db=fixture.sqlite&include_closed="+flag, "", token)
			var actual cfpNoticePage
			if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &actual) != nil {
				t.Fatal(response.Code, response.Body.String())
			}
			entry := entries[0]
			if id == "unadapted" {
				entry = entries[1]
			}
			snapshot, failure := cfpFindSnapshot(entry, snapshots)
			if failure != nil {
				t.Fatal(failure)
			}
			expected, failure := cfpPage(handlers.codec, "fixture.sqlite", entry, snapshot, includeClosed, nil, 50, actual.EvaluatedAt)
			if failure != nil {
				t.Fatal(failure)
			}
			encoded := httptest.NewRecorder()
			writeResponse(encoded, expected)
			if response.Body.String() != encoded.Body.String() {
				t.Fatal("notice response bytes changed", response.Body.String(), encoded.Body.String())
			}
		}
	}
	first := authRequest(router, "GET", "/api/cfp/journals/cfp-alias/notices?db=fixture.sqlite&limit=1", "", token)
	var actual cfpNoticePage
	if first.Code != 200 || json.Unmarshal(first.Body.Bytes(), &actual) != nil || actual.Page.NextCursor == nil {
		t.Fatal(first.Code, first.Body.String())
	}
	snapshot, failure := cfpFindSnapshot(entries[0], snapshots)
	if failure != nil {
		t.Fatal(failure)
	}
	expected, failure := cfpPage(handlers.codec, "fixture.sqlite", entries[0], snapshot, false, nil, 1, actual.EvaluatedAt)
	if failure != nil || expected.Page.NextCursor == nil {
		t.Fatal(expected, failure)
	}
	actualCursor, actualFailure := decodeCfpCursor(handlers.codec, *actual.Page.NextCursor)
	expectedCursor, expectedFailure := decodeCfpCursor(handlers.codec, *expected.Page.NextCursor)
	if actualFailure != nil || expectedFailure != nil || !reflect.DeepEqual(actualCursor, expectedCursor) {
		t.Fatal("encrypted cursor semantics changed", actualCursor, expectedCursor, actualFailure, expectedFailure)
	}
	var firstPage cfpRouteFirstPage
	if err := json.Unmarshal(first.Body.Bytes(), &firstPage); err != nil {
		t.Fatal(err)
	}
	assertCanonicalCfpContinuation(t, router, token, firstPage)
}

// TestCfpRoutesObserveExternalRefresh keeps live summaries and cursor rejection after independent publication.
func TestCfpRoutesObserveExternalRefresh(t *testing.T) {
	handlers, router, token := cfpFixture(t)
	assertCfpCatalogWithoutIndex(t, router, token)
	response := authRequest(router, "GET", "/api/cfp/journals/cfp-alias/notices?db=fixture.sqlite&limit=1", "", token)
	var first cfpRouteFirstPage
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &first) != nil || first.Page.NextCursor == nil {
		t.Fatal(response.Code, response.Body.String())
	}
	writer, err := storage.Open(handlers.storage.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	ctx := context.Background()
	lease, err := writer.BeginRefresh(ctx, "journal:cfp-fixture", first.EvaluatedAt, 120)
	if err != nil {
		t.Fatal(err)
	}
	response = authRequest(router, "GET", "/api/cfp/journals?db=fixture.sqlite&q=Example", "", token)
	var catalog cfpCatalogResponse
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &catalog) != nil || len(catalog.Items) != 1 || catalog.Items[0].RefreshStatus != "refreshing" {
		t.Fatal("lease status not visible", response.Code, response.Body.String())
	}
	source := domain.Source{CatalogIds: []string{"cfp-fixture", "cfp-alias"}, JournalTitle: "Example Journal", Title: "Updated call", TypeText: "Special Issue", DateText: "Submission deadline: 31 December 2000", SourceUrl: "https://example.org/calls", CheckedOn: "2026-10-20"}
	if err := writer.PublishRefresh(ctx, lease, storage.Publication{Capture: "updated capture", CaptureFormat: "html", SourceUrl: source.SourceUrl, Sources: []domain.Source{source}, ConfigVersion: 1}, first.EvaluatedAt); err != nil {
		t.Fatal(err)
	}
	response = authRequest(router, "GET", "/api/cfp/journals?db=fixture.sqlite&q=Example", "", token)
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &catalog) != nil || catalog.Items[0].NoticeCount != 1 || catalog.Items[0].CurrentCount != 0 || catalog.Items[0].RefreshStatus != "success" || catalog.Items[0].StateCounts[domain.Closed] != 1 {
		t.Fatal("published catalog not visible", response.Code, response.Body.String())
	}
	response = authRequest(router, "GET", "/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&cursor="+*first.Page.NextCursor, "", token)
	if response.Code != 409 || !strings.Contains(response.Body.String(), cfpCursorDetail) {
		t.Fatal("old cursor survived publication", response.Code, response.Body.String())
	}
}

// TestCfpSelectedReadRetainsConflictingAliasFailure rejects distinct owners even when only one is canonical.
func TestCfpSelectedReadRetainsConflictingAliasFailure(t *testing.T) {
	handlers, router, token := cfpFixture(t)
	seed := domain.Seed{FormatVersion: 1, Sources: []domain.Source{{CatalogIds: []string{"other"}, JournalTitle: "Other Journal", Title: "Other call", TypeText: "Special Issue", DateText: "Submission deadline: 31 December 2099", SourceUrl: "https://example.org/other", CheckedOn: "2026-09-15"}}}
	encoded, err := json.Marshal(seed)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := handlers.repository.ImportSeed(context.Background(), "other", encoded); err != nil {
		t.Fatal(err)
	}
	filename := filepath.Join(handlers.storage.MetaDir, "fixture.csv")
	catalog, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, []byte(strings.ReplaceAll(string(catalog), "cfp-fixture,cfp-alias,", "cfp-fixture,cfp-alias;other,")), 0600); err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"cfp-fixture", "cfp-alias", "other"} {
		response := authRequest(router, "GET", "/api/cfp/journals/"+id+"/notices?db=fixture.sqlite&cursor=invalid", "", token)
		if response.Code != 500 {
			t.Fatal("alias conflict lost precedence over cursor validation", id, response.Code, response.Body.String())
		}
	}
	response := authRequest(router, "GET", "/api/cfp/journals/unadapted/notices?db=fixture.sqlite", "", token)
	if response.Code != 200 {
		t.Fatal("unrelated alias conflict rejected unadapted member", response.Code, response.Body.String())
	}
}
