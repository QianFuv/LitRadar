package query

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	native "github.com/mattn/go-sqlite3"
)

// ErrInvalidCursor rejects malformed article pagination anchors.
var ErrInvalidCursor = errors.New("Invalid cursor")

// ErrUnsupportedArticleSort limits article pagination to its supported date ordering.
var ErrUnsupportedArticleSort = errors.New("Articles only support sort=date:desc or date:asc")

// ErrInvalidSearchExpression hides native SQL diagnostics for invalid public FTS expressions.
var ErrInvalidSearchExpression = errors.New("Invalid search expression")

// ArticleListParams preserves projection filters, optional counts and date/identifier keyset pagination.
type ArticleListParams struct {
	JournalId    []int64               `json:"journal_id"`
	IssueId      *int64                `json:"issue_id"`
	Year         *int64                `json:"year"`
	Area         []string              `json:"area"`
	Ratings      domain.JournalRatings `json:"ratings"`
	InPress      *bool                 `json:"in_press"`
	OpenAccess   *bool                 `json:"open_access"`
	DateFrom     *string               `json:"date_from"`
	DateTo       *string               `json:"date_to"`
	Doi          *string               `json:"doi"`
	Pmid         *string               `json:"pmid"`
	Query        *string               `json:"q"`
	SearchMode   domain.SearchMode     `json:"search_mode"`
	Sort         *string               `json:"sort"`
	Limit        int64                 `json:"limit"`
	Offset       int64                 `json:"offset"`
	Cursor       *string               `json:"cursor"`
	IncludeTotal *bool                 `json:"include_total"`
}

// DefaultArticleListParams supplies the original repository's article-specific defaults.
func DefaultArticleListParams() ArticleListParams {
	sort := "date:desc"
	return ArticleListParams{Limit: 50, Sort: &sort, SearchMode: domain.SearchSimple}
}

func validateCharacters(label, value string, maximum int) error {
	if utf8.RuneCountInString(value) > maximum {
		return InvalidInput{fmt.Sprintf("%s must be at most %d characters", label, maximum)}
	}
	return nil
}

func validateArticleInput(name *string, params ArticleListParams) error {
	if name != nil {
		if err := validateCharacters("db", *name, 255); err != nil {
			return err
		}
	}
	count := len(params.JournalId) + len(params.Area)
	for _, group := range ratingGroups(params.Ratings) {
		count += len(group.values)
	}
	if count > 500 {
		return InvalidInput{"search filters must contain at most 500 items"}
	}
	for _, area := range params.Area {
		if err := validateCharacters("area", area, 2048); err != nil {
			return err
		}
	}
	for _, field := range []struct {
		label string
		value *string
	}{{"date_from", params.DateFrom}, {"date_to", params.DateTo}, {"doi", params.Doi}, {"pmid", params.Pmid}, {"q", params.Query}, {"sort", params.Sort}, {"cursor", params.Cursor}} {
		if field.value != nil {
			if err := validateCharacters(field.label, *field.value, 2048); err != nil {
				return err
			}
		}
	}
	return nil
}

func (filter *filter) list(column string, values []any) {
	if len(values) == 0 {
		return
	}
	filter.add(column+" IN ("+placeholders(len(values))+")", values...)
}

func placeholders(count int) string { return strings.TrimSuffix(strings.Repeat("?,", count), ",") }
func arguments[Value any](values []Value) []any {
	result := make([]any, len(values))
	for index, value := range values {
		result[index] = value
	}
	return result
}
func (filter *filter) optionalText(clause string, value *string) {
	if value != nil {
		if trimmed := strings.TrimSpace(*value); trimmed != "" {
			filter.add(clause, trimmed)
		}
	}
}
func (filter *filter) optionalInteger(clause string, value *int64) {
	if value != nil {
		filter.add(clause, *value)
	}
}
func (filter *filter) optionalBoolean(clause string, value *bool) {
	if value != nil {
		integer := 0
		if *value {
			integer = 1
		}
		filter.add(clause, integer)
	}
}

func preparedMatch(params ArticleListParams) string {
	if params.Query == nil {
		return ""
	}
	query := strings.TrimSpace(*params.Query)
	if query == "" {
		return ""
	}
	if params.SearchMode == domain.SearchAdvanced {
		return query
	}
	return `"` + strings.ReplaceAll(query, `"`, `""`) + `"`
}

func classifySearchError(err error, params ArticleListParams) error {
	if err == nil || params.SearchMode != domain.SearchAdvanced || params.Query == nil || strings.TrimSpace(*params.Query) == "" {
		return err
	}
	var failure native.Error
	if errors.As(err, &failure) {
		message := asciiLower(failure.Error())
		if strings.Contains(message, "fts5: syntax error") || strings.Contains(message, "malformed match expression") || strings.Contains(message, "unterminated string") || strings.HasPrefix(message, "no such column:") {
			return ErrInvalidSearchExpression
		}
	}
	return err
}

