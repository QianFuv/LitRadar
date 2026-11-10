package telemetry

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// TestAggregationMergesHoursBeforePercentiles checks UTC boundaries and nearest-rank estimates.
func TestAggregationMergesHoursBeforePercentiles(t *testing.T) {
	collector := New()
	start := time.Date(2026, 11, 10, 10, 0, 0, 0, time.UTC)
	shape := Dimensions{Kind: "mcp", Name: "search_articles"}
	for index := range 100 {
		collector.Record(start.Add(time.Duration(index%2)*time.Hour), shape, time.Duration(index+1)*time.Microsecond)
	}
	collector.Record(start.Add(-time.Nanosecond), Dimensions{Kind: "http", Name: "/api/articles/{id}", IsError: true}, time.Millisecond)
	report, err := collector.Read(context.Background(), start, start.Add(2*time.Hour), "")
	if err != nil || len(report.Series) != 1 || len(report.Hourly) != 2 {
		t.Fatal(report, err)
	}
	series := report.Series[0]
	if series.Calls != 100 || series.P50Ms != 0.064 || series.P95Ms != 0.128 || series.P99Ms != 0.128 {
		t.Fatal(series)
	}
	local := start.In(time.FixedZone("local", 8*3600))
	collector.Record(local, shape, time.Millisecond)
	if len(collector.pending) != 3 {
		t.Fatal("UTC hour mismatch", collector.pending)
	}
}

// openCollector supplies an isolated, migrated telemetry database.
func openCollector(t *testing.T) *Collector {
	t.Helper()
	collector, err := Open(context.Background(), filepath.Join(t.TempDir(), "telemetry.sqlite"), 90)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(collector.Close)
	return collector
}

// TestFlushMergePendingRetentionAndReopen checks additive deltas and empty-flush idempotency.
func TestFlushMergePendingRetentionAndReopen(t *testing.T) {
	collector := openCollector(t)
	ctx, now := context.Background(), time.Now().UTC().Truncate(time.Hour)
	shape := Dimensions{Kind: "http", Name: "/api/articles/{id}", Method: "GET", StatusClass: "5xx", IsError: true}
	collector.Record(now, shape, time.Millisecond)
	collector.Record(now.AddDate(0, 0, -91), shape, time.Millisecond)
	if err := collector.Flush(ctx, now.AddDate(0, 0, -2)); err != nil {
		t.Fatal(err)
	}
	if err := collector.Flush(ctx, now); err != nil {
		t.Fatal(err)
	}
	collector.Record(now, shape, 2*time.Millisecond)
	report, err := collector.Read(ctx, now, now.Add(time.Hour), "http")
	if err != nil || len(report.Series) != 1 || report.Series[0].Calls != 2 || report.Series[0].Errors != 2 || report.Series[0].ErrorRate != 1 {
		t.Fatal(report, err)
	}
	if err := collector.Flush(ctx, now); err != nil {
		t.Fatal(err)
	}
	if err := collector.Flush(ctx, now); err != nil {
		t.Fatal(err)
	}
	var count int
	if err := collector.store.database.QueryRow("SELECT COUNT(*) FROM hourly_load").Scan(&count); err != nil || count != 1 {
		t.Fatal(count, err)
	}
	other, err := Open(ctx, collectorFilename(t, collector), 90)
	if err != nil {
		t.Fatal(err)
	}
	defer other.Close()
	report, err = other.Read(ctx, now, now.Add(time.Hour), "http")
	if err != nil || report.Series[0].Calls != 2 || report.Series[0].P95Ms != 2.048 {
		t.Fatal(report, err)
	}
}

// collectorFilename resolves only the test-owned database for the reopen assertion.
func collectorFilename(t *testing.T, collector *Collector) string {
	t.Helper()
	var sequence int
	var name, filename string
	if err := collector.store.database.QueryRow("PRAGMA database_list").Scan(&sequence, &name, &filename); err != nil {
		t.Fatal(err)
	}
	return filename
}

// TestDisabledAndBoundedRecording ensures disabled hooks and contention cannot stall requests.
func TestDisabledAndBoundedRecording(t *testing.T) {
	var disabled *Collector
	disabled.Http("/api/articles/{id}", "GET", 200, time.Millisecond)
	disabled.Mcp("search_articles", false, time.Millisecond)
	report, err := disabled.Read(context.Background(), time.Now(), time.Now(), "")
	if err != nil || report.Enabled || report.Series == nil {
		t.Fatal(report, err)
	}
	collector := New()
	collector.mutex.Lock()
	collector.Record(time.Now(), Dimensions{Kind: "http"}, time.Second)
	collector.mutex.Unlock()
	if len(collector.pending) != 0 {
		t.Fatal("contended observation was not dropped")
	}
	for index := range seriesLimit + 1 {
		collector.Record(time.Unix(int64(index)*3600, 0), Dimensions{Kind: "http"}, time.Second)
	}
	if len(collector.pending) != seriesLimit {
		t.Fatal(len(collector.pending))
	}
}

