package query

import (
	"reflect"
	"slices"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/weekly"
)

// TestWeeklyJournalOrdering preserves count priority, ASCII title folding and deterministic ID ties.
func TestWeeklyJournalOrdering(t *testing.T) {
	titles := []string{"z", "", "Alpha", "ALPHA", "Ä", "ä", "alpha", "last"}
	items := []domain.WeeklyJournalSummary{
		{JournalID: 9, JournalTitle: &titles[0], NewArticleCount: 3},
		{JournalID: 8, JournalTitle: nil, NewArticleCount: 2},
		{JournalID: 7, JournalTitle: &titles[1], NewArticleCount: 2},
		{JournalID: 6, JournalTitle: &titles[2], NewArticleCount: 2},
		{JournalID: 5, JournalTitle: &titles[3], NewArticleCount: 2},
		{JournalID: 4, JournalTitle: &titles[4], NewArticleCount: 2},
		{JournalID: 3, JournalTitle: &titles[5], NewArticleCount: 2},
		{JournalID: 2, JournalTitle: &titles[6], NewArticleCount: 2},
		{JournalID: 1, JournalTitle: &titles[7], NewArticleCount: 1},
	}
	original := slices.Clone(items)
	originalTitles := slices.Clone(titles)
	ordered := slices.Clone(items)
	compare := weeklyJournalSorter()
	slices.SortFunc(ordered, compare)
	var ids []identity.ID
	for index, journal := range ordered {
		ids = append(ids, journal.JournalID)
		if compare(journal, journal) != 0 || (index > 0 && (compare(ordered[index-1], journal) >= 0 || compare(journal, ordered[index-1]) <= 0)) {
			t.Fatal("journal ordering is inconsistent", ordered)
		}
	}
	if !reflect.DeepEqual(ids, []identity.ID{9, 7, 8, 2, 5, 6, 4, 3, 1}) {
		t.Fatal("journal count, title or ID priority changed", ids)
	}
	if !reflect.DeepEqual(items, original) || !reflect.DeepEqual(titles, originalTitles) {
		t.Fatal("journal comparison changed its input")
	}
}

// TestLegacyWeeklyJournalOrdering exercises the legacy wrapper with tied counts and titles.
func TestLegacyWeeklyJournalOrdering(t *testing.T) {
	articles := []domain.WeeklyArticle{
		{JournalID: 4, JournalTitle: "ä"},
		{JournalID: 3, JournalTitle: "Ä"},
		{JournalID: 2, JournalTitle: "Alpha"},
		{JournalID: 1, JournalTitle: "ALPHA"},
		{JournalID: 5, JournalTitle: ""},
		{JournalID: 6, JournalTitle: "Zulu"},
		{JournalID: 6, JournalTitle: "Zulu"},
	}
	original := slices.Clone(articles)
	generated, _ := weekly.ParseTimestamp("2026-10-03T12:00:00Z")
	for range 3 {
		result := legacyWeeklyDatabase("catalog.sqlite", &weeklyBucket{generated: generated}, articles)
		var ids []identity.ID
		for _, journal := range result.Journals {
			ids = append(ids, journal.JournalID)
		}
		if !reflect.DeepEqual(ids, []identity.ID{6, 5, 1, 2, 3, 4}) || result.NewArticleCount != len(articles) {
			t.Fatal("legacy journal ordering changed", result)
		}
	}
	if !reflect.DeepEqual(articles, original) {
		t.Fatal("legacy journal ordering changed input articles")
	}
}
