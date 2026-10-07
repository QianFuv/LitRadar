package index

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"reflect"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// ContentWriteOutcome counts only the work committed by one canonical content transaction.
type ContentWriteOutcome struct {
	ArticlesSeen         uint64 `json:"articles_seen"`
	ArticlesChanged      uint64 `json:"articles_changed"`
	IdentityAliasesAdded uint64 `json:"identity_aliases_added"`
	ChangeEventsEmitted  uint64 `json:"change_events_emitted"`
}

// ArticleIdCollision prevents a new hash allocation from replacing an unrelated existing row.
type ArticleIdCollision struct{ ArticleId int64 }

func (err *ArticleIdCollision) Error() string {
	return fmt.Sprintf("canonical article ID collision for %d", err.ArticleId)
}

// WriteContentBatch commits canonical content, projections, aliases and outbox events atomically.
func WriteContentBatch(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry, batch domain.ProviderBatch, revision, createdAt string) (ContentWriteOutcome, error) {
	return WriteContentBatchWithEvents(ctx, connection, catalog, batch, revision, createdAt, true)
}

// WriteContentBatchWithEvents suppresses outbox emission only when orchestration explicitly requests it.
func WriteContentBatchWithEvents(ctx context.Context, connection *sql.Conn, catalog domain.JournalCatalogEntry, batch domain.ProviderBatch, revision, createdAt string, shouldRecordEvents bool) (ContentWriteOutcome, error) {
	if err := provider.ValidateProviderBatch(catalog, batch); err != nil {
		return ContentWriteOutcome{}, err
	}
	if !isTrimmed(revision) {
		return ContentWriteOutcome{}, &InvalidContentSchema{"content revision must be non-empty and trimmed"}
	}
	outcome := ContentWriteOutcome{ArticlesSeen: uint64(len(batch.Articles))}
	err := immediate(ctx, connection, func() error {
		writer, err := prepareWriter(ctx, connection, shouldRecordEvents)
		if err != nil {
			return err
		}
		defer writer.close()
		if err := claimCatalog(ctx, connection, catalog); err != nil {
			return err
		}
		refresh, err := upsertJournal(ctx, connection, catalog)
		if err != nil {
			return err
		}
		journal := JournalId(catalog.CatalogId)
		for _, issue := range batch.Issues {
			if _, err := writer.upsertIssue(journal, issue); err != nil {
				return err
			}
		}
		for _, article := range batch.Articles {
			if err := writer.writeArticle(catalog, journal, article, revision, createdAt, &outcome); err != nil {
				return err
			}
		}
		issues := make([]int64, 0, len(writer.observedIssues))
		for issue := range writer.observedIssues {
			issues = append(issues, issue)
		}
		slices.Sort(issues)
		for _, issue := range issues {
			if _, err := connection.ExecContext(ctx, `UPDATE issues SET date=COALESCE((SELECT date FROM articles WHERE issue_id=?1 AND date IS NOT NULL ORDER BY length(date) DESC,date LIMIT 1),date) WHERE issue_id=?1`, issue); err != nil {
				return err
			}
		}
		return refreshJournal(ctx, connection, catalog, refresh)
	})
	if err != nil {
		return ContentWriteOutcome{}, err
	}
	return outcome, nil
}

type contentWriter struct {
	ctx                          context.Context
	connection                   *sql.Conn
	isSimple, shouldRecordEvents bool
	observedIssues               map[int64]bool
	statements                   map[string]*sql.Stmt
}

func prepareWriter(ctx context.Context, connection *sql.Conn, shouldRecordEvents bool) (*contentWriter, error) {
	isSimple, err := search.UsesSimple(ctx, connection)
	if err != nil {
		return nil, err
	}
	writer := &contentWriter{ctx, connection, isSimple, shouldRecordEvents, map[int64]bool{}, map[string]*sql.Stmt{}}
	for _, statement := range []string{journalProjectionSql, journalUpsertSql, issueUpsertSql, identityLookupSql, articleLookupSql, retractionLookupSql, articleUpsertSql, retractionDeleteSql, retractionInsertSql, listingUpsertSql, searchDeleteSql, searchInsertSql, identityClaimSql, changeEventInsertSql} {
		prepared, err := connection.PrepareContext(ctx, statement)
		if err != nil {
			writer.close()
			return nil, err
		}
		writer.statements[statement] = prepared
	}
	return writer, nil
}

func (writer *contentWriter) close() {
	for _, statement := range writer.statements {
		statement.Close()
	}
}
func (writer *contentWriter) exec(statement string, args ...any) (sql.Result, error) {
	return writer.statements[statement].ExecContext(writer.ctx, args...)
}
func (writer *contentWriter) queryRow(statement string, args ...any) *sql.Row {
	return writer.statements[statement].QueryRowContext(writer.ctx, args...)
}

