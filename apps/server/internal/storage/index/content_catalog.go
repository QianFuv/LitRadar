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

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/provider"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/contentdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

var contentQueries contentdb.Queries

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
	err := func() error {
		queryResult, err := contentQueries.JournalKeyOwner(ctx, connection, contentdb.JournalKeyOwnerParams{IdentityKind: sqlite.Text(key.Kind), IdentityValue: sqlite.Text(key.Value)})
		if err != nil {
			return err
		}
		previous = queryResult
		return nil
	}()
	if err == nil {
		if string(previous) != owner {
			return journalConflict()
		}
		return nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	_, err = contentQueries.InsertJournalKey(ctx, connection, contentdb.InsertJournalKeyParams{IdentityKind: sqlite.Text(key.Kind), IdentityValue: sqlite.Text(key.Value), CanonicalCatalogID: sqlite.Text(owner)})
	return err
}

func ensureJournalSlot(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry) error {
	var stored sqlite.Integer
	err := func() error {
		queryResult, err := contentQueries.JournalID(ctx, connection, contentdb.JournalIDParams{CatalogID: sqlite.Text(catalog.CatalogId)})
		if err != nil {
			return err
		}
		stored = queryResult
		return nil
	}()
	if err == nil && int64(stored) != JournalId(catalog.CatalogId) {
		return journalConflict()
	}
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	var owner sqlite.Text
	err = func() error {
		queryResult, err := contentQueries.JournalCatalogID(ctx, connection, contentdb.JournalCatalogIDParams{JournalID: sqlite.Integer(JournalId(catalog.CatalogId))})
		if err != nil {
			return err
		}
		owner = queryResult
		return nil
	}()
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
		if err := func() error {
			queryResult, err := contentQueries.HasJournalCatalog(ctx, connection, contentdb.HasJournalCatalogParams{CatalogID: sqlite.Text(alias)})
			if err != nil {
				return err
			}
			exists = sqlite.Integer(dbtypes.Boolean(queryResult))
			return nil
		}(); err != nil {
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
			if err := func() error {
				queryResult, err := contentQueries.HasJournalCatalog(ctx, connection, contentdb.HasJournalCatalogParams{CatalogID: sqlite.Text(entry.CatalogId)})
				if err != nil {
					return err
				}
				exists = sqlite.Integer(dbtypes.Boolean(queryResult))
				return nil
			}(); err != nil {
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
	err := func() error {
		queryResult, err := contentQueries.JournalID(ctx, connection, contentdb.JournalIDParams{CatalogID: sqlite.Text(alias)})
		if err != nil {
			return err
		}
		journal = queryResult
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) {
		_, err = contentQueries.DeleteJournalKeys(ctx, connection, contentdb.DeleteJournalKeysParams{CanonicalCatalogID: sqlite.Text(alias)})
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
	if _, err = contentQueries.DeleteJournalKeys(ctx, connection, contentdb.DeleteJournalKeysParams{CanonicalCatalogID: sqlite.Text(alias)}); err != nil {
		return err
	}
	result, err := contentQueries.DeleteEmptyJournal(ctx, connection, contentdb.DeleteEmptyJournalParams{JournalID: journal, CatalogID: sqlite.Text(alias)})
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
	err := func() error {
		queryResult, err := contentQueries.JournalProjection(ctx, connection, contentdb.JournalProjectionParams{JournalID: sqlite.Integer(JournalId(catalog.CatalogId))})
		if err != nil {
			return err
		}
		title = queryResult.Title
		if queryResult.Area == nil {
			area.Value = nil
		} else {
			area.Value = new(string(*queryResult.Area))
		}
		return nil
	}()
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
	_, err = contentQueries.UpsertJournal(ctx, connection, contentdb.UpsertJournalParams{JournalID: sqlite.Integer(JournalId(catalog.CatalogId)), CatalogID: sqlite.Text(catalog.CatalogId), Title: sqlite.Text(catalog.Title), TitleAliasesJSON: sqlite.Text(titles), IssnsJSON: sqlite.Text(issns), ISSN: (*sqlite.Text)(catalog.Issn), EISSN: (*sqlite.Text)(catalog.Eissn), Area: (*sqlite.Text)(catalog.Area), UTDRank: (*sqlite.Text)(rankings.UtdRank), UTDRating: (*sqlite.Text)(rankings.UtdRating), ABSRank: (*sqlite.Text)(rankings.AbsRank), ABSRating: (*sqlite.Text)(rankings.AbsRating), FMSRank: (*sqlite.Text)(rankings.FmsRank), FMSRating: (*sqlite.Text)(rankings.FmsRating), FMSCNRank: (*sqlite.Text)(rankings.FmscnRank), FMSCNRating: (*sqlite.Text)(rankings.FmscnRating)})
	return refresh, err
}

// refreshJournal republishes canonical metadata with one prepared statement pair and journal title per refresh.
func refreshJournal(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry, refresh projectionRefresh) error {
	journal := JournalId(catalog.CatalogId)
	if refresh.shouldRefreshArea {
		if _, err := contentQueries.UpdateListingArea(ctx, connection, contentdb.UpdateListingAreaParams{Area: (*sqlite.Text)(catalog.Area), JournalID: sqlite.Integer(journal)}); err != nil {
			return err
		}
	}
	if !refresh.shouldRefreshTitle {
		return nil
	}
	projections, err := readJournalProjections(ctx, connection, journal)
	if err != nil {
		return err
	}
	if len(projections) == 0 {
		return nil
	}
	deleteProjection, err := connection.PrepareContext(ctx, searchDeleteSql)
	if err != nil {
		return err
	}
	defer deleteProjection.Close()
	insertProjection, err := connection.PrepareContext(ctx, searchInsertSql)
	if err != nil {
		return err
	}
	defer insertProjection.Close()
	journalTitle := search.PrepareText(catalog.Title)
	for _, value := range projections {
		if _, err := deleteProjection.ExecContext(ctx, value.id); err != nil {
			return err
		}
		if _, err := insertProjection.ExecContext(ctx, value.id, search.PrepareText(string(value.title)), search.PrepareText(optionalString(value.abstract.Value)), search.PrepareText(optionalString(value.doi.Value)), search.PrepareText(optionalString(value.pmid.Value)), search.PrepareText(string(value.authors)), journalTitle); err != nil {
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

type journalProjection struct {
	id                  sqlite.Integer
	title, authors      sqlite.Text
	abstract, doi, pmid sqlite.OptionalText
}

func readJournalProjections(ctx context.Context, connection *sql.Conn, journal int64) ([]journalProjection, error) {
	rows, err := contentQueries.JournalProjections(ctx, connection, contentdb.JournalProjectionsParams{JournalID: sqlite.Integer(journal)})
	if err != nil {
		return nil, err
	}
	projections := make([]journalProjection, 0, len(rows))
	for _, row := range rows {
		var authors sqlite.Text
		if err := authors.Scan(row.Coalesce); err != nil {
			return nil, err
		}
		projections = append(projections, journalProjection{id: row.ArticleID, title: row.Title, authors: authors, abstract: sqlite.OptionalText{Value: (*string)(row.AbstractText)}, doi: sqlite.OptionalText{Value: (*string)(row.DOI)}, pmid: sqlite.OptionalText{Value: (*string)(row.PMID)}})
	}
	return projections, nil
}
