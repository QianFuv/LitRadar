package index

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"reflect"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func journalConflict() error {
	return &provider.ContractViolation{Message: "journal identity ownership conflicts with canonical catalog"}
}
func catalogKeys(catalog domain.JournalCatalogEntry) []ArticleIdentityKey {
	keys := []ArticleIdentityKey{{"catalog_id", catalog.CatalogId}}
	for _, value := range catalog.CatalogAliases {
		keys = append(keys, ArticleIdentityKey{"catalog_id", value})
	}
	for _, value := range catalog.AllIssns {
		keys = append(keys, ArticleIdentityKey{"issn", value})
	}
	return keys
}

func claimJournalKey(ctx context.Context, connection *sql.Conn, key ArticleIdentityKey, owner string) error {
	var previous sqlite.Text
	err := connection.QueryRowContext(ctx, "SELECT canonical_catalog_id FROM journal_identity_keys WHERE identity_kind=?1 AND identity_value=?2", key.Kind, key.Value).Scan(&previous)
	if err == nil {
		if string(previous) != owner {
			return journalConflict()
		}
		return nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	_, err = connection.ExecContext(ctx, "INSERT INTO journal_identity_keys(identity_kind,identity_value,canonical_catalog_id) VALUES(?1,?2,?3)", key.Kind, key.Value, owner)
	return err
}

func ensureJournalSlot(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry) error {
	var stored sqlite.Integer
	err := connection.QueryRowContext(ctx, "SELECT journal_id FROM journals WHERE catalog_id=?1", catalog.CatalogId).Scan(&stored)
	if err == nil && int64(stored) != JournalId(catalog.CatalogId) {
		return journalConflict()
	}
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	var owner sqlite.Text
	err = connection.QueryRowContext(ctx, "SELECT catalog_id FROM journals WHERE journal_id=?1", JournalId(catalog.CatalogId)).Scan(&owner)
	if err == nil && string(owner) != catalog.CatalogId {
		return journalConflict()
	}
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	return nil
}

func claimCatalog(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry) error {
	if err := ensureJournalSlot(ctx, connection, catalog); err != nil {
		return err
	}
	for _, alias := range catalog.CatalogAliases {
		var exists sqlite.Integer
		if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM journals WHERE catalog_id=?1)", alias).Scan(&exists); err != nil {
			return err
		}
		if exists != 0 {
			return journalConflict()
		}
	}
	for _, key := range catalogKeys(catalog) {
		if err := claimJournalKey(ctx, connection, key, catalog.CatalogId); err != nil {
			return err
		}
	}
	return nil
}

// ReconcileCatalogIdentities atomically adopts aliases only after proving any legacy journal shell empty.
func ReconcileCatalogIdentities(ctx context.Context, connection *sql.Conn, entries []domain.JournalCatalogEntry) error {
	desired := map[ArticleIdentityKey]string{}
	legacy := map[string]string{}
	for _, entry := range entries {
		if err := provider.ValidateCatalogEntry(entry); err != nil {
			return err
		}
		for _, key := range catalogKeys(entry) {
			if previous, ok := desired[key]; ok && previous != entry.CatalogId {
				return journalConflict()
			}
			desired[key] = entry.CatalogId
		}
		for _, alias := range entry.CatalogAliases {
			if previous, ok := legacy[alias]; ok && previous != entry.CatalogId {
				return journalConflict()
			}
			legacy[alias] = entry.CatalogId
		}
	}
	return immediate(ctx, connection, func() error {
		aliases := make([]string, 0, len(legacy))
		for alias := range legacy {
			aliases = append(aliases, alias)
		}
		slices.Sort(aliases)
		for _, alias := range aliases {
			if err := removeEmptyLegacyJournal(ctx, connection, alias); err != nil {
				return err
			}
		}
		keys := make([]ArticleIdentityKey, 0, len(desired))
		for key := range desired {
			keys = append(keys, key)
		}
		sortIdentityKeys(keys)
		for _, key := range keys {
			if err := claimJournalKey(ctx, connection, key, desired[key]); err != nil {
				return err
			}
		}
		for _, entry := range entries {
			if err := ensureJournalSlot(ctx, connection, entry); err != nil {
				return err
			}
			var exists sqlite.Integer
			if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM journals WHERE catalog_id=?1)", entry.CatalogId).Scan(&exists); err != nil {
				return err
			}
			if exists != 0 {
				refresh, err := upsertJournal(ctx, connection, entry)
				if err != nil {
					return err
				}
				if err := refreshJournal(ctx, connection, entry, refresh); err != nil {
					return err
				}
			}
		}
		return nil
	})
}

