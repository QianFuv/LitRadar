package index

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"reflect"
	"regexp"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/provider"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/contentdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/writerdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// ContentWriteOutcome counts only the work committed by one canonical content transaction.
type ContentWriteOutcome struct {
	ArticlesSeen         uint64 `json:"articles_seen"`
	ArticlesChanged      uint64 `json:"articles_changed"`
	IdentityAliasesAdded uint64 `json:"identity_aliases_added"`
	ChangeEventsEmitted  uint64 `json:"change_events_emitted"`
}

// ArticleIDCollision prevents a new hash allocation from replacing an unrelated existing row.
type ArticleIDCollision struct{ ArticleID int64 }

func (err *ArticleIDCollision) Error() string {
	return fmt.Sprintf("canonical article ID collision for %d", err.ArticleID)
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
		journal := JournalID(catalog.CatalogID)
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
			if _, err := contentQueries.UpdateIssueDate(ctx, connection, contentdb.UpdateIssueDateParams{IssueID: new(sqlite.Integer(issue))}); err != nil {
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
	ctx                    context.Context
	connection             *sql.Conn
	shouldRecordEvents     bool
	observedIssues         map[int64]bool
	statements             map[string]*sql.Stmt
	queries                *writerdb.Queries
	journalTitle           string
	isJournalTitlePrepared bool
}

// prepareWriter binds shared statements to one content transaction.
func prepareWriter(ctx context.Context, connection *sql.Conn, shouldRecordEvents bool) (*contentWriter, error) {
	writer := &contentWriter{ctx: ctx, connection: connection, shouldRecordEvents: shouldRecordEvents, observedIssues: map[int64]bool{}, statements: map[string]*sql.Stmt{}}
	var err error
	writer.queries, err = writerdb.Prepare(ctx, &statementTracker{Conn: connection, statements: writer.statements})
	if err != nil {
		writer.close()
		return nil, err
	}
	for _, statement := range []string{searchDeleteSQL, searchInsertSQL, identityClaimSQL} {
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
		if err := statement.Close(); err != nil {
			slog.WarnContext(writer.ctx, "storage.statement.close_failed", "component", "storage")
		}
	}
}
func (writer *contentWriter) exec(statement string, args ...any) (sql.Result, error) {
	return writer.statements[statement].ExecContext(writer.ctx, args...)
}
func (writer *contentWriter) queryRow(statement string, args ...any) *sql.Row {
	return writer.statements[statement].QueryRowContext(writer.ctx, args...)
}

func (writer *contentWriter) upsertIssue(journal int64, issue domain.IssueDraft) (*int64, error) {
	id := IssueID(journal, issue)
	if id == nil {
		return nil, nil
	}
	if _, err := writer.queries.UpsertIssue(writer.ctx, writerdb.UpsertIssueParams{IssueID: sqlite.Integer(*id), JournalID: sqlite.Integer(journal), PublicationYear: (*sqlite.Integer)(issue.PublicationYear), Title: (*sqlite.Text)(issue.Title), Volume: (*sqlite.Text)(issue.Volume), Number: (*sqlite.Text)(issue.Number), Date: (*sqlite.Text)(issue.Date)}); err != nil {
		return nil, err
	}
	writer.observedIssues[*id] = true
	return id, nil
}

func (writer *contentWriter) loadArticle(id int64) (*domain.ArticleDraft, *int64, error) {
	row, err := writer.queries.Article(writer.ctx, writerdb.ArticleParams{ArticleID: sqlite.Integer(id)})
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil, nil
	}
	if err != nil {
		return nil, nil, err
	}
	storedRetractions, err := writer.queries.Retractions(writer.ctx, writerdb.RetractionsParams{ArticleID: sqlite.Integer(id)})
	if err != nil {
		return nil, nil, err
	}
	retractions := make([]string, 0, len(storedRetractions))
	for _, value := range storedRetractions {
		retractions = append(retractions, string(value))
	}
	names, err := decodeCanonicalAuthors(string(row.AuthorsJSON))
	if err != nil {
		return nil, nil, err
	}
	return &domain.ArticleDraft{CatalogID: string(row.CatalogID), Title: string(row.Title), PublicationYear: (*int64)(row.PublicationYear), Date: (*string)(row.Date), IssueTitle: (*string)(row.Title_2), Volume: (*string)(row.Volume), IssueNumber: (*string)(row.Number), Authors: names, StartPage: (*string)(row.StartPage), EndPage: (*string)(row.EndPage), AbstractText: (*string)(row.AbstractText), Doi: (*string)(row.DOI), Pmid: (*string)(row.PMID), OpenAccess: integerBool((*int64)(row.OpenAccess)), InPress: integerBool((*int64)(row.InPress)), RetractionDois: retractions}, (*int64)(row.IssueID), nil
}

