package api

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/index"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

func TestCfpMatchesOriginalResponsesAndReadsOriginalCursor(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/api/cfp-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	type observed struct {
		Url           string
		Authenticated bool
		Status        int
		Body          string
	}
	var corpus struct {
		ExporterSha256       string `json:"exporter_sha256"`
		CatalogCsv           string `json:"catalog_csv"`
		Snapshot             storage.JournalSnapshot
		Catalog, First, Next observed
		Errors               []observed
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	exporter, err := os.ReadFile("../../tests/migration/api/export-cfp.mjs")
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(exporter)
	if hex.EncodeToString(digest[:]) != corpus.ExporterSha256 {
		t.Fatal("stale original CFP observations")
	}
	entries, err := index.ParseCatalogCsv(corpus.CatalogCsv)
	if err != nil {
		t.Fatal(err)
	}
	handlers, router, _ := cfpFixture(t)
	var expectedCatalog cfpCatalogResponse
	if err := json.Unmarshal([]byte(corpus.Catalog.Body), &expectedCatalog); err != nil {
		t.Fatal(err)
	}
	catalog, failure := cfpCatalog("cfp_wire.sqlite", nil, entries, []storage.JournalSnapshot{corpus.Snapshot}, expectedCatalog.EvaluatedAt)
	if failure != nil {
		t.Fatal(failure)
	}
	encoded, err := auth.EncodeJson(catalog)
	if err != nil || encoded != corpus.Catalog.Body {
		t.Fatal("original catalog differs", encoded, corpus.Catalog.Body, err)
	}
	var first struct {
		EvaluatedAt int64 `json:"evaluatedAt"`
		Page        struct {
			NextCursor *string `json:"next_cursor"`
		} `json:"page"`
	}
	if err := json.Unmarshal([]byte(corpus.First.Body), &first); err != nil || first.Page.NextCursor == nil {
		t.Fatal(corpus.First.Body, err)
	}
	page, failure := cfpPage(handlers.codec, "cfp_wire.sqlite", entries[0], &corpus.Snapshot, true, nil, 1, first.EvaluatedAt)
	if failure != nil || page.Page.NextCursor == nil {
		t.Fatal(page, failure)
	}
	ours, err := handlers.codec.Decrypt(*page.Page.NextCursor, cfpCursorContext)
	if err != nil {
		t.Fatal(err)
	}
	original, err := handlers.codec.Decrypt(*first.Page.NextCursor, cfpCursorContext)
	if err != nil || ours != original {
		t.Fatal("cursor plaintext/revision differs", ours, original, err)
	}
	page.Page.NextCursor = first.Page.NextCursor
	encoded, err = auth.EncodeJson(page)
	if err != nil || encoded != corpus.First.Body {
		t.Fatal("original first page differs", encoded, corpus.First.Body, err)
	}
	page, failure = cfpPage(handlers.codec, "cfp_wire.sqlite", entries[0], &corpus.Snapshot, true, first.Page.NextCursor, 200, first.EvaluatedAt)
	if failure != nil {
		t.Fatal("original cursor rejected", failure)
	}
	encoded, err = auth.EncodeJson(page)
	if err != nil || encoded != corpus.Next.Body {
		t.Fatal("original continuation differs", encoded, corpus.Next.Body, err)
	}
	for _, observed := range corpus.Errors {
		response := authRequest(router, "GET", observed.Url, "", "")
		if response.Code != observed.Status || response.Body.String() != observed.Body {
			t.Fatal(observed.Url, response.Code, response.Body.String(), "expected", observed.Status, observed.Body)
		}
	}
}

func cfpFixture(t *testing.T) (*cfpHandlers, *http.ServeMux, string) {
	t.Helper()
	auth, router, token := authFixture(t)
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.MetaDir, 0700); err != nil {
		t.Fatal(err)
	}
	if _, err := migration.Migrate(context.Background(), configuration.AuthDbPath); err != nil {
		t.Fatal(err)
	}
	repository, err := storage.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	entries := "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating\n"
	for _, row := range [][3]string{{"cfp-fixture", "cfp-alias", "Example Journal"}, {"unadapted", "", "Unadapted Journal"}} {
		fields := make([]string, 16)
		fields[0], fields[1], fields[2], fields[7] = row[0], row[1], row[2], "Test subject"
		entries += strings.Join(fields, ",") + "\n"
	}
	if err := os.WriteFile(filepath.Join(configuration.MetaDir, "fixture.csv"), []byte(entries), 0600); err != nil {
		t.Fatal(err)
	}
	seed := `{"formatVersion":1,"sources":[{"catalogIds":["cfp-fixture","cfp-alias"],"journalTitle":"Example Journal","title":"Open original title","typeText":"Special Issue","dateText":"Submission deadline: 31 December 2099","sourceUrl":"https://example.org/calls","checkedOn":"2026-09-15","timeZone":"UTC"},{"catalogIds":["cfp-fixture","cfp-alias"],"journalTitle":"Example Journal","title":"Another original title","typeText":"Special Issue","dateText":"Submission deadline: 31 December 2099","sourceUrl":"https://example.org/calls","checkedOn":"2026-09-15","timeZone":"UTC"}],"emptyJournals":[]}`
	if _, err := repository.ImportSeed(context.Background(), "api-fixture", []byte(seed)); err != nil {
		t.Fatal(err)
	}
	handlers := &cfpHandlers{configuration, repository, codec, auth.authenticator, auth.pool}
	for _, route := range handlers.routes() {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	return handlers, router, token
}

func TestCfpRoutesReadCatalogWithoutIndexAndContinueAliasCursor(t *testing.T) {
	_, router, token := cfpFixture(t)
	response := authRequest(router, "GET", "/api/cfp/journals?db=fixture.sqlite&q=Example", "", token)
	var catalog cfpCatalogResponse
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &catalog) != nil || catalog.Summary.Journals != 2 || catalog.Summary.AdaptedJournals != 1 || len(catalog.Items) != 1 || catalog.Items[0].NoticeCount != 2 {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "GET", "/api/cfp/journals/cfp-alias/notices?db=fixture.sqlite&limit=1", "", token)
	var first struct {
		EvaluatedAt int64            `json:"evaluatedAt"`
		Items       []map[string]any `json:"items"`
		Page        struct {
			NextCursor *string `json:"next_cursor"`
		} `json:"page"`
	}
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &first) != nil || first.Page.NextCursor == nil || len(first.Items) != 1 || first.Items[0]["state"] != "open" || first.Items[0]["entryDeadline"] == nil {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "GET", "/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&limit=2&cursor="+*first.Page.NextCursor, "", token)
	var second struct {
		EvaluatedAt int64            `json:"evaluatedAt"`
		Items       []map[string]any `json:"items"`
	}
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &second) != nil || len(second.Items) != 1 || second.EvaluatedAt != first.EvaluatedAt || first.Items[0]["id"] == second.Items[0]["id"] {
		t.Fatal(response.Code, response.Body.String())
	}
}

