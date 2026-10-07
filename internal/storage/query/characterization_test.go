package query

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
	"github.com/QianFuv/LitRadar/internal/testkit/content"
)

// queryFixture seeds real native catalog rows with ratings, a null date and sorted retraction data.
func queryFixture(t *testing.T) (config.Config, *sql.DB) {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	filename := filepath.Join(configuration.IndexDir, "catalog.sqlite")
	if err := content.Create(filename, 9); err != nil {
		t.Fatal(err)
	}
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	if _, err := database.Exec(`UPDATE journals SET area='X',abs_rating='4*';
 UPDATE articles SET date=NULL WHERE article_id=1001;
 UPDATE article_listing SET date=NULL WHERE article_id=1001;
 INSERT INTO article_retraction_dois VALUES(1012,'z'),(1012,'a');`); err != nil {
		t.Fatal(err)
	}
	return configuration, database
}

// TestArticlePagingKeepsNullCursorAndCountPolicy checks page metadata independently of the offset on a cursor request.
func TestArticlePagingKeepsNullCursorAndCountPolicy(t *testing.T) {
	configuration, _ := queryFixture(t)
	params := DefaultArticleListParams()
	params.Limit = 1
	first, err := ListArticles(context.Background(), configuration, nil, params)
	assertFirstArticlePage(t, first, err)
	params.Cursor = first.Page.NextCursor
	params.Offset = 99
	second, err := ListArticles(context.Background(), configuration, nil, params)
	if err != nil || len(second.Items) != 1 || second.Items[0].ArticleId != 1001 {
		t.Fatalf("null page: %#v %v", second, err)
	}
	if second.Page.Total != nil || second.Page.Offset != 99 || second.Page.HasMore == nil || *second.Page.HasMore {
		t.Fatalf("cursor metadata: %#v", second.Page)
	}
}

// assertFirstArticlePage checks the counted first page before admitting its cursor.
func assertFirstArticlePage(t *testing.T, first domain.Page[domain.Article], err error) {
	t.Helper()
	if err != nil || len(first.Items) != 1 || first.Items[0].ArticleId != 1012 {
		t.Fatalf("first page: %#v %v", first, err)
	}
	if first.Page.Total == nil || *first.Page.Total != 2 || first.Page.NextCursor == nil {
		t.Fatalf("first metadata: %#v", first.Page)
	}
}

// TestEmptyRatingBranchPreservesCursorBeforeMatchErrors checks the dedicated no-eligible-journal path.
func TestEmptyRatingBranchPreservesCursorBeforeMatchErrors(t *testing.T) {
	configuration, _ := queryFixture(t)
	params := DefaultArticleListParams()
	params.Ratings.AbsRating = []string{"missing"}
	query, cursor := "\"", "broken"
	params.Query, params.Cursor = &query, &cursor
	params.SearchMode = domain.SearchAdvanced
	if _, err := ListArticles(context.Background(), configuration, nil, params); !errors.Is(err, ErrInvalidCursor) {
		t.Fatalf("cursor precedence: %v", err)
	}
	cursor = "|1"
	if _, err := ListArticles(context.Background(), configuration, nil, params); !errors.Is(err, ErrInvalidSearchExpression) {
		t.Fatalf("match probe omitted: %v", err)
	}
	query = "Synthetic"
	includeTotal := true
	params.IncludeTotal = &includeTotal
	page, err := ListArticles(context.Background(), configuration, nil, params)
	assertEmptyArticlePage(t, page, err)
}

// assertEmptyArticlePage checks nonnil successful empty output and the explicit zero count.
func assertEmptyArticlePage(t *testing.T, page domain.Page[domain.Article], err error) {
	t.Helper()
	if err != nil || page.Items == nil || len(page.Items) != 0 {
		t.Fatalf("empty page: %#v %v", page, err)
	}
	if page.Page.Total == nil || *page.Page.Total != 0 || page.Page.HasMore == nil || *page.Page.HasMore {
		t.Fatalf("empty metadata: %#v", page.Page)
	}
}

// TestArticleEnrichmentPreservesFirstRequestedOccurrenceAndRetractionOrder checks duplicate and absent identifiers.
func TestArticleEnrichmentPreservesFirstRequestedOccurrenceAndRetractionOrder(t *testing.T) {
	_, database := queryFixture(t)
	items, err := fetchArticles(context.Background(), database, []int64{1012, 1001, 1012, 9999})
	if err != nil || len(items) != 2 || items[0].ArticleId != 1012 || items[1].ArticleId != 1001 {
		t.Fatalf("requested order: %#v %v", items, err)
	}
	if !reflect.DeepEqual(items[0].RetractionDois, []string{"a", "z"}) || items[1].RetractionDois == nil {
		t.Fatalf("retraction order: %#v", items)
	}
	empty, err := fetchArticles(context.Background(), nil, nil)
	if err != nil || empty == nil || len(empty) != 0 {
		t.Fatalf("empty identifiers touched SQL: %#v %v", empty, err)
	}
}

