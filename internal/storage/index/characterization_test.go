package index

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

func TestAuthorFieldErrorsPrecedeMalformedValuesAndRetainPrivateState(t *testing.T) {
	for _, text := range []string{
		"{" + "\"other\":!}",
		"{" + "\"display_name\":\"kept\",\"display_name\":!}",
	} {
		decoder := authorDecoder{text: text}
		author, err := decoder.author()
		var typed *AuthorJsonError
		if !errors.As(err, &typed) {
			t.Fatalf("missing typed diagnostic: %v", err)
		}
		expectedName := ""
		expectedMessage := "unknown field"
		if strings.Contains(text, "kept") {
			expectedName, expectedMessage = "kept", "duplicate field"
		}
		if author.DisplayName != expectedName || !strings.HasPrefix(typed.Message, expectedMessage) {
			t.Fatalf("name=%q diagnostic=%v", author.DisplayName, err)
		}
		if decoder.position != strings.LastIndex(text, ":") || typed.Column != decoder.position {
			t.Fatalf("advanced beyond key: position=%d diagnostic=%v", decoder.position, err)
		}
		values, err := diagnoseCanonicalAuthors("[" + text + "]")
		if err == nil || values != nil {
			t.Fatalf("list exposed partial authors: %v %v", values, err)
		}
	}
}

func TestAuthorDiagnosticLocationsCountUtf8Bytes(t *testing.T) {
	text := "[\n{\"display_name\":\"作者\",\"other\":!}]"
	_, err := diagnoseCanonicalAuthors(text)
	var typed *AuthorJsonError
	if !errors.As(err, &typed) {
		t.Fatalf("diagnostic=%v", err)
	}
	keyEnd := strings.LastIndex(text, ":")
	if typed.Line != 2 || typed.Column != keyEnd-strings.IndexByte(text, '\n')-1 {
		t.Fatalf("expected byte location: %v", err)
	}
}

func TestResolvedDraftPublicationFallbackAndOwnership(t *testing.T) {
	left := domain.ArticleDraft{CatalogId: "catalog", Title: "left", PublicationYear: ptr(int64(2025)), Date: ptr("2025-12-01"), Authors: []domain.ArticleAuthorDraft{{DisplayName: "A"}}, RetractionDois: []string{"x"}}
	right := domain.ArticleDraft{CatalogId: "catalog", Title: "right", PublicationYear: ptr(int64(2026))}
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
	defer content.Close()
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
	if err := os.WriteFile(first, []byte("{\"generated_at\":\" 99 \"}"), 0600); err != nil {
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