func (writer *contentWriter) upsertIssue(journal int64, issue domain.IssueDraft) (*int64, error) {
	id := IssueId(journal, issue)
	if id == nil {
		return nil, nil
	}
	if _, err := writer.exec(issueUpsertSql, *id, journal, issue.PublicationYear, issue.Title, issue.Volume, issue.Number, issue.Date); err != nil {
		return nil, err
	}
	writer.observedIssues[*id] = true
	return id, nil
}

func (writer *contentWriter) loadArticle(id int64) (*domain.ArticleDraft, *int64, error) {
	var catalog, title, authors sqlite.Text
	var year, access, inPress, issue sqlite.OptionalInteger
	var date, issueTitle, volume, number, start, end, abstract, doi, pmid sqlite.OptionalText
	err := writer.queryRow(articleLookupSql, id).Scan(&catalog, &title, &year, &date, &issueTitle, &volume, &number, &authors, &start, &end, &abstract, &doi, &pmid, &access, &inPress, &issue)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil, nil
	}
	if err != nil {
		return nil, nil, err
	}
	rows, err := writer.statements[retractionLookupSql].QueryContext(writer.ctx, id)
	if err != nil {
		return nil, nil, err
	}
	retractions := []string{}
	for rows.Next() {
		var value sqlite.Text
		if err := rows.Scan(&value); err != nil {
			rows.Close()
			return nil, nil, err
		}
		retractions = append(retractions, string(value))
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return nil, nil, err
	}
	names, err := decodeCanonicalAuthors(string(authors))
	if err != nil {
		return nil, nil, err
	}
	return &domain.ArticleDraft{CatalogId: string(catalog), Title: string(title), PublicationYear: year.Value, Date: date.Value, IssueTitle: issueTitle.Value, Volume: volume.Value, IssueNumber: number.Value, Authors: names, StartPage: start.Value, EndPage: end.Value, AbstractText: abstract.Value, Doi: doi.Value, Pmid: pmid.Value, OpenAccess: integerBool(access.Value), InPress: integerBool(inPress.Value), RetractionDois: retractions}, issue.Value, nil
}

func (writer *contentWriter) writeArticle(catalog domain.JournalCatalogEntry, journal int64, article domain.ArticleDraft, revision, createdAt string, outcome *ContentWriteOutcome) error {
	incoming := ArticleIdentityKeys(article)
	aliases, err := writer.lookupIdentityAliases(incoming)
	if err != nil {
		return err
	}
	resolution, err := ResolveArticleIdentity(article, aliases)
	if err != nil {
		return err
	}
	existing, previousIssue, err := writer.loadResolvedArticle(resolution)
	if err != nil {
		return err
	}
	merged, err := mergeStoredArticle(existing, article)
	if err != nil {
		return err
	}
	issue, err := writer.upsertIssue(journal, domain.IssueDraft{CatalogId: merged.CatalogId, PublicationYear: merged.PublicationYear, Title: merged.IssueTitle, Volume: merged.Volume, Number: merged.IssueNumber, Date: merged.Date})
	if err != nil {
		return err
	}
	isChanged := existing == nil || !reflect.DeepEqual(*existing, merged) || !reflect.DeepEqual(previousIssue, issue)
	if isChanged {
		if err := writer.persistArticleChange(catalog, resolution.ArticleId, journal, previousIssue, issue, existing, merged, revision, createdAt, outcome); err != nil {
			return err
		}
	}
	return writer.claimArticleAliases(incoming, merged, resolution.ArticleId, outcome)
}

func (writer *contentWriter) upsertArticle(id, journal int64, issue *int64, article domain.ArticleDraft) error {
	authors, err := jsonArray(article.Authors)
	if err != nil {
		return err
	}
	if _, err := writer.exec(articleUpsertSql, id, journal, issue, article.Title, article.PublicationYear, article.Date, authors, article.StartPage, article.EndPage, article.AbstractText, article.Doi, article.Pmid, article.OpenAccess, article.InPress); err != nil {
		return err
	}
	if _, err := writer.exec(retractionDeleteSql, id); err != nil {
		return err
	}
	for _, doi := range article.RetractionDois {
		if _, err := writer.exec(retractionInsertSql, id, doi); err != nil {
			return err
		}
	}
	return nil
}

func (writer *contentWriter) projectArticle(id, journal int64, issue *int64, catalog domain.JournalCatalogEntry, article domain.ArticleDraft) error {
	if _, err := writer.exec(listingUpsertSql, id, journal, issue, article.PublicationYear, article.Date, article.OpenAccess, article.InPress, article.Doi, article.Pmid, catalog.Area); err != nil {
		return err
	}
	if _, err := writer.exec(searchDeleteSql, id); err != nil {
		return err
	}
	authors := make([]string, len(article.Authors))
	for position, author := range article.Authors {
		authors[position] = author.DisplayName
	}
	values := []any{id}
	for _, value := range []string{article.Title, optionalString(article.AbstractText), optionalString(article.Doi), optionalString(article.Pmid), strings.Join(authors, "; "), catalog.Title} {
		values = append(values, search.PrepareText(value, writer.isSimple))
	}
	_, err := writer.exec(searchInsertSql, values...)
	return err
}