func (writer *contentWriter) writeArticle(catalog domain.JournalCatalogEntry, journal int64, article domain.ArticleDraft, revision, createdAt string, outcome *ContentWriteOutcome) error {
	incoming := ArticleIdentityKeys(article)
	aliases, err := writer.lookupIdentityAliases(incoming)
	if err != nil {
		return err
	}
	resolution, preservedAlias, err := writer.resolveStoredArticleIdentity(article, incoming, aliases)
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
	issue, err := writer.upsertIssue(journal, domain.IssueDraft{CatalogID: merged.CatalogID, PublicationYear: merged.PublicationYear, Title: merged.IssueTitle, Volume: merged.Volume, Number: merged.IssueNumber, Date: merged.Date})
	if err != nil {
		return err
	}
	isChanged := existing == nil || !reflect.DeepEqual(*existing, merged) || !reflect.DeepEqual(previousIssue, issue)
	if isChanged {
		if err := writer.persistArticleChange(catalog, resolution.ArticleID, journal, previousIssue, issue, existing, merged, revision, createdAt, outcome); err != nil {
			return err
		}
	}
	return writer.claimArticleAliases(incoming, aliases, merged, resolution.ArticleID, preservedAlias, outcome)
}

// resolveStoredArticleIdentity preserves an exact legacy duplicate's weak alias without merging IDs.
func (writer *contentWriter) resolveStoredArticleIdentity(article domain.ArticleDraft, keys []ArticleIdentityKey, aliases map[ArticleIdentityKey]int64) (ArticleIdentityResolution, *ArticleIdentityKey, error) {
	resolution, err := resolveArticleIdentity(keys, aliases)
	if _, ok := errors.AsType[*IdentityConflict](err); !ok {
		return resolution, nil, err
	}
	var doiKey, weakKey ArticleIdentityKey
	for _, key := range keys {
		switch key.Kind {
		case "doi":
			doiKey = key
		case "bibliographic":
			weakKey = key
		}
	}
	strongID, hasDoi := aliases[doiKey]
	weakID, hasWeak := aliases[weakKey]
	if !hasDoi || !hasWeak || strongID == weakID {
		return ArticleIdentityResolution{}, nil, err
	}
	for key, owner := range aliases {
		if key != weakKey && owner != strongID {
			return ArticleIdentityResolution{}, nil, err
		}
	}
	strong, _, loadError := writer.loadArticle(strongID)
	if loadError != nil {
		return ArticleIdentityResolution{}, nil, loadError
	}
	weak, _, loadError := writer.loadArticle(weakID)
	if loadError != nil {
		return ArticleIdentityResolution{}, nil, loadError
	}
	if !matchesLegacyPublication(strong, article) || !matchesLegacyPublication(weak, article) || weak.Doi != nil || weak.Pmid != nil || strong.Doi == nil || !reflect.DeepEqual(domain.NormalizeDoi(*strong.Doi), new(doiKey.Value)) || !reflect.DeepEqual(bibliographicFingerprint(*weak), new(weakKey.Value)) {
		return ArticleIdentityResolution{}, nil, err
	}
	if strong.Pmid != nil && article.Pmid != nil && !reflect.DeepEqual(domain.NormalizePmid(*strong.Pmid), domain.NormalizePmid(*article.Pmid)) {
		return ArticleIdentityResolution{}, nil, err
	}
	pmid := firstValue(article.Pmid, strong.Pmid)
	var hasStrongConflict bool
	loadError = func() error {
		queryResult, err := contentQueries.HasStoredArticleIdentity(writer.ctx, writer.connection, contentdb.HasStoredArticleIdentityParams{ArticleID: sqlite.Integer(weakID), ArticleID_2: sqlite.Integer(strongID), IdentityValue: sqlite.Text(doiKey.Value), IdentityValue_2: sqlite.Text(optionalString(pmid))})
		if err != nil {
			return err
		}
		hasStrongConflict = queryResult
		return nil
	}()
	if loadError != nil {
		return ArticleIdentityResolution{}, nil, loadError
	}
	if hasStrongConflict {
		return ArticleIdentityResolution{}, nil, err
	}
	return ArticleIdentityResolution{strongID, true, doiKey}, &weakKey, nil
}

