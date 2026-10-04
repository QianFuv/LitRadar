package cfp

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"math"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
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
	err = connection.QueryRowContext(ctx, `UPDATE cfp_sources SET generation=generation+1,lease_expires_at=?2,last_attempt=?3,last_error=NULL,status='refreshing' WHERE source_key=?1 RETURNING generation`, sourceKey, expires, now).Scan(&generation)
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
	result, err := repository.database.ExecContext(ctx, `UPDATE cfp_sources SET status=?3,last_error=?4,lease_expires_at=NULL WHERE source_key=?1 AND generation=?2 AND lease_expires_at IS NOT NULL`, lease.SourceKey, lease.Generation, status, reason)
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
		bindings := []string{}
		if err := readRows(ctx, connection, `SELECT journal_key FROM cfp_source_journals WHERE source_key=?`, []any{lease.SourceKey}, func(row *sql.Rows) error {
			var key sqlite.Text
			if err := row.Scan(&key); err != nil {
				return err
			}
			bindings = append(bindings, string(key))
			return nil
		}); err != nil {
			return err
		}
		slices.Sort(bindings)
		if len(bindings) == 0 || !slices.Equal(bindings, sortedKeys(prepared)) {
			return invalid("CFP publication does not cover every registered journal binding")
		}
		result, err := connection.ExecContext(ctx, `UPDATE cfp_sources SET capture=?3,capture_format=?4,content_hash=?5,url=?6,parser_version=?7,config_version=?8,last_success=CASE WHEN ?10 IS NULL THEN ?9 ELSE last_success END,last_error=?10,status=CASE WHEN ?10 IS NULL THEN 'success' ELSE 'failed' END,revision=revision+1,lease_expires_at=NULL WHERE source_key=?1 AND generation=?2 AND lease_expires_at>=?9`, lease.SourceKey, lease.Generation, publication.Capture, publication.CaptureFormat, digest([]byte(publication.Capture)), publication.SourceUrl, domain.ParserVersion, publication.ConfigVersion, now, warning)
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
			aliases := map[string]bool{}
			if err := readRows(ctx, connection, `SELECT catalog_id FROM cfp_journal_aliases WHERE journal_key=?`, []any{key}, func(row *sql.Rows) error {
				var alias sqlite.Text
				if err := row.Scan(&alias); err != nil {
					return err
				}
				aliases[string(alias)] = true
				return nil
			}); err != nil {
				return err
			}
			for _, alias := range journal.aliases {
				if !aliases[alias] {
					return invalid("CFP refresh cannot change journal alias ownership")
				}
			}
			if _, err = connection.ExecContext(ctx, `DELETE FROM cfp_notices WHERE source_key=? AND journal_key=?`, lease.SourceKey, key); err != nil {
				return err
			}
			if err = insertNotices(ctx, connection, key, lease.SourceKey, journal.notices); err != nil {
				return err
			}
			url, statement := emptyFields(journal)
			if _, err = connection.ExecContext(ctx, `UPDATE cfp_journals SET checked_on=?2,source_url=?3,source_statement=?4 WHERE journal_key=?1`, key, journal.checkedOn, url, statement); err != nil {
				return err
			}
		}
		return nil
	})
}