func TestCfpExtractionAuthenticationAndValidationPrecedence(t *testing.T) {
	_, router, token := cfpFixture(t)
	for _, scenario := range []struct {
		url           string
		authenticated bool
		status        int
		detail        string
	}{
		{"/api/cfp/journals", false, 400, "missing field `db`"},
		{"/api/cfp/journals?db=fixture.sqlite&db=fixture.sqlite", false, 400, "duplicate field"},
		{"/api/cfp/journals/x/notices?db=fixture.sqlite&limit=9223372036854775808", false, 401, "Authentication required"},
		{"/api/cfp/journals/x/notices?db=fixture.sqlite&limit=9223372036854775808", true, 400, "between 1 and 200"},
		{"/api/cfp/journals/x/notices?db=fixture.sqlite&limit=18446744073709551616", false, 400, "number too large"},
		{"/api/cfp/journals/x/notices?db=fixture.sqlite&limit=-1", false, 400, "invalid digit"},
		{"/api/cfp/journals/x/notices?db=fixture.sqlite&include_closed=1", false, 400, "not `true` or `false`"},
		{"/api/cfp/journals?db=fixture", true, 404, "database catalog not found"},
		{"/api/cfp/journals?db=%20fixture.sqlite", true, 404, "database catalog not found"},
		{"/api/cfp/journals?db=fixture.sqlite&q=" + strings.Repeat("x", 257), true, 400, "exceeds 256"},
		{"/api/cfp/journals/x/notices?db=fixture.sqlite&cursor=invalid", true, 404, "catalog member not found"},
		{"/api/cfp/journals/cfp-alias/notices?db=fixture.sqlite&cursor=invalid", true, 409, cfpCursorDetail},
	} {
		bearer := ""
		if scenario.authenticated {
			bearer = token
		}
		response := authRequest(router, "GET", scenario.url, "", bearer)
		if response.Code != scenario.status || !strings.Contains(response.Body.String(), scenario.detail) {
			t.Fatal(scenario.url, response.Code, response.Body.String())
		}
	}
}