// matchesLegacyPublication requires exact publication evidence and rejects populated field contradictions.
func matchesLegacyPublication(stored *domain.ArticleDraft, incoming domain.ArticleDraft) bool {
	if stored == nil || stored.CatalogID != incoming.CatalogID || legacyPublicationTitle(stored.Title) != legacyPublicationTitle(incoming.Title) || stored.PublicationYear == nil || incoming.PublicationYear == nil || *stored.PublicationYear != *incoming.PublicationYear {
		return false
	}
	storedFields := []*string{stored.Volume, stored.IssueNumber, stored.StartPage, stored.EndPage}
	incomingFields := []*string{incoming.Volume, incoming.IssueNumber, incoming.StartPage, incoming.EndPage}
	for position, field := range storedFields {
		if field != nil && (incomingFields[position] == nil || domain.NormalizeBibliographicLabel(*field) != domain.NormalizeBibliographicLabel(*incomingFields[position])) {
			return false
		}
	}
	return true
}

var legacyMathLabelSpacing = regexp.MustCompile(`\b([a-z]) ([0-9]+)\b`)

// legacyPublicationTitle normalizes isolated mathematical labels such as L 1 and L1.
func legacyPublicationTitle(title string) string {
	return legacyMathLabelSpacing.ReplaceAllString(domain.NormalizeBibliographicText(title), "${1}${2}")
}

func (writer *contentWriter) upsertArticle(id, journal int64, issue *int64, article domain.ArticleDraft) error {
	authors, err := jsonArray(article.Authors)
	if err != nil {
		return err
	}
	if _, err := writer.queries.UpsertArticle(writer.ctx, writerdb.UpsertArticleParams{ArticleID: sqlite.Integer(id), JournalID: sqlite.Integer(journal), IssueID: (*sqlite.Integer)(issue), Title: sqlite.Text(article.Title), PublicationYear: (*sqlite.Integer)(article.PublicationYear), Date: (*sqlite.Text)(article.Date), AuthorsJSON: sqlite.Text(authors), StartPage: (*sqlite.Text)(article.StartPage), EndPage: (*sqlite.Text)(article.EndPage), AbstractText: (*sqlite.Text)(article.AbstractText), DOI: (*sqlite.Text)(article.Doi), PMID: (*sqlite.Text)(article.Pmid), OpenAccess: booleanInteger(article.OpenAccess), InPress: booleanInteger(article.InPress)}); err != nil {
		return err
	}
	if _, err := writer.queries.DeleteRetractions(writer.ctx, writerdb.DeleteRetractionsParams{ArticleID: sqlite.Integer(id)}); err != nil {
		return err
	}
	for _, doi := range article.RetractionDois {
		if _, err := writer.queries.InsertRetraction(writer.ctx, writerdb.InsertRetractionParams{ArticleID: sqlite.Integer(id), RetractionDOI: sqlite.Text(doi)}); err != nil {
			return err
		}
	}
	return nil
}

// projectArticle publishes changed fields with bounded SQL arguments and a shared journal-title projection.
func (writer *contentWriter) projectArticle(id, journal int64, issue *int64, catalog domain.JournalCatalogEntry, article domain.ArticleDraft) error {
	if _, err := writer.queries.UpsertListing(writer.ctx, writerdb.UpsertListingParams{ArticleID: sqlite.Integer(id), JournalID: sqlite.Integer(journal), IssueID: (*sqlite.Integer)(issue), PublicationYear: (*sqlite.Integer)(article.PublicationYear), Date: (*sqlite.Text)(article.Date), OpenAccess: booleanInteger(article.OpenAccess), InPress: booleanInteger(article.InPress), DOI: (*sqlite.Text)(article.Doi), PMID: (*sqlite.Text)(article.Pmid), Area: (*sqlite.Text)(catalog.Area)}); err != nil {
		return err
	}
	if _, err := writer.exec(searchDeleteSQL, id); err != nil {
		return err
	}
	authors := make([]string, len(article.Authors))
	for position, author := range article.Authors {
		authors[position] = author.DisplayName
	}
	values := make([]any, 1, 7)
	values[0] = id
	for _, value := range []string{article.Title, optionalString(article.AbstractText), optionalString(article.Doi), optionalString(article.Pmid), strings.Join(authors, "; ")} {
		values = append(values, search.PrepareText(value))
	}
	if !writer.isJournalTitlePrepared {
		writer.journalTitle = search.PrepareText(catalog.Title)
		writer.isJournalTitlePrepared = true
	}
	values = append(values, writer.journalTitle)
	_, err := writer.exec(searchInsertSQL, values...)
	return err
}