func parseArticleCursor(cursor string) (string, int64, error) {
	date, raw, hasSeparator := strings.Cut(cursor, "|")
	if !hasSeparator {
		return "", 0, ErrInvalidCursor
	}
	id, err := strconv.ParseInt(raw, 10, 64)
	if err != nil {
		return "", 0, ErrInvalidCursor
	}
	return date, id, nil
}

func (filter *filter) articleCursor(cursor *string, direction string) error {
	if cursor == nil {
		return nil
	}
	date, id, err := parseArticleCursor(*cursor)
	if err != nil {
		return err
	}
	if date == "" {
		if direction == "ASC" {
			filter.add("((l.date IS NULL AND l.article_id>?) OR l.date IS NOT NULL)", id)
		} else {
			filter.add("(l.date IS NULL AND l.article_id<?)", id)
		}
	} else if direction == "ASC" {
		filter.add("(l.date>? OR (l.date=? AND l.article_id>?))", date, date, id)
	} else {
		filter.add("(l.date IS NULL OR l.date<? OR (l.date=? AND l.article_id<?))", date, date, id)
	}
	return nil
}

// ListArticles searches the dedicated projections and enriches only the selected bounded identifiers.
func ListArticles(ctx context.Context, configuration config.Config, name *string, params ArticleListParams) (domain.Page[domain.Article], error) {
	empty := domain.Page[domain.Article]{}
	if err := validatePagination(params.Limit, params.Offset); err != nil {
		return empty, err
	}
	if err := validateArticleInput(name, params); err != nil {
		return empty, err
	}
	database, err := open(configuration, name)
	if err != nil {
		return empty, err
	}
	defer database.Close()
	params, err = normalizeArticleParams(ctx, database, params)
	if err != nil {
		return empty, err
	}
	base, ratings, match, err := articleFilters(params)
	if err != nil {
		return empty, err
	}
	direction, err := articleDirection(params)
	if err != nil {
		return empty, err
	}
	return selectArticlePage(ctx, database, base, ratings, match, direction, params)
}

type articlePosition struct {
	id   int64
	date *string
}

func articlePage(ctx context.Context, database *sql.DB, positions []articlePosition, total *int64, params ArticleListParams) (domain.Page[domain.Article], error) {
	hasMore := int64(len(positions)) > params.Limit
	var cursor *string
	if hasMore {
		positions = positions[:params.Limit]
		last := positions[len(positions)-1]
		date := ""
		if last.date != nil {
			date = *last.date
		}
		value := date + "|" + strconv.FormatInt(last.id, 10)
		cursor = &value
	}
	ids := make([]int64, len(positions))
	for index, position := range positions {
		ids[index] = position.id
	}
	items, err := fetchArticles(ctx, database, ids)
	if err != nil {
		return domain.Page[domain.Article]{}, err
	}
	return domain.Page[domain.Article]{Items: items, Page: domain.PageMeta{Total: total, Limit: params.Limit, Offset: params.Offset, NextCursor: cursor, HasMore: &hasMore}}, nil
}

// GetArticle returns canonical data and sorted retraction identifiers, independent of search projections.
func GetArticle(ctx context.Context, configuration config.Config, name *string, id int64) (domain.Article, error) {
	database, err := open(configuration, name)
	if err != nil {
		return domain.Article{}, err
	}
	defer database.Close()
	items, err := fetchArticles(ctx, database, []int64{id})
	if err != nil {
		return domain.Article{}, err
	}
	if len(items) == 0 {
		return domain.Article{}, NotFound{"Article not found"}
	}
	return items[0], nil
}

func optionalBoolean(value *int64) *bool {
	if value == nil {
		return nil
	}
	result := *value != 0
	return &result
}

func articleFromRow(row scanner) (domain.Article, error) {
	var id, journal storage.Integer
	var issue, year, inPress, openAccess storage.OptionalInteger
	var title, authors, journalTitle storage.Text
	var date, start, end, abstract, doi, pmid, volume, number storage.OptionalText
	if err := row.Scan(&id, &journal, &issue, &title, &year, &date, &authors, &start, &end, &abstract, &doi, &pmid, &inPress, &openAccess, &journalTitle, &volume, &number); err != nil {
		return domain.Article{}, err
	}
	names, err := search.DecodeAuthorNames(string(authors))
	if err != nil {
		return domain.Article{}, err
	}
	result := domain.Article{ArticleId: identity.Id(id), JournalId: identity.Id(journal), IssueId: issue.Value, Title: string(title), PublicationYear: year.Value, Date: date.Value, Authors: names, StartPage: start.Value, EndPage: end.Value, Abstract: abstract.Value, Doi: doi.Value, Pmid: pmid.Value, InPress: optionalBoolean(inPress.Value), OpenAccess: optionalBoolean(openAccess.Value), RetractionDois: []string{}, JournalTitle: string(journalTitle), Volume: volume.Value, Number: number.Value}
	if date.Value != nil {
		result.DatePrecision = domain.DatePrecision(*date.Value)
	}
	return result, nil
}

