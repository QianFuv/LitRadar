package scholarly

import (
	"context"
	"database/sql"
	"errors"
	"path/filepath"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// openAlexVolumeFixture reproduces the observed chapter's truncated OpenAlex volume.
func openAlexVolumeFixture() (domain.JournalCatalogEntry, map[string]any) {
	catalog := domain.JournalCatalogEntry{CatalogId: "issn-0733-558x", Title: "Research in the Sociology of Organizations", Issn: new("0733-558X"), AllIssns: []string{"0733-558X"}}
	work := map[string]any{
		"display_name": "From Agents to Principles: The Changing Relationship between Hospitalist Identity and Logics of Health care",
		"doi":          "https://doi.org/10.1108/S0733-558X(2013)0039A011", "publication_year": int64(2013), "publication_date": "2013-09-05",
		"biblio":      map[string]any{"volume": "39", "first_page": "203", "last_page": "241"},
		"authorships": []any{map[string]any{"author": map[string]any{"display_name": "Vanessa Pouthier"}}},
	}
	return catalog, work
}

// TestOpenAlexVolumeErratumPreservesExistingIdentities verifies conversion through canonical storage.
func TestOpenAlexVolumeErratumPreservesExistingIdentities(t *testing.T) {
	ctx := context.Background()
	catalog, work := openAlexVolumeFixture()
	article := openAlexArticle(catalog, work)
	first, second := *article, *article
	first.Volume = nil
	second.Doi, second.Volume, second.Date = new("10.1108/s0733-558x(2013)0039ab011"), new("39"), new("2013-01-01")
	connection, err := storage.OpenContent(ctx, filepath.Join(t.TempDir(), "erratum.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	seed := domain.BatchFromArticles(catalog, []domain.ArticleDraft{first, second}, domain.ProviderProgress{State: domain.Complete})
	if _, err := storage.WriteContentBatch(ctx, connection.Conn, catalog, seed, "seed", "epoch"); err != nil {
		t.Fatal(err)
	}
	const firstId int64 = 2796358338862224948
	const secondId int64 = 6800538005797110105
	before := snapshotErratumOwner(t, connection.Conn, secondId)
	batch := domain.BatchFromArticles(catalog, []domain.ArticleDraft{*article}, domain.ProviderProgress{State: domain.Complete})
	outcome, err := storage.WriteContentBatch(ctx, connection.Conn, catalog, batch, "corrected", "epoch")
	if err != nil || outcome.ArticlesChanged != 1 || outcome.IdentityAliasesAdded != 1 {
		t.Fatalf("corrected volume did not reuse its DOI owner: %#v %v", outcome, err)
	}
	if !reflect.DeepEqual(before, snapshotErratumOwner(t, connection.Conn, secondId)) {
		t.Fatal("correction changed the other immutable identity")
	}
	var count, owner int64
	for _, original := range []struct {
		article domain.ArticleDraft
		owner   int64
	}{{first, firstId}, {second, secondId}} {
		for _, key := range storage.ArticleIdentityKeys(original.article) {
			if err := connection.QueryRowContext(ctx, "SELECT article_id FROM article_identity_keys WHERE identity_kind=? AND identity_value=?", key.Kind, key.Value).Scan(&owner); err != nil || owner != original.owner {
				t.Fatal("correction reassigned a legacy alias", owner, err)
			}
		}
	}
	if err := connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM articles").Scan(&count); err != nil || count != 2 {
		t.Fatal("correction changed article identities", count, err)
	}
	for _, key := range storage.ArticleIdentityKeys(*article) {
		if err := connection.QueryRowContext(ctx, "SELECT article_id FROM article_identity_keys WHERE identity_kind=? AND identity_value=?", key.Kind, key.Value).Scan(&owner); err != nil || owner != firstId {
			t.Fatal("corrected alias lost its DOI owner", owner, err)
		}
	}
	outcome, err = storage.WriteContentBatch(ctx, connection.Conn, catalog, batch, "replay", "epoch")
	if err != nil || outcome.ArticlesChanged != 0 || outcome.IdentityAliasesAdded != 0 || outcome.ChangeEventsEmitted != 0 {
		t.Fatal("corrected replay was not idempotent", outcome, err)
	}
	batch.Articles[0].Volume = new("39")
	var conflict *storage.IdentityConflict
	if _, err := storage.WriteContentBatch(ctx, connection.Conn, catalog, batch, "uncorrected", "epoch"); !errors.As(err, &conflict) {
		t.Fatal("original identity bridge was admitted", err)
	}
	if providerVolume := field(field(work, "biblio"), "volume"); providerVolume != "39" {
		t.Fatal("conversion mutated the upstream observation")
	}
}

// TestOpenAlexVolumeErratumIsRestricted refuses to generalize a verified correction to other records.
func TestOpenAlexVolumeErratumIsRestricted(t *testing.T) {
	for _, scenario := range []struct {
		name   string
		change func(*domain.JournalCatalogEntry, map[string]any)
		volume string
	}{
		{"verified-prefixed-uppercase-doi", func(*domain.JournalCatalogEntry, map[string]any) {}, "39 Part A"},
		{"other-doi", func(_ *domain.JournalCatalogEntry, work map[string]any) {
			work["doi"] = "10.1108/s0733-558x(2013)0039ab011"
		}, "39"},
		{"other-catalog", func(catalog *domain.JournalCatalogEntry, _ map[string]any) { catalog.CatalogId = "other" }, "39"},
		{"other-year", func(_ *domain.JournalCatalogEntry, work map[string]any) { work["publication_year"] = int64(2014) }, "39"},
		{"other-volume", func(_ *domain.JournalCatalogEntry, work map[string]any) {
			work["biblio"].(map[string]any)["volume"] = "40"
		}, "40"},
		{"other-first-page", func(_ *domain.JournalCatalogEntry, work map[string]any) {
			work["biblio"].(map[string]any)["first_page"] = "204"
		}, "39"},
		{"other-last-page", func(_ *domain.JournalCatalogEntry, work map[string]any) {
			work["biblio"].(map[string]any)["last_page"] = "240"
		}, "39"},
		{"has-issue", func(_ *domain.JournalCatalogEntry, work map[string]any) {
			work["biblio"].(map[string]any)["issue"] = "1"
		}, "39"},
		{"already-correct", func(_ *domain.JournalCatalogEntry, work map[string]any) {
			work["biblio"].(map[string]any)["volume"] = "39 Part A"
		}, "39 Part A"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			catalog, work := openAlexVolumeFixture()
			scenario.change(&catalog, work)
			article := openAlexArticle(catalog, work)
			if article == nil || article.Volume == nil || *article.Volume != scenario.volume {
				t.Fatalf("erratum escaped its verified record: %#v", article)
			}
		})
	}
}

// snapshotErratumOwner checks all article-owned canonical, alias, projection and event rows.
func snapshotErratumOwner(t *testing.T, connection *sql.Conn, id int64) map[string][][]any {
	t.Helper()
	result := map[string][][]any{}
	for _, table := range []string{"articles", "article_identity_keys", "article_listing", "article_search", "article_change_events"} {
		rows, err := connection.QueryContext(context.Background(), "SELECT * FROM "+table+" WHERE article_id=? ORDER BY 1,2", id)
		if err != nil {
			t.Fatal(err)
		}
		columns, err := rows.Columns()
		if err != nil {
			rows.Close()
			t.Fatal(err)
		}
		for rows.Next() {
			record := make([]any, len(columns))
			destinations := make([]any, len(columns))
			for ordinal := range record {
				destinations[ordinal] = &record[ordinal]
			}
			if err := rows.Scan(destinations...); err != nil {
				rows.Close()
				t.Fatal(err)
			}
			result[table] = append(result[table], record)
		}
		if err := rows.Err(); err != nil {
			rows.Close()
			t.Fatal(err)
		}
		rows.Close()
	}
	return result
}