// TestJournalFiltersKeepValidationAndCanonicalMembership checks raw filter counts and canonical HasArticles.
func TestJournalFiltersKeepValidationAndCanonicalMembership(t *testing.T) {
	configuration, _ := queryFixture(t)
	blank := " "
	ratings := make([]string, 500)
	params := JournalListParams{Area: &blank, Ratings: domain.JournalRatings{AbsRating: ratings}, Limit: 0}
	if _, err := ListJournals(context.Background(), configuration, nil, params); err == nil || err.Error() != "limit must be between 1 and 200" {
		t.Fatalf("pagination precedence: %v", err)
	}
	params.Limit = 20
	if _, err := ListJournals(context.Background(), configuration, nil, params); err == nil || err.Error() != "search filters must contain at most 500 items" {
		t.Fatalf("blank area count: %v", err)
	}
	area, hasArticles := " X ", true
	params = JournalListParams{Area: &area, HasArticles: &hasArticles, Limit: 20}
	page, err := ListJournals(context.Background(), configuration, nil, params)
	if err != nil || len(page.Items) != 2 || page.Page.Total == nil || *page.Page.Total != 2 {
		t.Fatalf("canonical journals: %#v %v", page, err)
	}
}

// writeQueryManifest writes one deterministic fixed-window publication under the real discovery path.
func writeQueryManifest(t *testing.T, configuration config.Config, name string, ids []int64) {
	t.Helper()
	filename := filepath.Join(configuration.ProjectRoot, "data", "push_state", name+".changes.json")
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		t.Fatal(err)
	}
	payload, err := json.Marshal(map[string]any{"db_name": name, "generated_at": "2026-10-03T12:00:00Z", "run_id": "fixed", "notifiable_article_ids": ids})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, payload, 0600); err != nil {
		t.Fatal(err)
	}
}

// TestWeeklyJournalAdmissionPrecedesMalformedManifest checks the missing-journal error on an otherwise valid request.
func TestWeeklyJournalAdmissionPrecedesMalformedManifest(t *testing.T) {
	configuration, _ := queryFixture(t)
	writeQueryManifest(t, configuration, "catalog.sqlite", []int64{1001, 1012})
	filename := filepath.Join(configuration.ProjectRoot, "data", "push_state", "catalog.sqlite.changes.json")
	if err := os.WriteFile(filename, []byte("{broken"), 0600); err != nil {
		t.Fatal(err)
	}
	params := WeeklyArticlePageParams{DbName: "catalog.sqlite", JournalId: 9999, WindowEnd: "2026-10-04T00:00:00Z", Limit: 1}
	if _, err := WeeklyArticles(context.Background(), configuration, params, nil); err == nil || err.Error() != "Journal not found" {
		t.Fatalf("journal precedence: %v", err)
	}
	params.JournalId = 1
	if _, err := WeeklyArticles(context.Background(), configuration, params, nil); !errors.Is(err, weekly.ErrManifestJson) {
		t.Fatalf("manifest rejection: %v", err)
	}
}

// TestWeeklyMembershipKeepsNullDateAndLegacyCeiling checks fixed membership paging and pre-pruning reference admission.
func TestWeeklyMembershipKeepsNullDateAndLegacyCeiling(t *testing.T) {
	configuration, _ := queryFixture(t)
	writeQueryManifest(t, configuration, "catalog.sqlite", []int64{1001, 1012, 1001, 9999})
	params := WeeklyArticlePageParams{DbName: "catalog.sqlite", JournalId: 1, WindowEnd: "2026-10-04T00:00:00Z", Limit: 1}
	page, err := WeeklyArticles(context.Background(), configuration, params, nil)
	if err != nil || len(page.Items) != 1 || page.Items[0].ArticleId != 1001 {
		t.Fatalf("weekly null item: %#v %v", page, err)
	}
	if page.Page.HasMore == nil || *page.Page.HasMore || page.Page.Total != nil {
		t.Fatalf("weekly metadata: %#v", page.Page)
	}
	ids := make([]int64, 2001)
	for index := range ids {
		ids[index] = int64(index + 1)
	}
	writeQueryManifest(t, configuration, "missing.sqlite", ids)
	end, _ := weekly.ParseTimestamp(params.WindowEnd)
	if _, err := WeeklyUpdates(context.Background(), configuration, end); !errors.Is(err, ErrLegacyWeeklyLimit) {
		t.Fatalf("legacy pre-pruning ceiling: %v", err)
	}
}