func TestCfpCursorBindsRevisionFilterAndTimeButNotLimit(t *testing.T) {
	handlers, _, _ := cfpFixture(t)
	entries, failure := handlers.catalogMembers("fixture.sqlite")
	if failure != nil {
		t.Fatal(failure)
	}
	snapshots, err := handlers.repository.LoadJournals(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	snapshot, failure := cfpFindSnapshot(entries[0], snapshots)
	if failure != nil {
		t.Fatal(failure)
	}
	const now = int64(1791000000)
	first, failure := cfpPage(handlers.codec, "fixture.sqlite", entries[0], snapshot, false, nil, 1, now)
	if failure != nil || first.Page.NextCursor == nil {
		t.Fatal(first, failure)
	}
	for _, delta := range []int64{-1, 0, 900, 901} {
		page, failure := cfpPage(handlers.codec, "fixture.sqlite", entries[0], snapshot, false, first.Page.NextCursor, 200, now+delta)
		if delta == -1 || delta == 901 {
			if failure == nil || failure.status != 409 {
				t.Fatal(delta, page, failure)
			}
		} else if failure != nil || len(page.Items) != 1 || page.EvaluatedAt != now {
			t.Fatal(delta, page, failure)
		}
	}
	changed := entries[0]
	changed.Title += " changed"
	for _, scenario := range []struct {
		entry         sources.JournalCatalogEntry
		database      string
		includeClosed bool
	}{{changed, "fixture.sqlite", false}, {entries[0], "other.sqlite", false}, {entries[0], "fixture.sqlite", true}} {
		if _, failure := cfpPage(handlers.codec, scenario.database, scenario.entry, snapshot, scenario.includeClosed, first.Page.NextCursor, 1, now); failure == nil || failure.status != 409 {
			t.Fatal("accepted changed cursor binding", failure)
		}
	}
	cursor, failure := decodeCfpCursor(handlers.codec, *first.Page.NextCursor)
	if failure != nil {
		t.Fatal(failure)
	}
	for _, position := range []uint64{2, 3, ^uint64(0)} {
		cursor.Position = position
		plaintext, _ := auth.EncodeJson(cursor)
		ciphertext, _ := handlers.codec.Encrypt(plaintext, cfpCursorContext)
		page, failure := cfpPage(handlers.codec, "fixture.sqlite", entries[0], snapshot, false, &ciphertext, 1, now)
		if position == 2 {
			if failure != nil || len(page.Items) != 0 {
				t.Fatal(page, failure)
			}
		} else if failure == nil {
			t.Fatal("out-of-range cursor accepted")
		}
	}
	missing, _ := cfpRevision(entries[0], nil)
	empty, _ := cfpRevision(entries[0], &storage.JournalSnapshot{})
	if missing == empty {
		t.Fatal("unadapted and adapted-empty revisions collapsed")
	}
}

func TestCfpSummaryKeepsSourceTiesLeaseClockAndStateOrder(t *testing.T) {
	attempt, expired := int64(10), int64(99)
	entry := sources.JournalCatalogEntry{CatalogId: "x", CatalogAliases: []string{}, AllIssns: []string{}, TitleAliases: []string{}}
	snapshot := storage.JournalSnapshot{CatalogIds: []string{"x"}, Sources: []storage.SourceStatus{{SourceKey: "first", Status: "success", LastAttempt: &attempt}, {SourceKey: "last", Status: "refreshing", LastAttempt: &attempt, LeaseExpiresAt: &expired}}}
	value := cfpSummary(entry, &snapshot, 10, 100)
	if value.Coverage != "adapted" || value.RefreshStatus != "failed" || value.LastError == nil || value.NoticeCount != 0 {
		t.Fatal(value)
	}
	expired = 100
	if value := cfpSummary(entry, &snapshot, 10, 100); value.RefreshStatus != "refreshing" {
		t.Fatal(value)
	}
	if _, failure := cfpFindSnapshot(entry, []storage.JournalSnapshot{snapshot, snapshot}); failure == nil {
		t.Fatal("ambiguous source snapshot accepted")
	}
	encoded, err := auth.EncodeJson(cfpStateCounts{domain.Uncertain: 1, domain.Closed: 2, domain.Open: 3})
	if err != nil || encoded != `{"open":3,"closed":2,"uncertain":1}` {
		t.Fatal(encoded, err)
	}
}

func TestCfpCursorRejectsMalformedAuthenticatedPlaintext(t *testing.T) {
	handlers, _, _ := cfpFixture(t)
	for _, plaintext := range []string{`{}`, `[]`, `null`, `{"database":"a","database":"b"}`, `["x","y",false,"r",1,-1,"source-v1"]`, `["x","y",false,"r",1,0,"source-v1",null]`, `["x","y",false,"r",1,0,"source-v1"] garbage`} {
		ciphertext, err := handlers.codec.Encrypt(plaintext, cfpCursorContext)
		if err != nil {
			t.Fatal(err)
		}
		if _, failure := decodeCfpCursor(handlers.codec, ciphertext); failure == nil {
			t.Fatal("accepted", plaintext)
		}
	}
	for _, plaintext := range []string{`["x","y",false,"r",1,0,"source-v1"]`, `{"database":"x","catalog_id":"y","include_closed":false,"revision":"r","evaluated_at":1,"position":18446744073709551615,"order":"source-v1"}`} {
		ciphertext, _ := handlers.codec.Encrypt(plaintext, cfpCursorContext)
		if _, failure := decodeCfpCursor(handlers.codec, ciphertext); failure != nil {
			t.Fatal(plaintext, failure)
		}
	}
}
