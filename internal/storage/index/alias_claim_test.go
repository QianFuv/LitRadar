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
