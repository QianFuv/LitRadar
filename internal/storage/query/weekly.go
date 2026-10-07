package query

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

// ErrLegacyWeeklyLimit directs oversized legacy responses to the paginated endpoints.
var ErrLegacyWeeklyLimit = errors.New("Weekly updates exceed 2000 articles; use /api/weekly-updates/summary and /api/weekly-updates/articles")

type weeklyBucket struct {
	generated weekly.Timestamp
	runId     *string
	ids       []int64
	seen      map[int64]bool
}

func loadWeeklyBuckets(configuration config.Config, end weekly.Timestamp, cache *weekly.Cache) (map[string]*weeklyBucket, error) {
	manifests, err := weekly.LoadManifests(configuration, end.WindowStart(), end, cache)
	if err != nil {
		return nil, err
	}
	result := map[string]*weeklyBucket{}
	for _, manifest := range manifests {
		bucket := result[manifest.DbName]
		if bucket == nil {
			bucket = &weeklyBucket{manifest.GeneratedAt, manifest.RunId, []int64{}, map[int64]bool{}}
			result[manifest.DbName] = bucket
		}
		for _, id := range manifest.ArticleIds {
			if !bucket.seen[id] {
				bucket.seen[id] = true
				bucket.ids = append(bucket.ids, id)
			}
		}
	}
	return result, nil
}

