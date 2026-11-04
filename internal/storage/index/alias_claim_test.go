package index

import (
	"context"
	"errors"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// aliasClaimFixture seeds two separately owned bibliographic and DOI identities.
func aliasClaimFixture(t *testing.T) (*Connection, domain.JournalCatalogEntry, domain.ProviderBatch) {
	t.Helper()
	connection, err := OpenContent(context.Background(), filepath.Join(t.TempDir(), "aliases.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	catalog, batch := contentInput(t)
	article := &batch.Articles[0]
	article.Title = "A sufficiently rich original research title"
	article.PublicationYear, article.Volume, article.IssueNumber, article.StartPage = new(int64(2026)), new("1"), new("2"), new("3")
	other := *article
	other.Title, other.Doi, other.StartPage = "Another independently owned article", new("10.1234/other"), new("20")
	batch.Articles = append(batch.Articles, other)
	outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "seed", "epoch")
	if err != nil || outcome.ArticlesChanged != 2 || outcome.IdentityAliasesAdded != 4 {
		t.Fatal("initial identity claims changed", outcome, err)
	}
	return connection, catalog, batch
}

// TestAliasClaimCountsDistinguishReplayAndEnrichment verifies counting without suppressing existing claims.
func TestAliasClaimCountsDistinguishReplayAndEnrichment(t *testing.T) {
	connection, catalog, batch := aliasClaimFixture(t)
	outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "replay", "epoch")
	if err != nil || outcome.ArticlesChanged != 0 || outcome.IdentityAliasesAdded != 0 || outcome.ChangeEventsEmitted != 0 {
		t.Fatal("replay counted existing aliases", outcome, err)
	}
	batch.Articles[0].Pmid = new("12345")
	outcome, err = WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "enrich", "epoch")
	if err != nil || outcome.ArticlesChanged != 1 || outcome.IdentityAliasesAdded != 1 {
		t.Fatal("new PMID claim was lost or double counted", outcome, err)
	}
	outcome, err = WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "replay-enriched", "epoch")
	if err != nil || outcome.ArticlesChanged != 0 || outcome.IdentityAliasesAdded != 0 {
		t.Fatal("enriched aliases were counted twice", outcome, err)
	}
}

// TestExistingAliasClaimsRetainLateFailureAndOwnerValidation rolls back mutations after initial ownership lookup.
func TestExistingAliasClaimsRetainLateFailureAndOwnerValidation(t *testing.T) {
	for _, scenario := range []string{"insert-abort", "update-abort", "reassign"} {
		t.Run(scenario, func(t *testing.T) {
			connection, catalog, batch := aliasClaimFixture(t)
			firstId, otherId := articleAliasOwner(t, connection, batch.Articles[0]), articleAliasOwner(t, connection, batch.Articles[1])
			trigger := `CREATE TRIGGER fail_existing_alias BEFORE INSERT ON article_identity_keys WHEN NEW.identity_kind='doi' BEGIN SELECT RAISE(ABORT,'existing alias failure'); END`
			if scenario == "update-abort" {
				trigger = `CREATE TRIGGER fail_existing_alias BEFORE UPDATE ON article_identity_keys WHEN NEW.identity_kind='doi' BEGIN SELECT RAISE(ABORT,'existing alias failure'); END`
			} else if scenario == "reassign" {
				trigger = `CREATE TRIGGER fail_existing_alias BEFORE INSERT ON article_identity_keys WHEN NEW.identity_kind='doi' BEGIN UPDATE article_identity_keys SET article_id=` + strconv.FormatInt(otherId, 10) + ` WHERE identity_kind=NEW.identity_kind AND identity_value=NEW.identity_value; END`
			}
			if _, err := connection.ExecContext(context.Background(), trigger); err != nil {
				t.Fatal(err)
			}
			batch.Articles[0].StartPage = new("4")
			before := selectedSnapshot(t, connection.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
			outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "update", "epoch")
			var conflict *IdentityConflict
			if scenario == "reassign" {
				if !errors.As(err, &conflict) || !reflect.DeepEqual(conflict.ArticleIds, []int64{otherId, firstId}) {
					t.Fatal("claim trusted stale ownership", err)
				}
			} else if err == nil || !strings.Contains(err.Error(), "existing alias failure") {
				t.Fatal("existing claim skipped its SQL operation", err)
			}
			after := selectedSnapshot(t, connection.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
			if outcome != (ContentWriteOutcome{}) || !reflect.DeepEqual(before, after) {
				t.Fatal("late claim failure leaked writes or counters", outcome)
			}
			if _, err := connection.ExecContext(context.Background(), "DROP TRIGGER fail_existing_alias"); err != nil {
				t.Fatal(err)
			}
			outcome, err = WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "update", "epoch")
			if err != nil || outcome.ArticlesChanged != 1 || outcome.IdentityAliasesAdded != 1 {
				t.Fatal("retry lost a new bibliographic alias", outcome, err)
			}
		})
	}
}

