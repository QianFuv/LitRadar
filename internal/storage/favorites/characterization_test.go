package favorites

import (
	"context"
	"database/sql"
	"encoding/base64"
	"errors"
	"math"
	"path/filepath"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// TestFavoriteCursorRetainsExactFiniteTimestampBits checks accepted signed zero and subnormal anchors.
func TestFavoriteCursorRetainsExactFiniteTimestampBits(t *testing.T) {
	for _, item := range []struct {
		raw  string
		bits uint64
	}{{"1|+1|02|8000000000000000|+3", 0x8000000000000000}, {"1|1|2|+0000000000000001|3", 1}} {
		encoded := base64.RawURLEncoding.EncodeToString([]byte(item.raw))
		stamp, identifier, err := decodeCursor(encoded, 1, 2)
		if err != nil || identifier != 3 || math.Float64bits(stamp) != item.bits {
			t.Fatalf("cursor bits changed: %s %016x %d %v", item.raw, math.Float64bits(stamp), identifier, err)
		}
	}
}

// TestSafeMetadataNamesPreserveByteGrammar checks logging redaction independently of runtime catalog rules.
func TestSafeMetadataNamesPreserveByteGrammar(t *testing.T) {
	cases := []struct{ value, expected string }{
		{" ", "default"}, {"a", "a"}, {" 0._- ", "0._-"}, {"_a", "invalid"}, {"A", "invalid"},
		{"private/path", "invalid"}, {"中文", "invalid"}, {strings.Repeat("a", 255), strings.Repeat("a", 255)}, {strings.Repeat("a", 256), "invalid"},
	}
	for _, item := range cases {
		if actual := safeName(item.value); actual != item.expected {
			t.Fatalf("safe name %q: %q", item.value, actual)
		}
	}
}

// favoriteMetadataFixture creates an actual native catalog without introducing auth ownership.
func favoriteMetadataFixture(t *testing.T) (config.Config, *sql.DB, string) {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	installMetadata(t, configuration)
	filename := filepath.Join(configuration.IndexDir, "metadata.sqlite")
	database, err := storage.OpenPlain(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	return configuration, database, filename
}

// TestMetadataEmptyAdmissionAndDuplicatePointerSharing checks group projections and row-local ownership.
func TestMetadataEmptyAdmissionAndDuplicatePointerSharing(t *testing.T) {
	configuration, _, _ := favoriteMetadataFixture(t)
	canceled, cancel := context.WithCancel(context.Background())
	cancel()
	empty, err := loadMetadata(canceled, "missing", "original", []int64{0, -1})
	if err != nil || empty == nil || len(empty) != 0 {
		t.Fatalf("empty IDs touched storage: %#v %v", empty, err)
	}
	favorites := []Favorite{{Reference: Reference{1001, "metadata"}}, {Reference: Reference{1001, "metadata"}}, {Reference: Reference{1012, "metadata"}}}
	items := Enrich(context.Background(), configuration, favorites)
	assertMetadataPointerOwnership(t, items)
}

// assertMetadataPointerOwnership distinguishes duplicate-reference sharing from separate canonical rows.
func assertMetadataPointerOwnership(t *testing.T, items []Article) {
	t.Helper()
	if len(items) != 3 || items[0].MetadataStatus != "available" || items[2].MetadataStatus != "available" {
		t.Fatalf("metadata unavailable: %#v", items)
	}
	if items[0].Authors != items[1].Authors || items[0].Title != items[1].Title {
		t.Fatal("duplicate metadata stopped sharing pointers")
	}
	if items[0].Authors == items[2].Authors || items[0].JournalId == items[2].JournalId {
		t.Fatal("distinct rows share mutable pointers")
	}
}

// TestLateMetadataChunkFailureDiscardsPriorGroupRows checks failure admission after the 500-row boundary.
func TestLateMetadataChunkFailureDiscardsPriorGroupRows(t *testing.T) {
	configuration, database, filename := favoriteMetadataFixture(t)
	if _, err := database.Exec(`WITH RECURSIVE numbers(id) AS (VALUES(2000) UNION ALL SELECT id+1 FROM numbers WHERE id<2500)
 INSERT INTO articles(article_id,journal_id,title,authors_json) SELECT id,1,'chunk',CASE WHEN id=2500 THEN 'null' ELSE '[]' END FROM numbers;`); err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, 501)
	favorites := make([]Favorite, 501)
	for index := range ids {
		ids[index] = int64(index + 2000)
		favorites[index] = Favorite{Reference: Reference{identity.Id(ids[index]), "metadata"}}
	}
	loaded, err := loadMetadata(context.Background(), filename, "metadata", ids)
	if loaded != nil || !errors.Is(err, search.ErrInvalidAuthors) {
		t.Fatalf("partial metadata admitted: %d %v", len(loaded), err)
	}
	items := Enrich(context.Background(), configuration, favorites)
	for _, item := range items {
		if item.MetadataStatus != "unavailable" || item.Title != nil {
			t.Fatalf("prior chunk survived failed group: %#v", item)
		}
	}
}