// fetchArticles decodes canonical rows and retractions before retaining requested identifier order.
func fetchArticles(ctx context.Context, database rowQuerier, ids []int64) ([]domain.Article, error) {
	if len(ids) == 0 {
		return []domain.Article{}, nil
	}
	values := arguments(ids)
	items, err := collect(ctx, database, `SELECT a.article_id,a.journal_id,a.issue_id,a.title,a.publication_year,a.date,a.authors_json,a.start_page,a.end_page,a.abstract_text,a.doi,a.pmid,a.in_press,a.open_access,j.title,i.volume,i.number FROM articles a LEFT JOIN issues i ON i.issue_id=a.issue_id JOIN journals j ON j.journal_id=a.journal_id WHERE a.article_id IN (`+placeholders(len(ids))+")", values, articleFromRow)
	if err != nil {
		return nil, err
	}
	byId := make(map[int64]*domain.Article, len(items))
	for index := range items {
		byId[int64(items[index].ArticleId)] = &items[index]
	}
	rows, err := database.QueryContext(ctx, "SELECT article_id,retraction_doi FROM article_retraction_dois WHERE article_id IN ("+placeholders(len(ids))+") ORDER BY article_id,retraction_doi", values...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	if err := appendArticleRetractions(rows, byId); err != nil {
		return nil, err
	}
	return orderRequestedArticles(byId, ids), nil
}

// normalizeArticleParams closes the schema-inspection owner before rebinding only the local query.
func normalizeArticleParams(ctx context.Context, database *sql.DB, params ArticleListParams) (ArticleListParams, error) {
	connection, err := database.Conn(ctx)
	if err != nil {
		return params, err
	}
	usesSimple, err := search.UsesSimple(ctx, connection)
	connection.Close()
	if err != nil {
		return params, err
	}
	if params.Query != nil {
		normalized := search.PrepareQuery(*params.Query, usesSimple, params.SearchMode)
		params.Query = &normalized
	}
	return params, nil
}

// articleFilters constructs rating and listing clauses in their original argument order.
func articleFilters(params ArticleListParams) (filter, filter, string, error) {
	base, ratings := filter{}, filter{}
	if err := ratings.ratings(params.Ratings); err != nil {
		return filter{}, filter{}, "", err
	}
	base.list("l.journal_id", arguments(params.JournalId))
	base.optionalInteger("l.issue_id=?", params.IssueId)
	if len(ratings.clauses) == 0 {
		base.list("l.area", arguments(params.Area))
	} else {
		ratings.list("j.area", arguments(params.Area))
		ratings.list("j.journal_id", arguments(params.JournalId))
	}
	base.optionalBoolean("l.in_press=?", params.InPress)
	base.optionalBoolean("l.open_access=?", params.OpenAccess)
	base.optionalText("l.date>=?", params.DateFrom)
	base.optionalText("l.date<=?", params.DateTo)
	base.optionalText("l.doi=?", params.Doi)
	base.optionalText("l.pmid=?", params.Pmid)
	base.optionalInteger("l.publication_year=?", params.Year)
	match := preparedMatch(params)
	if match != "" {
		base.add("l.article_id IN (SELECT rowid FROM article_search WHERE article_search MATCH ?)", match)
	}
	return base, ratings, match, nil
}

// articleDirection keeps the single date-sort admission before rating eligibility.
func articleDirection(params ArticleListParams) (string, error) {
	order, err := orderBy(params.Sort, "date:desc", "l", "date")
	if err != nil {
		return "", err
	}
	if order == "" || strings.Contains(order, ",") {
		return "", ErrUnsupportedArticleSort
	}
	direction := "ASC"
	if strings.HasSuffix(order, " DESC") {
		direction = "DESC"
	}
	return direction, nil
}

// shouldCountArticles preserves the cursor-derived default and explicit override.
func shouldCountArticles(params ArticleListParams) bool {
	shouldCount := params.Cursor == nil
	if params.IncludeTotal != nil {
		shouldCount = *params.IncludeTotal
	}
	return shouldCount
}

// applyArticleRatings checks listing existence independently of the article-specific filters.
func applyArticleRatings(ctx context.Context, database *sql.DB, base *filter, ratings filter) (bool, error) {
	if len(ratings.clauses) == 0 {
		return true, nil
	}
	var hasEligible bool
	if err := database.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM journals j "+ratings.where()+" AND EXISTS(SELECT 1 FROM article_listing eligible WHERE eligible.journal_id=j.journal_id))", ratings.values...).Scan(&hasEligible); err != nil {
		return false, err
	}
	if hasEligible {
		base.add("l.journal_id IN (SELECT j.journal_id FROM journals j "+ratings.where()+")", ratings.values...)
	}
	return hasEligible, nil
}