// TestMergedOnlyAliasConflictRollsBack rejects ownership absent from the incoming-key lookup.
func TestMergedOnlyAliasConflictRollsBack(t *testing.T) {
	connection, catalog, batch := aliasClaimFixture(t)
	firstId, otherId := articleAliasOwner(t, connection, batch.Articles[0]), articleAliasOwner(t, connection, batch.Articles[1])
	incoming := batch.Articles[0]
	incoming.Title, incoming.StartPage = "X", new("4")
	merged, err := MergeResolvedArticleDrafts(batch.Articles[0], incoming)
	if err != nil {
		t.Fatal(err)
	}
	key := ArticleIdentityKeys(merged)[1]
	if key.Kind != "bibliographic" {
		t.Fatal("fixture lost its merged-only key", key)
	}
	if _, err := connection.ExecContext(context.Background(), "INSERT INTO article_identity_keys VALUES(?,?,?)", key.Kind, key.Value, otherId); err != nil {
		t.Fatal(err)
	}
	batch.Articles = []domain.ArticleDraft{incoming}
	before := selectedSnapshot(t, connection.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
	outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "conflict", "epoch")
	var conflict *IdentityConflict
	if !errors.As(err, &conflict) || !reflect.DeepEqual(conflict.ArticleIds, []int64{otherId, firstId}) {
		t.Fatal("merged-only alias conflict was not detected", err)
	}
	after := selectedSnapshot(t, connection.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
	if outcome != (ContentWriteOutcome{}) || !reflect.DeepEqual(before, after) {
		t.Fatal("merged-only conflict leaked writes or counters", outcome)
	}
	if _, err := connection.ExecContext(context.Background(), "DELETE FROM article_identity_keys WHERE identity_kind=? AND identity_value=?", key.Kind, key.Value); err != nil {
		t.Fatal(err)
	}
	outcome, err = WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "conflict", "epoch")
	if err != nil || outcome.ArticlesChanged != 1 || outcome.IdentityAliasesAdded != 2 {
		t.Fatal("retry lost incoming or merged-only alias counts", outcome, err)
	}
}

// articleAliasOwner resolves the fixture's canonical DOI without deriving IDs from weak aliases.
func articleAliasOwner(t *testing.T, connection *Connection, article domain.ArticleDraft) int64 {
	t.Helper()
	var owner int64
	if err := connection.QueryRowContext(context.Background(), identityLookupSql, "doi", *article.Doi).Scan(&owner); err != nil {
		t.Fatal(err)
	}
	return owner
}

