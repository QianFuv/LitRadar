package weekly

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/config"
)

// TestManifestAdmissionPreservesSourceValues distinguishes ignored publications from malformed fields.
func TestManifestAdmissionPreservesSourceValues(t *testing.T) {
	cases := []struct {
		body    string
		ids     []int64
		run     string
		invalid bool
	}{
		{`{"db_name":"catalog","run_id":" source ","generated_at":"2026-10-03T12:00:00Z","notifiable_article_ids":[0,-1,-0,1,1,1.0,1e0,9223372036854775808]}`, []int64{0, -1, 1}, " source ", false},
		{`["catalog","2026-10-03T12:00:00Z"," source ",[1]]`, []int64{1}, " source ", false},
		{`{"db_name":null,"generated_at":false}`, nil, "", true},
		{`{"db_name":null,"notifiable_article_ids":[1]}`, nil, "", false},
		{`{"db_name":"catalog","generated_at":"invalid","run_id":"2026-10-03T12:00:00Z","notifiable_article_ids":[1]}`, nil, "", false},
		{`{"db_name":"catalog","db_name":"catalog"}`, nil, "", true},
		{`{"ignored":1,"ignored":2}`, nil, "", false},
	}
	for _, item := range cases {
		manifest, err := ParseManifest([]byte(item.body))
		if errors.Is(err, ErrManifestJson) != item.invalid {
			t.Fatalf("admission: %s %v", item.body, err)
		}
		assertManifestSource(t, manifest, item.ids, item.run)
	}
}

// assertManifestSource checks ignored results and retained untrimmed publication identity.
func assertManifestSource(t *testing.T, manifest *Manifest, ids []int64, run string) {
	t.Helper()
	if ids == nil {
		if manifest != nil {
			t.Fatalf("ignored publication admitted: %+v", manifest)
		}
		return
	}
	if manifest == nil || !reflect.DeepEqual(manifest.ArticleIds, ids) || manifest.RunId == nil || *manifest.RunId != run {
		t.Fatalf("source values: %+v", manifest)
	}
}

// TestTimestampGrammarRetainsLeapSecondsAndOffsetSign checks exact fractional and timezone grammar.
func TestTimestampGrammarRetainsLeapSecondsAndOffsetSign(t *testing.T) {
	cases := []struct {
		input   string
		seconds int64
		nanos   uint32
		valid   bool
	}{
		{" 1970-01-01t00:00:60.12345678999z ", 59, 1123456789, true},
		{"1970-01-01 00:00:00−00:01", 60, 0, true},
		{"1970-01-01T00:00:00+00:01", -60, 0, true},
		{"1970-01-01T00:00:00.Z", 0, 0, false},
		{"2026-02-29T00:00:00Z", 0, 0, false},
		{"1970-01-01T00:00:00+24:00", 0, 0, false},
	}
	for _, item := range cases {
		stamp, valid := ParseTimestamp(item.input)
		if valid != item.valid || stamp != (Timestamp{item.seconds, item.nanos}) {
			t.Fatalf("timestamp %q: %+v %v", item.input, stamp, valid)
		}
	}
}

// TestAvailableRunIdentityPrecedesPruningAndSelection checks immutable fallback IDs and global parse admission.
func TestAvailableRunIdentityPrecedesPruningAndSelection(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	database, err := platform.Open(platform.Config{Filename: filepath.Join(configuration.IndexDir, "catalog.sqlite"), Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	directory, end := availableSourceFixture(t, configuration, database)
	first, err := LoadAvailable(context.Background(), configuration, end, nil)
	if err != nil {
		t.Fatal(err)
	}
	assertAvailablePublication(t, first, []int64{7})
	if _, err := database.Exec("INSERT INTO journals VALUES(999); INSERT INTO article_listing VALUES(999,1)"); err != nil {
		t.Fatal(err)
	}
	second, err := LoadAvailable(context.Background(), configuration, end, []string{"catalog.sqlite"})
	if err != nil {
		t.Fatal(err)
	}
	assertAvailablePublication(t, second, []int64{7, 8, 999})
	if *first[0].RunId != *second[0].RunId {
		t.Fatal("availability changed immutable run ID")
	}
	if err := os.WriteFile(filepath.Join(directory, "unrelated.changes.json"), []byte("{broken"), 0600); err != nil {
		t.Fatal(err)
	}
	if result, err := LoadAvailable(context.Background(), configuration, end, []string{"missing.sqlite"}); err == nil || result != nil {
		t.Fatalf("selection masked malformed source: %+v %v", result, err)
	}
}

// assertAvailablePublication checks ordered pruning and synthesized source identity.
func assertAvailablePublication(t *testing.T, manifests []Manifest, ids []int64) {
	t.Helper()
	if len(manifests) != 1 || !reflect.DeepEqual(manifests[0].ArticleIds, ids) || manifests[0].RunId == nil || !strings.HasPrefix(*manifests[0].RunId, "weekly-") {
		t.Fatalf("available: %+v", manifests)
	}
}

// availableSourceFixture initializes original membership before availability mutations.
func availableSourceFixture(t *testing.T, configuration config.Config, database *sql.DB) (string, Timestamp) {
	t.Helper()
	if _, err := database.Exec(`CREATE TABLE journals(journal_id INTEGER PRIMARY KEY); CREATE TABLE article_listing(article_id INTEGER,journal_id INTEGER); INSERT INTO journals VALUES(1); INSERT INTO article_listing VALUES(7,1),(8,999);`); err != nil {
		t.Fatal(err)
	}
	directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	body := `{"db_name":"catalog.sqlite","generated_at":"2026-10-03T12:00:00.123456789Z","notifiable_article_ids":[7,8,999]}`
	filename := filepath.Join(directory, "catalog.changes.json")
	if err := os.WriteFile(filename, []byte(body), 0600); err != nil {
		t.Fatal(err)
	}
	end, _ := ParseTimestamp("2026-10-03T12:00:00.123456789Z")
	return directory, end
}