func (writer *contentWriter) recordEvent(revision string, id int64, kind string, journal int64, issue *int64, inPress *bool, createdAt string) (uint64, error) {
	isInPress := inPress != nil && *inPress
	result, err := writer.queries.InsertChangeEvent(writer.ctx, writerdb.InsertChangeEventParams{ContentRevision: sqlite.Text(revision), ArticleID: sqlite.Integer(id), ChangeKind: sqlite.Text(kind), JournalID: sqlite.Integer(journal), IssueID: (*sqlite.Integer)(issue), InPress: sqlite.Integer(dbtypes.Boolean(isInPress)), CreatedAt: sqlite.Text(createdAt)})
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
	return new(*value != 0)
}

func (writer *contentWriter) lookupIdentityAliases(incoming []ArticleIdentityKey) (map[ArticleIdentityKey]int64, error) {
	aliases := map[ArticleIdentityKey]int64{}
	for _, key := range incoming {
		owner, err := writer.queries.IdentityOwner(writer.ctx, writerdb.IdentityOwnerParams{IdentityKind: sqlite.Text(key.Kind), IdentityValue: sqlite.Text(key.Value)})
		if err == nil {
			aliases[key] = int64(owner)
		} else if !errors.Is(err, sql.ErrNoRows) {
			return nil, err
		}
	}
	return aliases, nil
}

// claimArticleAliases rechecks every owner while reusing existence known within the immediate transaction.
func (writer *contentWriter) claimArticleAliases(incoming []ArticleIdentityKey, knownAliases map[ArticleIdentityKey]int64, merged domain.ArticleDraft, id int64, preservedAlias *ArticleIdentityKey, outcome *ContentWriteOutcome) error {
	keys := append(slices.Clone(incoming), ArticleIdentityKeys(merged)...)
	sortIdentityKeys(keys)
	keys = slices.Compact(keys)
	for _, key := range keys {
		claimID := id
		if preservedAlias != nil && key == *preservedAlias {
			claimID = knownAliases[key]
		}
		var owner, wasInserted sqlite.Integer
		if _, hasKnownOwner := knownAliases[key]; hasKnownOwner {
			var err error
			owner, err = writer.queries.ClaimArticleAlias(writer.ctx, writerdb.ClaimArticleAliasParams{IdentityKind: sqlite.Text(key.Kind), IdentityValue: sqlite.Text(key.Value), ArticleID: sqlite.Integer(claimID)})
			if err != nil {
				return err
			}
		} else {
			if err := writer.queryRow(identityClaimSQL, key.Kind, key.Value, claimID).Scan(&owner, &wasInserted); err != nil {
				return err
			}
		}
		if wasInserted != 0 {
			outcome.IdentityAliasesAdded++
		}
		if int64(owner) != claimID {
			return &IdentityConflict{[]int64{int64(owner), claimID}}
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
		if _, err := contentQueries.DeleteEmptyIssue(writer.ctx, writer.connection, contentdb.DeleteEmptyIssueParams{IssueID: sqlite.Integer(*previousIssue)}); err != nil {
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
	existing, previousIssue, err := writer.loadArticle(resolution.ArticleID)
	if err != nil {
		return nil, nil, err
	}
	if !resolution.IsExisting && existing != nil {
		return nil, nil, &ArticleIDCollision{resolution.ArticleID}
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

func booleanInteger(value *bool) *sqlite.Integer {
	if value == nil {
		return nil
	}
	return new(sqlite.Integer(dbtypes.Boolean(*value)))
}

// statementTracker retains ownership of partially prepared sqlc query sets.
type statementTracker struct {
	*sql.Conn
	statements map[string]*sql.Stmt
}

// PrepareContext records each statement before sqlc can return a later preparation error.
func (tracker *statementTracker) PrepareContext(ctx context.Context, query string) (*sql.Stmt, error) {
	statement, err := tracker.Conn.PrepareContext(ctx, query)
	if err != nil {
		return nil, err
	}
	tracker.statements[query] = statement
	return statement, nil
}