func removeEmptyLegacyJournal(ctx context.Context, connection *sql.Conn, alias string) error {
	var journal sqlite.Integer
	err := connection.QueryRowContext(ctx, "SELECT journal_id FROM journals WHERE catalog_id=?1", alias).Scan(&journal)
	if errors.Is(err, sql.ErrNoRows) {
		_, err = connection.ExecContext(ctx, "DELETE FROM journal_identity_keys WHERE canonical_catalog_id=?1", alias)
		return err
	}
	if err != nil {
		return err
	}
	var exists sqlite.Integer
	err = connection.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM issues WHERE journal_id=?1 UNION ALL SELECT 1 FROM articles WHERE journal_id=?1 UNION ALL SELECT 1 FROM article_listing WHERE journal_id=?1 UNION ALL SELECT 1 FROM article_search JOIN articles ON articles.article_id=article_search.rowid WHERE articles.journal_id=?1 UNION ALL SELECT 1 FROM article_change_events WHERE journal_id=?1)`, journal).Scan(&exists)
	if err != nil {
		return err
	}
	if exists != 0 {
		return &provider.ContractViolation{Message: "legacy journal entity owns content or durable history"}
	}
	if _, err = connection.ExecContext(ctx, "DELETE FROM journal_identity_keys WHERE canonical_catalog_id=?1", alias); err != nil {
		return err
	}
	result, err := connection.ExecContext(ctx, "DELETE FROM journals WHERE journal_id=?1 AND catalog_id=?2", journal, alias)
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if count != 1 {
		return journalConflict()
	}
	return nil
}

type projectionRefresh struct{ shouldRefreshArea, shouldRefreshTitle bool }

func upsertJournal(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry) (projectionRefresh, error) {
	refresh := projectionRefresh{}
	var title sqlite.Text
	var area sqlite.OptionalText
	err := connection.QueryRowContext(ctx, journalProjectionSql, JournalId(catalog.CatalogId)).Scan(&title, &area)
	if err == nil {
		refresh = projectionRefresh{!reflect.DeepEqual(area.Value, catalog.Area), string(title) != catalog.Title}
	} else if !errors.Is(err, sql.ErrNoRows) {
		return refresh, err
	}
	titles, err := jsonArray(append([]string{}, catalog.TitleAliases...))
	if err != nil {
		return refresh, err
	}
	issns, err := jsonArray(append([]string{}, catalog.AllIssns...))
	if err != nil {
		return refresh, err
	}
	rankings := catalog.Rankings
	_, err = connection.ExecContext(ctx, journalUpsertSql, JournalId(catalog.CatalogId), catalog.CatalogId, catalog.Title, titles, issns, catalog.Issn, catalog.Eissn, catalog.Area, rankings.UtdRank, rankings.UtdRating, rankings.AbsRank, rankings.AbsRating, rankings.FmsRank, rankings.FmsRating, rankings.FmscnRank, rankings.FmscnRating)
	return refresh, err
}

func refreshJournal(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry, refresh projectionRefresh) error {
	journal := JournalId(catalog.CatalogId)
	if refresh.shouldRefreshArea {
		if _, err := connection.ExecContext(ctx, "UPDATE article_listing SET area=?1 WHERE journal_id=?2", catalog.Area, journal); err != nil {
			return err
		}
	}
	if !refresh.shouldRefreshTitle {
		return nil
	}
	rows, err := connection.QueryContext(ctx, `SELECT articles.article_id,articles.title,articles.abstract_text,articles.doi,articles.pmid,COALESCE((SELECT group_concat(CASE authors.type WHEN 'object' THEN json_extract(authors.value,'$.display_name') ELSE CAST(authors.value AS TEXT) END,'; ') FROM json_each(articles.authors_json) AS authors),'') FROM articles WHERE articles.journal_id=?1 ORDER BY articles.article_id`, journal)
	if err != nil {
		return err
	}
	type projection struct {
		id                  sqlite.Integer
		title, authors      sqlite.Text
		abstract, doi, pmid sqlite.OptionalText
	}
	projections := []projection{}
	for rows.Next() {
		var value projection
		if err := rows.Scan(&value.id, &value.title, &value.abstract, &value.doi, &value.pmid, &value.authors); err != nil {
			rows.Close()
			return err
		}
		projections = append(projections, value)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return err
	}
	isSimple, err := search.UsesSimple(ctx, connection)
	if err != nil {
		return err
	}
	for _, value := range projections {
		if _, err := connection.ExecContext(ctx, searchDeleteSql, value.id); err != nil {
			return err
		}
		if _, err := connection.ExecContext(ctx, searchInsertSql, value.id, search.PrepareText(string(value.title), isSimple), search.PrepareText(optionalString(value.abstract.Value), isSimple), search.PrepareText(optionalString(value.doi.Value), isSimple), search.PrepareText(optionalString(value.pmid.Value), isSimple), search.PrepareText(string(value.authors), isSimple), search.PrepareText(catalog.Title, isSimple)); err != nil {
			return err
		}
	}
	return nil
}

func sortIdentityKeys(keys []ArticleIdentityKey) {
	slices.SortFunc(keys, func(first, second ArticleIdentityKey) int {
		if result := strings.Compare(first.Kind, second.Kind); result != 0 {
			return result
		}
		return strings.Compare(first.Value, second.Value)
	})
}
func jsonBytes(value any) ([]byte, error) {
	encoded, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	var normalized any
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.UseNumber()
	if err := decoder.Decode(&normalized); err != nil {
		return nil, err
	}
	return domain.Json(normalized)
}
