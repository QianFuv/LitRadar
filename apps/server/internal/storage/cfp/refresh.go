package cfp

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"math"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/cfpdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// BeginRefresh replaces even an active generation; source work never holds a database transaction.
func (repository *Repository) BeginRefresh(ctx context.Context, sourceKey string, now, leaseSeconds int64) (result RefreshLease, resultError error) {
	defer func() { resultError = databaseError(resultError) }()
	if leaseSeconds < 1 || leaseSeconds > 600 {
		return RefreshLease{}, invalid("CFP lease must be between 1 and 600 seconds")
	}
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return RefreshLease{}, err
	}
	defer connection.Close()
	if now > math.MaxInt64-leaseSeconds {
		return RefreshLease{}, invalid("Invalid CFP lease timestamp")
	}
	expires := now + leaseSeconds
	var generation sqlite.Integer
	err = func() error {
		queryResult, err := queries.BeginRefresh(ctx, connection, cfpdb.BeginRefreshParams{SourceKey: sqlite.Text(sourceKey), LeaseExpiresAt: new(sqlite.Integer(expires)), LastAttempt: new(sqlite.Integer(now))})
		if err != nil {
			return err
		}
		generation = queryResult
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) {
		return RefreshLease{}, invalid("CFP source is not registered in storage")
	}
	if err != nil {
		return RefreshLease{}, err
	}
	return RefreshLease{SourceKey: sourceKey, Generation: int64(generation), ExpiresAt: expires}, nil
}

// FailRefresh may finish an expired generation until a newer attempt supersedes it.
func (repository *Repository) FailRefresh(ctx context.Context, lease RefreshLease, reason string, isUnsupported bool) (resultError error) {
	defer func() { resultError = databaseError(resultError) }()
	status := "failed"
	if isUnsupported {
		status = "unsupported"
	}
	characters := []rune(reason)
	if len(characters) > 1000 {
		reason = string(characters[:1000])
	}
	result, err := queries.FailRefresh(ctx, repository.database, cfpdb.FailRefreshParams{SourceKey: sqlite.Text(lease.SourceKey), Generation: sqlite.Integer(lease.Generation), Status: sqlite.Text(status), LastError: new(sqlite.Text(reason))})
	if err != nil {
		return err
	}
	changed, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if changed != 1 {
		return ErrStaleRefresh
	}
	return nil
}

// PublishRefresh replaces every bound journal atomically under both lease expiry checks.
func (repository *Repository) PublishRefresh(ctx context.Context, lease RefreshLease, publication Publication, now int64) error {
	return repository.publish(ctx, lease, publication, now, nil)
}

type noticeIdentity struct {
	aliases []string
	title   string
}

func identities(sources []domain.Source) []noticeIdentity {
	result := make([]noticeIdentity, 0, len(sources))
	for _, source := range sources {
		result = append(result, noticeIdentity{aliases: source.CatalogIds, title: source.Title})
	}
	slices.SortFunc(result, func(first, second noticeIdentity) int {
		if order := slices.Compare(first.aliases, second.aliases); order != 0 {
			return order
		}
		return strings.Compare(first.title, second.title)
	})
	return result
}

// PublishFullText preserves the multiset of original titles and ordered catalog aliases.
func (repository *Repository) PublishFullText(ctx context.Context, lease RefreshLease, publication Publication, now int64, unresolved uint64) error {
	originals, err := repository.LoadOriginals(ctx, lease.SourceKey)
	if err != nil {
		return err
	}
	first, second := identities(originals), identities(publication.Sources)
	if !slices.EqualFunc(first, second, func(first, second noticeIdentity) bool {
		return first.title == second.title && slices.Equal(first.aliases, second.aliases)
	}) {
		return invalid("Full-text enrichment cannot remove or replace existing notice identities")
	}
	var warning *string
	if unresolved > 0 {
		value := fmt.Sprintf("Full original text remains unadapted for %d notices; previous records retained", unresolved)
		warning = &value
	}
	return repository.publish(ctx, lease, publication, now, warning)
}

func (repository *Repository) publish(ctx context.Context, lease RefreshLease, publication Publication, now int64, warning *string) error {
	if strings.TrimSpace(publication.Capture) == "" || !domain.IsSourceUrl(publication.SourceUrl) {
		return invalid("CFP publication requires a verified capture and URL")
	}
	prepared, err := prepare(domain.Seed{FormatVersion: 1, Sources: publication.Sources, EmptyJournals: publication.EmptyJournals})
	if err != nil {
		return err
	}
	return repository.transaction(ctx, true, func(connection *sql.Conn) error {
		rows, err := queries.SourceJournals(ctx, connection, cfpdb.SourceJournalsParams{SourceKey: sqlite.Text(lease.SourceKey)})
		if err != nil {
			return err
		}
		bindings := make([]string, 0, len(rows))
		for _, key := range rows {
			bindings = append(bindings, string(key))
		}
		slices.Sort(bindings)
		if len(bindings) == 0 || !slices.Equal(bindings, sortedKeys(prepared)) {
			return invalid("CFP publication does not cover every registered journal binding")
		}
		result, err := queries.PublishSource(ctx, connection, cfpdb.PublishSourceParams{SourceKey: sqlite.Text(lease.SourceKey), Generation: sqlite.Integer(lease.Generation), Capture: sqlite.Text(publication.Capture), CaptureFormat: sqlite.Text(publication.CaptureFormat), ContentHash: sqlite.Text(digest([]byte(publication.Capture))), URL: sqlite.Text(publication.SourceUrl), ParserVersion: sqlite.Integer(domain.ParserVersion), ConfigVersion: sqlite.Integer(publication.ConfigVersion), LastSuccess: new(sqlite.Integer(now)), LastError: (*sqlite.Text)(warning)})
		if err != nil {
			return err
		}
		changed, err := result.RowsAffected()
		if err != nil {
			return err
		}
		if changed != 1 || now > lease.ExpiresAt {
			return ErrStaleRefresh
		}
		for _, key := range sortedKeys(prepared) {
			journal := prepared[key]
			rows, err := queries.JournalAliases(ctx, connection, cfpdb.JournalAliasesParams{JournalKey: sqlite.Text(key)})
			if err != nil {
				return err
			}
			aliases := map[string]bool{}
			for _, alias := range rows {
				aliases[string(alias)] = true
			}
			for _, alias := range journal.aliases {
				if !aliases[alias] {
					return invalid("CFP refresh cannot change journal alias ownership")
				}
			}
			if _, err = queries.DeleteNotices(ctx, connection, cfpdb.DeleteNoticesParams{SourceKey: sqlite.Text(lease.SourceKey), JournalKey: sqlite.Text(key)}); err != nil {
				return err
			}
			if err = insertNotices(ctx, connection, key, lease.SourceKey, journal.notices); err != nil {
				return err
			}
			url, statement := emptyFields(journal)
			if _, err = queries.UpdateJournal(ctx, connection, cfpdb.UpdateJournalParams{JournalKey: sqlite.Text(key), CheckedOn: sqlite.Text(journal.checkedOn), SourceURL: (*sqlite.Text)(url), SourceStatement: (*sqlite.Text)(statement)}); err != nil {
				return err
			}
		}
		return nil
	})
}