// legacyAliasFixture reproduces independent bibliographic and DOI records for one publication.
func legacyAliasFixture(t *testing.T, change func(*domain.ArticleDraft, *domain.ArticleDraft)) (*Connection, domain.JournalCatalogEntry, domain.ProviderBatch, int64, int64) {
	t.Helper()
	connection, err := OpenContent(context.Background(), filepath.Join(t.TempDir(), "legacy-aliases.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	catalog, _ := contentInput(t)
	legacy := domain.ArticleDraft{CatalogId: catalog.CatalogId, Title: "Iterated Learning in Dynamic Social Networks", PublicationYear: new(int64(2019)), Date: new("2019-01-01"), Volume: new("20"), IssueNumber: new("29"), StartPage: new("1"), EndPage: new("28")}
	strong := domain.ArticleDraft{CatalogId: catalog.CatalogId, Title: "Iterated learning in dynamic social networks", PublicationYear: new(int64(2019)), Date: new("2019-01-01"), Doi: new("10.5555/3322706.3322735")}
	if change != nil {
		change(&legacy, &strong)
	}
	incoming := legacy
	incoming.Doi = strong.Doi
	incoming.Pmid = nil
	incoming.AbstractText = new("Newly available publication metadata")
	for _, article := range []domain.ArticleDraft{legacy, strong} {
		batch := domain.BatchFromArticles(catalog, []domain.ArticleDraft{article}, domain.ProviderProgress{State: domain.Complete})
		if _, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "seed", "epoch"); err != nil {
			t.Fatal(err)
		}
	}
	key := ArticleIdentityKeys(legacy)[len(ArticleIdentityKeys(legacy))-1]
	var weakId int64
	if err := connection.QueryRowContext(context.Background(), identityLookupSql, key.Kind, key.Value).Scan(&weakId); err != nil {
		t.Fatal(err)
	}
	batch := domain.BatchFromArticles(catalog, []domain.ArticleDraft{incoming}, domain.ProviderProgress{State: domain.Complete})
	return connection, catalog, batch, weakId, articleAliasOwner(t, connection, strong)
}

// TestLegacyBibliographicAliasPreservesBothIdentities permits exact DOI enrichment without reassigning aliases.
func TestLegacyBibliographicAliasPreservesBothIdentities(t *testing.T) {
	for _, scenario := range []string{"exact-title", "math-label-spacing"} {
		t.Run(scenario, func(t *testing.T) {
			connection, catalog, batch, weakId, strongId := legacyAliasFixture(t, func(legacy, strong *domain.ArticleDraft) {
				if scenario == "math-label-spacing" {
					legacy.Title = "An error bound for L 1 -norm support vector machine coefficients"
					strong.Title = "An error bound for L1-norm support vector machine coefficients"
				}
			})
			writer, err := prepareWriter(context.Background(), connection.Conn, true)
			if err != nil {
				t.Fatal(err)
			}
			defer writer.close()
			legacyBefore, _, err := writer.loadArticle(weakId)
			if err != nil {
				t.Fatal(err)
			}
			aliasesBefore := selectedSnapshot(t, connection.Conn, []string{"article_identity_keys"})
			outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "enrich", "epoch")
			if err != nil || outcome.ArticlesChanged != 1 || outcome.IdentityAliasesAdded != 0 || outcome.ChangeEventsEmitted != 2 {
				t.Fatal("exact legacy publication prevented DOI enrichment", outcome, err)
			}
			legacyAfter, _, err := writer.loadArticle(weakId)
			if err != nil || !reflect.DeepEqual(legacyBefore, legacyAfter) {
				t.Fatal("legacy article identity or metadata changed", err)
			}
			strong, _, err := writer.loadArticle(strongId)
			if err != nil || strong == nil || !reflect.DeepEqual(strong.AbstractText, batch.Articles[0].AbstractText) || !reflect.DeepEqual(strong.Volume, batch.Articles[0].Volume) {
				t.Fatal("DOI owner was not refreshed", strong, err)
			}
			var count int
			if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM articles").Scan(&count); err != nil || count != 2 {
				t.Fatal("historical IDs were not preserved", count, err)
			}
			if !reflect.DeepEqual(aliasesBefore, selectedSnapshot(t, connection.Conn, []string{"article_identity_keys"})) {
				t.Fatal("legacy alias ownership changed")
			}
			outcome, err = WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "enrich", "epoch")
			if err != nil || outcome.ArticlesChanged != 0 || outcome.IdentityAliasesAdded != 0 || outcome.ChangeEventsEmitted != 0 {
				t.Fatal("legacy enrichment replay was not idempotent", outcome, err)
			}
		})
	}
}