// emptyRatedArticlePage validates cursor before MATCH and constructs a successful empty result.
func emptyRatedArticlePage(ctx context.Context, database *sql.DB, match string, shouldCount bool, params ArticleListParams) (domain.Page[domain.Article], error) {
	empty := domain.Page[domain.Article]{}
	var total *int64
	if params.Cursor != nil {
		if _, _, err := parseArticleCursor(*params.Cursor); err != nil {
			return empty, err
		}
	}
	if match != "" {
		var ignored storage.Integer
		err := database.QueryRowContext(ctx, "SELECT rowid FROM article_search WHERE article_search MATCH ? LIMIT 1", match).Scan(&ignored)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return empty, classifySearchError(err, params)
		}
	}
	if shouldCount {
		zero := int64(0)
		total = &zero
	}
	return articlePage(ctx, database, nil, total, params)
}

// articleTotal counts before cursor restrictions and preserves advanced error classification.
func articleTotal(ctx context.Context, database *sql.DB, base filter, shouldCount bool, params ArticleListParams) (*int64, error) {
	var total *int64
	if shouldCount {
		var count int64
		if err := database.QueryRowContext(ctx, "SELECT COUNT(*) FROM article_listing l "+base.where(), base.values...).Scan(&count); err != nil {
			return nil, classifySearchError(err, params)
		}
		total = &count
	}
	return total, nil
}

// collectArticlePositions adds cursor and pagination arguments only after optional counting.
func collectArticlePositions(ctx context.Context, database *sql.DB, base filter, direction string, params ArticleListParams) ([]articlePosition, error) {
	pagination := "LIMIT ?"
	values := append(base.values, params.Limit+1)
	if params.Cursor == nil {
		pagination += " OFFSET ?"
		values = append(values, params.Offset)
	}
	return collect(ctx, database, "SELECT l.article_id,l.date FROM article_listing l "+base.where()+" ORDER BY l.date "+direction+",l.article_id "+direction+" "+pagination, values, func(row scanner) (articlePosition, error) {
		var id storage.Integer
		var date storage.OptionalText
		if err := row.Scan(&id, &date); err != nil {
			return articlePosition{}, err
		}
		return articlePosition{int64(id), date.Value}, nil
	})
}

// selectArticlePage retains distinct rated-empty, counted and cursor-selection paths.
func selectArticlePage(ctx context.Context, database *sql.DB, base, ratings filter, match, direction string, params ArticleListParams) (domain.Page[domain.Article], error) {
	empty := domain.Page[domain.Article]{}
	shouldCount := shouldCountArticles(params)
	hasEligible, err := applyArticleRatings(ctx, database, &base, ratings)
	if err != nil {
		return empty, err
	}
	if !hasEligible {
		return emptyRatedArticlePage(ctx, database, match, shouldCount, params)
	}
	total, err := articleTotal(ctx, database, base, shouldCount, params)
	if err != nil {
		return empty, err
	}
	if err := base.articleCursor(params.Cursor, direction); err != nil {
		return empty, err
	}
	ids, err := collectArticlePositions(ctx, database, base, direction, params)
	if err != nil {
		return empty, classifySearchError(err, params)
	}
	return articlePage(ctx, database, ids, total, params)
}

// appendArticleRetractions validates every native row while the caller retains row ownership.
func appendArticleRetractions(rows *sql.Rows, byId map[int64]*domain.Article) error {
	for rows.Next() {
		var id storage.Integer
		var doi storage.Text
		if err := rows.Scan(&id, &doi); err != nil {
			return err
		}
		if item := byId[int64(id)]; item != nil {
			item.RetractionDois = append(item.RetractionDois, string(doi))
		}
	}
	if err := rows.Err(); err != nil {
		return err
	}
	return nil
}

// orderRequestedArticles keeps first occurrence order while excluding absent canonical records.
func orderRequestedArticles(byId map[int64]*domain.Article, ids []int64) []domain.Article {
	ordered := make([]domain.Article, 0, len(ids))
	for _, id := range ids {
		if item := byId[id]; item != nil {
			ordered = append(ordered, *item)
			delete(byId, id)
		}
	}
	return ordered
}
