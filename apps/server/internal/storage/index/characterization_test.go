package index

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
)

// TestCanonicalAuthorDecodingRejectsInvalidFormats checks persisted data before mutation.
func TestCanonicalAuthorDecodingRejectsInvalidFormats(t *testing.T) {
	for _, text := range []string{`[{"other":!}]`, `[{"display_name":"kept","display_name":!}]`, `[["old tuple"]]`, `null`, `[{"display_name":null}]`} {
		if authors, err := decodeCanonicalAuthors(text); err == nil || authors != nil {
			t.Fatal(text, authors, err)
		}
	}
	if authors, err := decodeCanonicalAuthors(`[{"display_name":"作者"}]`); err != nil || len(authors) != 1 || authors[0].DisplayName != "作者" {
		t.Fatal(authors, err)
	}
}

func TestResolvedDraftPublicationFallbackAndOwnership(t *testing.T) {
	left := domain.ArticleDraft{CatalogID: "catalog", Title: "left", PublicationYear: new(int64(2025)), Date: new("2025-12-01"), Authors: []domain.ArticleAuthorDraft{{DisplayName: "A"}}, RetractionDois: []string{"x"}}
	right := domain.ArticleDraft{CatalogID: "catalog", Title: "right", PublicationYear: new(int64(2026))}
	merged, err := MergeResolvedArticleDrafts(left, right)
	if err != nil || merged.Date != nil || merged.PublicationYear == nil || *merged.PublicationYear != 2026 {
		t.Fatalf("incompatible fallback admitted: %+v %v", merged, err)
	}
	merged.Authors[0].DisplayName = "mutated"
	merged.RetractionDois[0] = "mutated"
	*merged.PublicationYear = 2000
	assertDraftMergeInputsUnchanged(t, left, right)
	empty, err := MergeArticleDrafts(domain.ArticleDraft{}, domain.ArticleDraft{})
	if err != nil || empty.Authors == nil || empty.RetractionDois == nil {
		t.Fatalf("empty collections lost representation: %+v %v", empty, err)
	}
}

func assertDraftMergeInputsUnchanged(t *testing.T, left, right domain.ArticleDraft) {
	t.Helper()
	if left.Authors[0].DisplayName != "A" || left.RetractionDois[0] != "x" || *right.PublicationYear != 2026 {
		t.Fatal("merge result aliases inputs")
	}
}

func TestLateAliasFailureRollsBackContentProjectionsEventsAndCounters(t *testing.T) {
	ctx := context.Background()
	content, err := OpenContent(ctx, filepath.Join(t.TempDir(), "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := content.Close(); err != nil {
			t.Errorf("content.Close: %v", err)
		}
	}()
	catalog, batch := contentInput(t)
	if _, err := content.ExecContext(ctx, "CREATE TRIGGER reject_alias BEFORE INSERT ON article_identity_keys BEGIN SELECT RAISE(ABORT,'late alias failure'); END"); err != nil {
		t.Fatal(err)
	}
	before := selectedSnapshot(t, content.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
	outcome, err := WriteContentBatch(ctx, content.Conn, catalog, batch, "revision", "epoch")
	if err == nil || !strings.Contains(err.Error(), "late alias failure") || outcome != (ContentWriteOutcome{}) {
		t.Fatalf("outcome=%+v error=%v", outcome, err)
	}
	after := selectedSnapshot(t, content.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
	if !reflect.DeepEqual(before, after) {
		t.Fatalf("late failure leaked durable writes: before=%v after=%v", before, after)
	}
}

func TestHistoryPruningReturnsEarlierRemovalOnMalformedLaterFile(t *testing.T) {
	directory := t.TempDir()
	first := filepath.Join(directory, strings.Repeat("0", 64)+".changes.json")
	last := filepath.Join(directory, strings.Repeat("f", 64)+".changes.json")
	if err := os.WriteFile(first, []byte("{\"generated_at\":\"1970-01-01T00:01:39Z\"}"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(last, []byte("invalid JSON"), 0600); err != nil {
		t.Fatal(err)
	}
	removed, err := PruneContentChangeHistory(directory, 100)
	if removed != 1 || err == nil || err.Error() != "invalid change history JSON" {
		t.Fatalf("removed=%d error=%v", removed, err)
	}
	if _, err := os.Stat(first); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("earlier removal lost: %v", err)
	}
	if _, err := os.Stat(last); err != nil {
		t.Fatalf("malformed file removed: %v", err)
	}
}

func TestBatchCatalogValidationPrecedesSelectionAndMode(t *testing.T) {
	catalog := CatalogInput{Filename: "catalog.csv", CatalogName: "catalog", ProviderName: "scholarly", CsvSha256: strings.Repeat("0", 64)}
	catalogs := []CatalogInput{catalog, catalog}
	_, err := NewBatchRequest(catalogs, "invalid", domain.IndexSyncMode("invalid"), 1, false, false)
	if err == nil || !strings.Contains(err.Error(), "filenames must be unique") {
		t.Fatalf("wrong first error: %v", err)
	}
	_, err = NewBatchRequest(catalogs, "explicit_file", domain.Incremental, 0, false, false)
	if err == nil || !strings.Contains(err.Error(), "exactly one catalog") {
		t.Fatalf("cardinality lost precedence: %v", err)
	}
}
