package query

import (
	"context"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
)

// TestSearchTelemetryCoversMatchEntryPoints verifies shared hooks for HTTP and MCP callers.
func TestSearchTelemetryCoversMatchEntryPoints(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec(`UPDATE articles SET title='shared private research' WHERE article_id IN (1001,1012)`); err != nil {
		t.Fatal(err)
	}
	rebuildQueryFixtureSearch(t, database)
	collector := telemetry.New()
	ctx := telemetry.WithCollector(context.Background(), collector)
	text := "shared"
	params := DefaultArticleListParams()
	params.Query = &text
	if _, err := ListArticles(ctx, configuration, nil, params); err != nil {
		t.Fatal(err)
	}
	text = "shared AND"
	params.SearchMode = domain.SearchAdvanced
	if _, err := ListArticles(ctx, configuration, nil, params); err == nil {
		t.Fatal("expected malformed MATCH")
	}
	text = "shared"
	writeQueryManifest(t, configuration, "catalog.sqlite", []int64{1001, 1012})
	if _, err := WeeklyArticles(ctx, configuration, WeeklyArticlePageParams{WindowEnd: "2026-10-04T00:00:00Z", DBName: "catalog.sqlite", JournalID: 1, Limit: 10, Query: &text}, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := RelatedArticles(ctx, configuration, nil, 1001, 10); err != nil {
		t.Fatal(err)
	}
	params.Query = nil
	if _, err := ListArticles(ctx, configuration, nil, params); err != nil {
		t.Fatal(err)
	}
	now := time.Now().UTC().Truncate(time.Hour)
	report, err := collector.Read(ctx, now, now.Add(time.Hour), "search")
	if err != nil || len(report.Series) != 4 {
		t.Fatal(report, err)
	}
	calls, errors := uint64(0), uint64(0)
	names := map[string]bool{}
	for _, series := range report.Series {
		calls += series.Calls
		errors += series.Errors
		names[series.Name] = true
		if series.Database != "catalog.sqlite" {
			t.Fatal(series)
		}
	}
	if calls != 4 || errors != 1 || len(names) != 3 || len(report.SearchShapes) == 0 {
		t.Fatal(report)
	}
}