func installWeeklyMembership(ctx context.Context, connection *sql.Conn, ids []int64) error {
	if _, err := connection.ExecContext(ctx, "CREATE TEMP TABLE weekly_membership(article_id INTEGER PRIMARY KEY) WITHOUT ROWID; BEGIN;"); err != nil {
		return err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	statement, err := connection.PrepareContext(ctx, "INSERT OR IGNORE INTO temp.weekly_membership(article_id) VALUES(?)")
	if err != nil {
		return err
	}
	defer statement.Close()
	for _, id := range ids {
		if _, err := statement.ExecContext(ctx, id); err != nil {
			return err
		}
	}
	_, err = connection.ExecContext(ctx, "COMMIT")
	return err
}

func sortWeeklyJournals(first, second domain.WeeklyJournalSummary) int {
	if first.NewArticleCount != second.NewArticleCount {
		if first.NewArticleCount > second.NewArticleCount {
			return -1
		}
		return 1
	}
	firstTitle, secondTitle := "", ""
	if first.JournalTitle != nil {
		firstTitle = asciiLower(*first.JournalTitle)
	}
	if second.JournalTitle != nil {
		secondTitle = asciiLower(*second.JournalTitle)
	}
	if order := strings.Compare(firstTitle, secondTitle); order != 0 {
		return order
	}
	if first.JournalId < second.JournalId {
		return -1
	}
	if first.JournalId > second.JournalId {
		return 1
	}
	return 0
}

func sortWeeklyDatabases[Journal any](databases []domain.WeeklyDatabase[Journal]) {
	slices.SortFunc(databases, func(first, second domain.WeeklyDatabase[Journal]) int {
		if order := strings.Compare(second.GeneratedAt, first.GeneratedAt); order != 0 {
			return order
		}
		return strings.Compare(second.DbName, first.DbName)
	})
}

// WeeklySummary returns counts from immutable manifest membership without loading article bodies.
func WeeklySummary(ctx context.Context, configuration config.Config, end weekly.Timestamp, cache *weekly.Cache) (domain.WeeklyResponse[domain.WeeklyJournalSummary], error) {
	result := domain.WeeklyResponse[domain.WeeklyJournalSummary]{GeneratedAt: end.Format(true), WindowEnd: end.Format(true), WindowStart: end.WindowStart().Format(true), Databases: []domain.WeeklyDatabase[domain.WeeklyJournalSummary]{}}
	buckets, err := loadWeeklyBuckets(configuration, end, cache)
	if err != nil {
		return result, err
	}
	for name, bucket := range buckets {
		filename := filepath.Join(configuration.IndexDir, name)
		if _, err := os.Stat(filename); err != nil || len(bucket.ids) == 0 {
			continue
		}
		journals, err := weeklyJournalCounts(ctx, filename, bucket.ids)
		if err != nil {
			return domain.WeeklyResponse[domain.WeeklyJournalSummary]{}, err
		}
		if len(journals) == 0 {
			continue
		}
		count := 0
		for _, journal := range journals {
			count += journal.NewArticleCount
		}
		result.Databases = append(result.Databases, domain.WeeklyDatabase[domain.WeeklyJournalSummary]{DbName: name, RunId: bucket.runId, GeneratedAt: bucket.generated.Format(false), NewArticleCount: count, Journals: journals})
	}
	sortWeeklyDatabases(result.Databases)
	return result, nil
}

func weeklyJournalCounts(ctx context.Context, filename string, ids []int64) ([]domain.WeeklyJournalSummary, error) {
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer connection.Close()
	if err := installWeeklyMembership(ctx, connection, ids); err != nil {
		return nil, err
	}
	items, err := collect(ctx, connection, `SELECT l.journal_id,j.title,COUNT(*) FROM temp.weekly_membership membership JOIN article_listing l ON l.article_id=membership.article_id JOIN journals j ON j.journal_id=l.journal_id GROUP BY l.journal_id,j.title`, nil, func(row scanner) (domain.WeeklyJournalSummary, error) {
		var journal, count storage.Integer
		var title storage.Text
		if err := row.Scan(&journal, &title, &count); err != nil {
			return domain.WeeklyJournalSummary{}, err
		}
		name := string(title)
		return domain.WeeklyJournalSummary{JournalId: identity.Id(journal), JournalTitle: &name, NewArticleCount: int(count)}, nil
	})
	if err != nil {
		return nil, err
	}
	slices.SortFunc(items, sortWeeklyJournals)
	return items, nil
}

// WeeklyArticlePageParams fixes membership at one RFC3339 window end across cursor requests.
type WeeklyArticlePageParams struct {
	DbName    string
	JournalId int64
	WindowEnd string
	Query     *string
	Limit     int64
	Cursor    *string
}

// WeeklyArticles validates the selected journal before loading every publication in the fixed window.
func WeeklyArticles(ctx context.Context, configuration config.Config, params WeeklyArticlePageParams, cache *weekly.Cache) (domain.Page[domain.WeeklyArticle], error) {
	empty := domain.Page[domain.WeeklyArticle]{}
	end, name, err := validateWeeklyArticleParams(params)
	if err != nil {
		return empty, err
	}
	database, err := open(configuration, &name)
	if err != nil {
		return empty, err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return empty, err
	}
	defer connection.Close()
	filters, err := weeklyArticleFilters(ctx, configuration, params, cache, connection, end, name)
	if err != nil {
		return empty, err
	}
	positions, err := collect(ctx, connection, "SELECT l.article_id,l.date FROM temp.weekly_membership membership JOIN article_listing l ON l.article_id=membership.article_id "+filters.where()+" ORDER BY l.date DESC,l.article_id DESC LIMIT ?", append(filters.values, params.Limit+1), func(row scanner) (articlePosition, error) {
		var id storage.Integer
		var date storage.OptionalText
		if err := row.Scan(&id, &date); err != nil {
			return articlePosition{}, err
		}
		return articlePosition{int64(id), date.Value}, nil
	})
	if err != nil {
		return empty, err
	}
	return weeklyArticlePage(ctx, connection, positions, params)
}

func weeklyArticle(article domain.Article) domain.WeeklyArticle {
	return domain.WeeklyArticle{ArticleId: article.ArticleId, JournalId: article.JournalId, IssueId: article.IssueId, Title: article.Title, PublicationYear: article.PublicationYear, Date: article.Date, DatePrecision: article.DatePrecision, Authors: article.Authors, Abstract: article.Abstract, Doi: article.Doi, JournalTitle: article.JournalTitle, OpenAccess: article.OpenAccess, InPress: article.InPress, Volume: article.Volume, Number: article.Number}
}

// WeeklyUpdates retains the legacy 2,000-reference ceiling before pruning unavailable databases or articles.
func WeeklyUpdates(ctx context.Context, configuration config.Config, end weekly.Timestamp) (domain.WeeklyResponse[domain.WeeklyJournalUpdate], error) {
	result := domain.WeeklyResponse[domain.WeeklyJournalUpdate]{GeneratedAt: end.Format(false), WindowEnd: end.Format(false), WindowStart: end.WindowStart().Format(false), Databases: []domain.WeeklyDatabase[domain.WeeklyJournalUpdate]{}}
	buckets, err := loadWeeklyBuckets(configuration, end, nil)
	if err != nil {
		return result, err
	}
	if err := validateLegacyWeeklyCount(buckets); err != nil {
		return domain.WeeklyResponse[domain.WeeklyJournalUpdate]{}, err
	}
	for name, bucket := range buckets {
		filename := filepath.Join(configuration.IndexDir, name)
		if _, err := os.Stat(filename); err != nil || len(bucket.ids) == 0 {
			continue
		}
		articles, err := fetchLegacyWeeklyArticles(ctx, filename, bucket.ids)
		if err != nil {
			return domain.WeeklyResponse[domain.WeeklyJournalUpdate]{}, err
		}
		if len(articles) == 0 {
			continue
		}
		result.Databases = append(result.Databases, legacyWeeklyDatabase(name, bucket, articles))
	}
	sortWeeklyDatabases(result.Databases)
	return result, nil
}

func fetchLegacyWeeklyArticles(ctx context.Context, filename string, ids []int64) ([]domain.WeeklyArticle, error) {
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	byId := map[int64]domain.WeeklyArticle{}
	for start := 0; start < len(ids); start += 500 {
		chunk := ids[start:min(start+500, len(ids))]
		items, err := collect(ctx, database, `SELECT a.article_id,a.journal_id,a.issue_id,a.title,a.publication_year,a.date,a.authors_json,a.abstract_text,a.doi,a.open_access,a.in_press,j.title,i.volume,i.number FROM articles a LEFT JOIN issues i ON i.issue_id=a.issue_id JOIN journals j ON j.journal_id=a.journal_id WHERE a.article_id IN (`+placeholders(len(chunk))+")", arguments(chunk), func(row scanner) (domain.WeeklyArticle, error) {
			var id, journal storage.Integer
			var issue, year, openAccess, inPress storage.OptionalInteger
			var title, authors, journalTitle storage.Text
			var date, abstract, doi, volume, number storage.OptionalText
			if err := row.Scan(&id, &journal, &issue, &title, &year, &date, &authors, &abstract, &doi, &openAccess, &inPress, &journalTitle, &volume, &number); err != nil {
				return domain.WeeklyArticle{}, err
			}
			names, err := search.DecodeAuthorNames(string(authors))
			if err != nil {
				return domain.WeeklyArticle{}, err
			}
			result := domain.WeeklyArticle{ArticleId: identity.Id(id), JournalId: identity.Id(journal), IssueId: issue.Value, Title: string(title), PublicationYear: year.Value, Date: date.Value, Authors: names, Abstract: abstract.Value, Doi: doi.Value, OpenAccess: optionalBoolean(openAccess.Value), InPress: optionalBoolean(inPress.Value), JournalTitle: string(journalTitle), Volume: volume.Value, Number: number.Value}
			if result.Date != nil {
				result.DatePrecision = domain.DatePrecision(*result.Date)
			}
			return result, nil
		})
		if err != nil {
			return nil, err
		}
		for _, item := range items {
			byId[int64(item.ArticleId)] = item
		}
	}
	result := make([]domain.WeeklyArticle, 0, len(ids))
	for _, id := range ids {
		if item, exists := byId[id]; exists {
			result = append(result, item)
			delete(byId, id)
		}
	}
	return result, nil
}

// validateWeeklyArticleLengths preserves db/window/query/cursor validation priority.
func validateWeeklyArticleLengths(params WeeklyArticlePageParams) error {
	for _, field := range []struct {
		name, value string
		limit       int
	}{{"db", params.DbName, 255}, {"window_end", params.WindowEnd, 2048}} {
		if err := validateCharacters(field.name, field.value, field.limit); err != nil {
			return err
		}
	}
	if params.Query != nil {
		if err := validateCharacters("q", *params.Query, 2048); err != nil {
			return err
		}
	}
	if params.Cursor != nil {
		if err := validateCharacters("cursor", *params.Cursor, 2048); err != nil {
			return err
		}
	}
	return nil
}

// validateWeeklyArticleParams admits journal, pagination, timestamp and database in the original order.
func validateWeeklyArticleParams(params WeeklyArticlePageParams) (weekly.Timestamp, string, error) {
	if err := validateWeeklyArticleLengths(params); err != nil {
		return weekly.Timestamp{}, "", err
	}
	if params.JournalId <= 0 {
		return weekly.Timestamp{}, "", InvalidInput{"journal_id must be greater than 0"}
	}
	if err := validatePagination(params.Limit, 0); err != nil {
		return weekly.Timestamp{}, "", err
	}
	end, ok := weekly.ParseTimestamp(params.WindowEnd)
	if !ok {
		return weekly.Timestamp{}, "", InvalidInput{"window_end must be a valid RFC3339 timestamp"}
	}
	name := config.NormalizeDatabaseName(params.DbName)
	if name == "" {
		return weekly.Timestamp{}, "", InvalidInput{"db must select a database"}
	}
	return end, name, nil
}

// weeklyArticleFilters keeps journal admission before manifests on the caller's pinned connection.
func weeklyArticleFilters(ctx context.Context, configuration config.Config, params WeeklyArticlePageParams, cache *weekly.Cache, connection *sql.Conn, end weekly.Timestamp, name string) (filter, error) {
	var exists bool
	if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM journals WHERE journal_id=?)", params.JournalId).Scan(&exists); err != nil {
		return filter{}, err
	}
	if !exists {
		return filter{}, NotFound{"Journal not found"}
	}
	buckets, err := loadWeeklyBuckets(configuration, end, cache)
	if err != nil {
		return filter{}, err
	}
	ids := []int64{}
	if bucket := buckets[name]; bucket != nil {
		ids = bucket.ids
	}
	if err := installWeeklyMembership(ctx, connection, ids); err != nil {
		return filter{}, err
	}
	usesSimple, err := search.UsesSimple(ctx, connection)
	if err != nil {
		return filter{}, err
	}
	query := params.Query
	if query != nil {
		value := search.PrepareQuery(*query, usesSimple, domain.SearchSimple)
		query = &value
	}
	filters := filter{}
	filters.add("l.journal_id=?", params.JournalId)
	if match := preparedMatch(ArticleListParams{Query: query}); match != "" {
		filters.add("l.article_id IN (SELECT rowid FROM article_search WHERE article_search MATCH ?)", match)
	}
	if err := filters.articleCursor(params.Cursor, "DESC"); err != nil {
		return filter{}, err
	}
	return filters, nil
}

