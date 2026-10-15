package query

import (
	"context"
	"fmt"
	"math"
	"reflect"
	"slices"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	native "github.com/mattn/go-sqlite3"
)

// TestWeeklyMembershipPreservesExactIntegerSet covers parameter boundaries and duplicate ownership.
func TestWeeklyMembershipPreservesExactIntegerSet(t *testing.T) {
	for _, count := range []int{0, 1, 2, 499, 500, 501, 1000, 1001} {
		t.Run(fmt.Sprint(count), func(t *testing.T) {
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
			ids := make([]int64, count)
			for index := range ids {
				ids[index] = int64(index + 1)
			}
			if count > 1 {
				ids[1] = ids[0]
			}
			if count > 7 {
				ids[2], ids[3], ids[4], ids[5] = 0, -1, math.MinInt64, math.MaxInt64
			}
			if count > 500 {
				ids[500] = ids[0]
			}
			before := slices.Clone(ids)
			expected := map[int64]bool{}
			for _, id := range ids {
				expected[id] = true
			}
			if err := installWeeklyMembership(context.Background(), connection, ids); err != nil {
				t.Fatal(err)
			}
			rows, err := connection.QueryContext(context.Background(), "SELECT article_id FROM temp.weekly_membership")
			if err != nil {
				t.Fatal(err)
			}
			defer rows.Close()
			actual := map[int64]bool{}
			for rows.Next() {
				var id int64
				if err := rows.Scan(&id); err != nil {
					t.Fatal(err)
				}
				actual[id] = true
			}
			if err := rows.Err(); err != nil || !reflect.DeepEqual(actual, expected) || !slices.Equal(ids, before) {
				t.Fatal("weekly membership or caller input changed", actual, expected, err)
			}
		})
	}
}

// TestWeeklyMembershipLateFailureRollsBack discards earlier accepted chunks and leaves the connection reusable.
func TestWeeklyMembershipLateFailureRollsBack(t *testing.T) {
	for _, shouldRejectStatistics := range []bool{false, true} {
		t.Run(fmt.Sprintf("statistics=%t", shouldRejectStatistics), func(t *testing.T) {
			assertWeeklyMembershipFailure(t, shouldRejectStatistics)
		})
	}
}

// assertWeeklyMembershipFailure checks rollback and reuse after insertion or planner-statistics rejection.
func assertWeeklyMembershipFailure(t *testing.T, shouldRejectStatistics bool) {
	t.Helper()
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
	insertAuthorizations := 0
	hasRejected, hasCanonicalAnalyze := false, false
	if err := connection.Raw(func(raw any) error {
		raw.(*native.SQLiteConn).RegisterAuthorizer(func(operation int, table, column, database string) int {
			if operation == native.SQLITE_ANALYZE {
				if database != "temp" {
					hasCanonicalAnalyze = true
					return native.SQLITE_DENY
				}
				if shouldRejectStatistics && table == "weekly_membership" {
					hasRejected = true
					return native.SQLITE_DENY
				}
			}
			if operation == native.SQLITE_INSERT && table == "weekly_membership" {
				insertAuthorizations++
				if !shouldRejectStatistics && insertAuthorizations == 2 {
					hasRejected = true
					return native.SQLITE_DENY
				}
			}
			return native.SQLITE_OK
		})
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, 501)
	for index := range ids {
		ids[index] = int64(index + 1)
	}
	if err := installWeeklyMembership(context.Background(), connection, ids); err == nil || !hasRejected || hasCanonicalAnalyze || insertAuthorizations != 2 {
		t.Fatal("late membership failure was not induced or modified canonical statistics", insertAuthorizations, hasCanonicalAnalyze, err)
	}
	var count int
	if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM temp.weekly_membership").Scan(&count); err != nil || count != 0 {
		t.Fatal("partial membership survived rollback", count, err)
	}
	if _, err := connection.ExecContext(context.Background(), "BEGIN; ROLLBACK;"); err != nil {
		t.Fatal("failed install left an active transaction", err)
	}
	if err := connection.Raw(func(raw any) error { raw.(*native.SQLiteConn).RegisterAuthorizer(nil); return nil }); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(context.Background(), "DROP TABLE temp.weekly_membership"); err != nil {
		t.Fatal(err)
	}
	if err := installWeeklyMembership(context.Background(), connection, ids); err != nil {
		t.Fatal("failed connection could not install fresh membership", err)
	}
	if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM temp.weekly_membership").Scan(&count); err != nil || count != len(ids) {
		t.Fatal("retry retained an incomplete membership", count, err)
	}
}

// TestWeeklyLargeMembershipPreservesCursorPaging keeps unavailable IDs out of pages and retains NULL-date cursors.
func TestWeeklyLargeMembershipPreservesCursorPaging(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec("UPDATE articles SET journal_id=1 WHERE article_id=1012; UPDATE article_listing SET journal_id=1 WHERE article_id=1012"); err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, 1001)
	for index := range ids {
		ids[index] = int64(index + 1)
	}
	ids = append(ids, 1012, 1001, 1012)
	writeQueryManifest(t, configuration, "catalog.sqlite", ids)
	params := WeeklyArticlePageParams{DbName: "catalog.sqlite", JournalId: 1, WindowEnd: "2026-10-04T00:00:00Z", Limit: 1}
	first, err := WeeklyArticles(context.Background(), configuration, params, nil)
	if err != nil || len(first.Items) != 1 || first.Items[0].ArticleId != 1012 || first.Page.HasMore == nil || !*first.Page.HasMore || first.Page.NextCursor == nil {
		t.Fatal("large membership first page changed", first, err)
	}
	params.Cursor = first.Page.NextCursor
	second, err := WeeklyArticles(context.Background(), configuration, params, nil)
	if err != nil || len(second.Items) != 1 || second.Items[0].ArticleId != 1001 || second.Items[0].Date != nil || second.Page.HasMore == nil || *second.Page.HasMore || second.Page.NextCursor != nil {
		t.Fatal("large membership NULL-date page changed", second, err)
	}
}