func (writer *contentWriter) recordEvent(revision string, id int64, kind string, journal int64, issue *int64, inPress *bool, createdAt string) (uint64, error) {
	isInPress := inPress != nil && *inPress
	result, err := writer.exec(changeEventInsertSql, revision, id, kind, journal, issue, isInPress, createdAt)
	if err != nil {
		return 0, err
	}
	count, err := result.RowsAffected()
	return uint64(count), err
}

func integerBool(value *int64) *bool {
	if value == nil {
		return nil
	}
	return ptr(*value != 0)
}

func (writer *contentWriter) lookupIdentityAliases(incoming []ArticleIdentityKey) (map[ArticleIdentityKey]int64, error) {
	aliases := map[ArticleIdentityKey]int64{}
	for _, key := range incoming {
		var owner sqlite.Integer
		err := writer.queryRow(identityLookupSql, key.Kind, key.Value).Scan(&owner)
		if err == nil {
			aliases[key] = int64(owner)
		} else if !errors.Is(err, sql.ErrNoRows) {
			return nil, err
		}
	}
	return aliases, nil
}

func (writer *contentWriter) claimArticleAliases(incoming []ArticleIdentityKey, merged domain.ArticleDraft, id int64, outcome *ContentWriteOutcome) error {
	keys := append(incoming, ArticleIdentityKeys(merged)...)
	sortIdentityKeys(keys)
	keys = slices.Compact(keys)
	for _, key := range keys {
		var owner, wasInserted sqlite.Integer
		if err := writer.queryRow(identityClaimSql, key.Kind, key.Value, id).Scan(&owner, &wasInserted); err != nil {
			return err
		}
		if wasInserted != 0 {
			outcome.IdentityAliasesAdded++
		}
		if int64(owner) != id {
			return &IdentityConflict{[]int64{int64(owner), id}}
		}
	}
	return nil
}

func (writer *contentWriter) persistArticleChange(catalog domain.JournalCatalogEntry, id, journal int64, previousIssue, issue *int64, existing *domain.ArticleDraft, merged domain.ArticleDraft, revision, createdAt string, outcome *ContentWriteOutcome) error {
	if err := writer.upsertArticle(id, journal, issue, merged); err != nil {
		return err
	}
	if err := writer.projectArticle(id, journal, issue, catalog, merged); err != nil {
		return err
	}
	if writer.shouldRecordEvents {
		if err := writer.recordArticleChange(id, journal, previousIssue, issue, existing, merged, revision, createdAt, outcome); err != nil {
			return err
		}
	}
	if !reflect.DeepEqual(previousIssue, issue) && previousIssue != nil {
		writer.observedIssues[*previousIssue] = true
		if _, err := writer.connection.ExecContext(writer.ctx, "DELETE FROM issues WHERE issue_id=?1 AND NOT EXISTS(SELECT 1 FROM articles WHERE issue_id=?1)", *previousIssue); err != nil {
			return err
		}
	}
	outcome.ArticlesChanged++
	return nil
}

func (writer *contentWriter) recordArticleChange(id, journal int64, previousIssue, issue *int64, existing *domain.ArticleDraft, merged domain.ArticleDraft, revision, createdAt string, outcome *ContentWriteOutcome) error {
	if existing != nil && (!reflect.DeepEqual(previousIssue, issue) || !reflect.DeepEqual(existing.InPress, merged.InPress)) {
		count, err := writer.recordEvent(revision, id, "remove", journal, previousIssue, existing.InPress, createdAt)
		if err != nil {
			return err
		}
		outcome.ChangeEventsEmitted += count
	}
	count, err := writer.recordEvent(revision, id, "upsert", journal, issue, merged.InPress, createdAt)
	if err != nil {
		return err
	}
	outcome.ChangeEventsEmitted += count
	return nil
}

func (writer *contentWriter) loadResolvedArticle(resolution ArticleIdentityResolution) (*domain.ArticleDraft, *int64, error) {
	existing, previousIssue, err := writer.loadArticle(resolution.ArticleId)
	if err != nil {
		return nil, nil, err
	}
	if !resolution.IsExisting && existing != nil {
		return nil, nil, &ArticleIdCollision{resolution.ArticleId}
	}
	return existing, previousIssue, nil
}

func mergeStoredArticle(existing *domain.ArticleDraft, article domain.ArticleDraft) (domain.ArticleDraft, error) {
	merged := article
	if existing != nil {
		value, err := MergeResolvedArticleDrafts(*existing, article)
		merged = value
		if err != nil {
			return merged, err
		}
	}
	merged.Authors = append([]domain.ArticleAuthorDraft{}, merged.Authors...)
	merged.RetractionDois = append([]string{}, merged.RetractionDois...)
	return merged, nil
}