// TestConcurrentRecordingAndSnapshots checks histogram consistency under request and flush concurrency.
func TestConcurrentRecordingAndSnapshots(t *testing.T) {
	collector := openCollector(t)
	var workers sync.WaitGroup
	for range 8 {
		workers.Go(func() {
			for range 1000 {
				collector.Http("/api/articles", "GET", 200, time.Millisecond)
			}
		})
	}
	for range 5 {
		if err := collector.Flush(context.Background(), time.Now()); err != nil {
			t.Fatal(err)
		}
	}
	workers.Wait()
	now := time.Now().UTC().Truncate(time.Hour)
	report, err := collector.Read(context.Background(), now, now.Add(time.Hour), "")
	if err != nil || len(report.Series) != 1 || report.Series[0].Calls == 0 || report.Series[0].Calls > 8000 || report.Series[0].P99Ms != 1.024 {
		t.Fatal(report, err)
	}
}

// TestSearchPrivacyInStoredRows verifies that query text and identity cannot enter persisted dimensions.
func TestSearchPrivacyInStoredRows(t *testing.T) {
	collector := openCollector(t)
	secret := "private_search_canary"
	shape := SearchShape("articles", "advanced", "catalog.sqlite", secret+`* AND "private user identity"`)
	shape.Hits = Magnitude(18)
	collector.Record(time.Now(), shape, time.Millisecond)
	if err := collector.Flush(context.Background(), time.Now()); err != nil {
		t.Fatal(err)
	}
	var labels, encoded string
	if err := collector.store.database.QueryRow("SELECT dimensions,aggregate FROM hourly_load").Scan(&labels, &encoded); err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{secret, "private", "identity", "username", "user_id", "token", "query", "ip_address"} {
		if strings.Contains(labels+encoded, forbidden) {
			t.Fatal("private value reached storage", forbidden)
		}
	}
	var stored Dimensions
	if err := json.Unmarshal([]byte(labels), &stored); err != nil || !stored.UsesBoolean || !stored.UsesPrefix || stored.Hits != "10-99" {
		t.Fatal(stored, err)
	}
}

// TestSearchShapeSemantics distinguishes literal simple text and quoted advanced operators.
func TestSearchShapeSemantics(t *testing.T) {
	for _, scenario := range []struct {
		text, mode, terms string
		prefix, boolean   bool
	}{
		{`alpha AND beta*`, "simple", "2-4", false, false},
		{`"AND star*"`, "advanced", "1", false, false},
		{`"AND"`, "advanced", "1", false, false},
		{`title:alpha`, "advanced", "1", false, false},
		{`NEAR(alpha beta) OR gamma*`, "advanced", "2-4", true, true},
	} {
		shape := SearchShape("articles", scenario.mode, "catalog.sqlite", scenario.text)
		if shape.TermCount != scenario.terms || shape.UsesPrefix != scenario.prefix || shape.UsesBoolean != scenario.boolean {
			t.Fatal(shape)
		}
	}
	for _, scenario := range []struct {
		count int64
		want  string
	}{{0, "0"}, {1, "1-9"}, {9, "1-9"}, {10, "10-99"}, {99, "10-99"}, {100, "100-999"}, {999, "100-999"}, {1000, "1000+"}} {
		if Magnitude(scenario.count) != scenario.want {
			t.Fatal(scenario)
		}
	}
}

// TestFailedFlushDropsBatch prevents retries from double-counting a failed transaction.
func TestFailedFlushDropsBatch(t *testing.T) {
	collector := openCollector(t)
	collector.Http("/api/articles", "GET", 200, time.Millisecond)
	if _, err := collector.store.database.Exec("DROP TABLE hourly_load"); err != nil {
		t.Fatal(err)
	}
	if err := collector.Flush(context.Background(), time.Now()); err == nil || len(collector.pending) != 0 {
		t.Fatal("failed batch retained", err)
	}
}

// BenchmarkRecord measures the complete normalized HTTP recording hot path.
func BenchmarkRecord(b *testing.B) {
	for _, isEnabled := range []bool{false, true} {
		name := "disabled"
		var collector *Collector
		if isEnabled {
			name = "enabled"
			collector = New()
		}
		b.Run(name, func(b *testing.B) {
			b.ReportAllocs()
			for b.Loop() {
				collector.Http("/api/articles/{article_id}", "GET", 200, time.Millisecond)
			}
		})
	}
}

// TestHistogramRoundsSubMicrosecondRemaindersUp preserves percentile upper bounds.
func TestHistogramRoundsSubMicrosecondRemaindersUp(t *testing.T) {
	collector := New()
	collector.Record(time.Now(), Dimensions{Kind: "http"}, time.Microsecond+time.Nanosecond)
	for _, value := range collector.pending {
		if value.percentile(50) != 0.002 {
			t.Fatal(value)
		}
	}
}