// weeklyArticlePage creates cursor metadata from positions before canonical enrichment.
func weeklyArticlePage(ctx context.Context, connection *sql.Conn, positions []articlePosition, params WeeklyArticlePageParams) (domain.Page[domain.WeeklyArticle], error) {
	empty := domain.Page[domain.WeeklyArticle]{}
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
	selected := make([]int64, len(positions))
	for index, position := range positions {
		selected[index] = position.id
	}
	articles, err := fetchArticles(ctx, connection, selected)
	if err != nil {
		return empty, err
	}
	items := make([]domain.WeeklyArticle, 0, len(articles))
	for _, article := range articles {
		items = append(items, weeklyArticle(article))
	}
	return domain.Page[domain.WeeklyArticle]{Items: items, Page: domain.PageMeta{Limit: params.Limit, NextCursor: cursor, HasMore: &hasMore}}, nil
}

// validateLegacyWeeklyCount retains the reference ceiling before any availability pruning.
func validateLegacyWeeklyCount(buckets map[string]*weeklyBucket) error {
	count := 0
	for _, bucket := range buckets {
		count += len(bucket.ids)
		if count > 2000 {
			return ErrLegacyWeeklyLimit
		}
	}
	return nil
}

// legacyWeeklyDatabase preserves per-journal item order and summary sorting for one publication.
func legacyWeeklyDatabase(name string, bucket *weeklyBucket, articles []domain.WeeklyArticle) domain.WeeklyDatabase[domain.WeeklyJournalUpdate] {
	groups := map[identity.Id][]domain.WeeklyArticle{}
	for _, article := range articles {
		groups[article.JournalId] = append(groups[article.JournalId], article)
	}
	journals := make([]domain.WeeklyJournalUpdate, 0, len(groups))
	for id, items := range groups {
		title := items[0].JournalTitle
		journals = append(journals, domain.WeeklyJournalUpdate{WeeklyJournalSummary: domain.WeeklyJournalSummary{JournalId: id, JournalTitle: &title, NewArticleCount: len(items)}, Articles: items})
	}
	slices.SortFunc(journals, func(first, second domain.WeeklyJournalUpdate) int {
		return sortWeeklyJournals(first.WeeklyJournalSummary, second.WeeklyJournalSummary)
	})
	return domain.WeeklyDatabase[domain.WeeklyJournalUpdate]{DbName: name, RunId: bucket.runId, GeneratedAt: bucket.generated.Format(false), NewArticleCount: len(articles), Journals: journals}
}