// TestLegacyBibliographicAliasRejectsConflicts keeps contradictory publication evidence atomic.
func TestLegacyBibliographicAliasRejectsConflicts(t *testing.T) {
	for _, scenario := range []string{"legacy-doi", "legacy-pmid", "historical-doi", "historical-pmid", "strong-historical-doi", "strong-historical-pmid", "strong-title", "math-label-value", "ordinary-word-spacing", "strong-year", "strong-volume", "strong-issue", "strong-page", "strong-end-page", "legacy-title", "legacy-year", "legacy-page", "strong-pmid", "late-owner"} {
		t.Run(scenario, func(t *testing.T) {
			connection, catalog, batch, weakId, strongId := legacyAliasFixture(t, func(legacy, strong *domain.ArticleDraft) {
				switch scenario {
				case "legacy-doi":
					legacy.Doi = new("10.1234/unrelated")
				case "legacy-pmid":
					legacy.Pmid = new("42")
				case "strong-title":
					strong.Title = "An unrelated publication"
				case "math-label-value":
					legacy.Title, strong.Title = "An error bound for L 1 -norm coefficients", "An error bound for L2-norm coefficients"
				case "ordinary-word-spacing":
					strong.Title = "Iteratedlearning in dynamic social networks"
				case "strong-year":
					strong.PublicationYear, strong.Date = new(int64(2020)), new("2020-01-01")
				case "strong-volume":
					strong.Volume = new("21")
				case "strong-issue":
					strong.IssueNumber = new("30")
				case "strong-page":
					strong.StartPage = new("2")
				case "strong-end-page":
					strong.EndPage = new("29")
				case "strong-pmid":
					strong.Pmid = new("42")
				}
			})
			if strings.Contains(scenario, "historical-") {
				kind, value := "doi", "10.1234/historical"
				if strings.HasSuffix(scenario, "pmid") {
					kind, value = "pmid", "42"
				}
				owner := weakId
				if strings.HasPrefix(scenario, "strong-") {
					owner = strongId
				}
				if _, err := connection.ExecContext(context.Background(), "INSERT INTO article_identity_keys VALUES(?,?,?)", kind, value, owner); err != nil {
					t.Fatal(err)
				}
			}
			if strings.HasPrefix(scenario, "legacy-") && scenario != "legacy-doi" && scenario != "legacy-pmid" {
				statement := "UPDATE articles SET title='An unrelated publication' WHERE article_id=?"
				if scenario == "legacy-year" {
					statement = "UPDATE articles SET publication_year=2020,date='2020-01-01' WHERE article_id=?"
				} else if scenario == "legacy-page" {
					statement = "UPDATE articles SET start_page='2' WHERE article_id=?"
				}
				if _, err := connection.ExecContext(context.Background(), statement, weakId); err != nil {
					t.Fatal(err)
				}
			}
			if scenario == "strong-pmid" {
				batch.Articles[0].Pmid = new("43")
			}
			if scenario == "late-owner" {
				trigger := "CREATE TRIGGER move_legacy_alias AFTER UPDATE ON articles WHEN NEW.article_id=" + strconv.FormatInt(strongId, 10) + " BEGIN UPDATE article_identity_keys SET article_id=NEW.article_id WHERE identity_kind='bibliographic' AND article_id=" + strconv.FormatInt(weakId, 10) + "; END"
				if _, err := connection.ExecContext(context.Background(), trigger); err != nil {
					t.Fatal(err)
				}
			}
			before := selectedSnapshot(t, connection.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
			outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batch, "conflict", "epoch")
			var conflict *IdentityConflict
			if !errors.As(err, &conflict) || outcome != (ContentWriteOutcome{}) {
				t.Fatal("contradictory identity was accepted", outcome, err)
			}
			after := selectedSnapshot(t, connection.Conn, []string{"journals", "issues", "articles", "article_listing", "article_search", "article_identity_keys", "article_change_events"})
			if !reflect.DeepEqual(before, after) {
				t.Fatal("identity conflict leaked writes")
			}
		})
	}
}
