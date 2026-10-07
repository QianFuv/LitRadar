package query

import (
	"context"
	"database/sql"
	"errors"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// JournalListParams keeps repository zero defaults separate from transport defaults.
type JournalListParams struct {
	Area          *string
	Ratings       domain.JournalRatings
	HasArticles   *bool
	Year          *int64
	Sort          *string
	Limit, Offset int64
}

// IssueListParams selects canonical issues independently of article availability.
type IssueListParams struct {
	JournalId, Year *int64
	Sort            *string
	Limit, Offset   int64
}

const journalColumns = `j.journal_id,j.catalog_id,j.title,j.title_aliases_json,j.issns_json,j.issn,j.eissn,j.area,j.utd_rank,j.utd_rating,j.abs_rank,j.abs_rating,j.fms_rank,j.fms_rating,j.fmscn_rank,j.fmscn_rating,EXISTS(SELECT 1 FROM articles a WHERE a.journal_id=j.journal_id)`
const issueColumns = `i.issue_id,i.journal_id,i.publication_year,i.title,i.volume,i.number,i.date`

func journalFromRow(row scanner) (domain.Journal, error) {
	var id, hasArticles storage.Integer
	var catalog, title, aliases, issns storage.Text
	var optional [11]storage.OptionalText
	arguments := []any{&id, &catalog, &title, &aliases, &issns}
	for index := range optional {
		arguments = append(arguments, &optional[index])
	}
	arguments = append(arguments, &hasArticles)
	if err := row.Scan(arguments...); err != nil {
		return domain.Journal{}, err
	}
	names, err := search.DecodeAuthorNames(string(aliases))
	if err != nil {
		return domain.Journal{}, err
	}
	identifiers, err := search.DecodeAuthorNames(string(issns))
	if err != nil {
		return domain.Journal{}, err
	}
	return domain.Journal{JournalId: identity.Id(id), CatalogId: string(catalog), Title: string(title), TitleAliases: names, Issns: identifiers, Issn: optional[0].Value, Eissn: optional[1].Value, Area: optional[2].Value, UtdRank: optional[3].Value, UtdRating: optional[4].Value, AbsRank: optional[5].Value, AbsRating: optional[6].Value, FmsRank: optional[7].Value, FmsRating: optional[8].Value, FmscnRank: optional[9].Value, FmscnRating: optional[10].Value, HasArticles: hasArticles != 0}, nil
}

func issueFromRow(row scanner) (domain.Issue, error) {
	var id, journal storage.Integer
	var year storage.OptionalInteger
	var title, volume, number, date storage.OptionalText
	if err := row.Scan(&id, &journal, &year, &title, &volume, &number, &date); err != nil {
		return domain.Issue{}, err
	}
	result := domain.Issue{IssueId: int64(id), JournalId: identity.Id(journal), PublicationYear: year.Value, Title: title.Value, Volume: volume.Value, Number: number.Value, Date: date.Value}
	if date.Value != nil {
		result.DatePrecision = domain.DatePrecision(*date.Value)
	}
	return result, nil
}

// ListJournals validates public filters in their original order and counts all matching journals.
func ListJournals(ctx context.Context, configuration config.Config, name *string, params JournalListParams) (domain.Page[domain.Journal], error) {
	empty := domain.Page[domain.Journal]{}
	if err := validatePagination(params.Limit, params.Offset); err != nil {
		return empty, err
	}
	if err := validateJournalFilterCount(params); err != nil {
		return empty, err
	}
	database, err := open(configuration, name)
	if err != nil {
		return empty, err
	}
	defer database.Close()
	if err := database.PingContext(ctx); err != nil {
		return empty, err
	}
	filters, err := journalFilters(params)
	if err != nil {
		return empty, err
	}
	order, err := orderBy(params.Sort, "title:asc", "j", "journal_id", "title", "issn", "eissn", "area")
	if err != nil {
		return empty, err
	}
	var total int64
	if err := database.QueryRowContext(ctx, "SELECT COUNT(*) FROM journals j "+filters.where(), filters.values...).Scan(&total); err != nil {
		return empty, err
	}
	items, err := collect(ctx, database, "SELECT "+journalColumns+" FROM journals j "+filters.where()+" "+order+" LIMIT ? OFFSET ?", append(filters.values, params.Limit, params.Offset), journalFromRow)
	if err != nil {
		return empty, err
	}
	return domain.Page[domain.Journal]{Items: items, Page: domain.PageMeta{Total: &total, Limit: params.Limit, Offset: params.Offset}}, nil
}

// GetJournal returns one canonical row without imposing a new positive-ID restriction.
func GetJournal(ctx context.Context, configuration config.Config, name *string, id int64) (domain.Journal, error) {
	database, err := open(configuration, name)
	if err != nil {
		return domain.Journal{}, err
	}
	defer database.Close()
	record, err := journalFromRow(database.QueryRowContext(ctx, "SELECT "+journalColumns+" FROM journals j WHERE j.journal_id=?", id))
	if errors.Is(err, sql.ErrNoRows) {
		return record, NotFound{"Journal not found"}
	}
	return record, err
}

// ListIssues counts and pages the issue inventory rather than deriving it from articles.
func ListIssues(ctx context.Context, configuration config.Config, name *string, params IssueListParams) (domain.Page[domain.Issue], error) {
	empty := domain.Page[domain.Issue]{}
	if err := validatePagination(params.Limit, params.Offset); err != nil {
		return empty, err
	}
	database, err := open(configuration, name)
	if err != nil {
		return empty, err
	}
	defer database.Close()
	if err := database.PingContext(ctx); err != nil {
		return empty, err
	}
	filters := filter{}
	if params.JournalId != nil {
		filters.add("i.journal_id=?", *params.JournalId)
	}
	if params.Year != nil {
		filters.add("i.publication_year=?", *params.Year)
	}
	order, err := orderBy(params.Sort, "publication_year:desc", "i", "issue_id", "publication_year", "title", "date", "volume", "number")
	if err != nil {
		return empty, err
	}
	var total int64
	if err := database.QueryRowContext(ctx, "SELECT COUNT(*) FROM issues i "+filters.where(), filters.values...).Scan(&total); err != nil {
		return empty, err
	}
	items, err := collect(ctx, database, "SELECT "+issueColumns+" FROM issues i "+filters.where()+" "+order+" LIMIT ? OFFSET ?", append(filters.values, params.Limit, params.Offset), issueFromRow)
	if err != nil {
		return empty, err
	}
	return domain.Page[domain.Issue]{Items: items, Page: domain.PageMeta{Total: &total, Limit: params.Limit, Offset: params.Offset}}, nil
}

// GetIssue preserves stored date text while deriving its validated precision.
func GetIssue(ctx context.Context, configuration config.Config, name *string, id int64) (domain.Issue, error) {
	database, err := open(configuration, name)
	if err != nil {
		return domain.Issue{}, err
	}
	defer database.Close()
	record, err := issueFromRow(database.QueryRowContext(ctx, "SELECT "+issueColumns+" FROM issues i WHERE i.issue_id=?", id))
	if errors.Is(err, sql.ErrNoRows) {
		return record, NotFound{"Issue not found"}
	}
	return record, err
}

func valueCountFromRow(row scanner) (domain.ValueCount, error) {
	var value storage.Text
	var count storage.Integer
	if err := row.Scan(&value, &count); err != nil {
		return domain.ValueCount{}, err
	}
	return domain.ValueCount{Value: string(value), Count: int64(count)}, nil
}

// ListAreas counts nonempty stored labels, including labels containing only whitespace.
func ListAreas(ctx context.Context, configuration config.Config, name *string) ([]domain.ValueCount, error) {
	database, err := open(configuration, name)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	return collect(ctx, database, "SELECT area,COUNT(*) FROM journals WHERE area IS NOT NULL AND area!='' GROUP BY area ORDER BY area ASC", nil, valueCountFromRow)
}

// ListJournalRatings includes four arrays and counts journals without articles.
func ListJournalRatings(ctx context.Context, configuration config.Config, name *string) (domain.RatingOptions, error) {
	database, err := open(configuration, name)
	if err != nil {
		return domain.RatingOptions{}, err
	}
	defer database.Close()
	result := domain.RatingOptions{}
	for _, group := range []struct {
		field  string
		target *[]domain.ValueCount
	}{{"utd_rating", &result.UtdRating}, {"abs_rating", &result.AbsRating}, {"fms_rating", &result.FmsRating}, {"fmscn_rating", &result.FmscnRating}} {
		*group.target, err = collect(ctx, database, "SELECT "+group.field+",COUNT(*) FROM journals WHERE "+group.field+" IS NOT NULL AND "+group.field+"!='' GROUP BY "+group.field+" ORDER BY "+group.field, nil, valueCountFromRow)
		if err != nil {
			return domain.RatingOptions{}, err
		}
	}
	return result, nil
}

// ListJournalOptions returns the complete title-ordered journal selector inventory.
func ListJournalOptions(ctx context.Context, configuration config.Config, name *string) ([]domain.JournalOption, error) {
	database, err := open(configuration, name)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	return collect(ctx, database, "SELECT journal_id,title FROM journals ORDER BY title ASC", nil, func(row scanner) (domain.JournalOption, error) {
		var id storage.Integer
		var title storage.Text
		if err := row.Scan(&id, &title); err != nil {
			return domain.JournalOption{}, err
		}
		return domain.JournalOption{JournalId: identity.Id(id), Title: string(title)}, nil
	})
}

// ListYears counts distinct issues and journals grouped by stored publication year.
func ListYears(ctx context.Context, configuration config.Config, name *string) ([]domain.YearSummary, error) {
	database, err := open(configuration, name)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	return collect(ctx, database, "SELECT publication_year,COUNT(DISTINCT issue_id),COUNT(DISTINCT journal_id) FROM issues WHERE publication_year IS NOT NULL GROUP BY publication_year ORDER BY publication_year DESC", nil, func(row scanner) (domain.YearSummary, error) {
		var year, issues, journals storage.Integer
		if err := row.Scan(&year, &issues, &journals); err != nil {
			return domain.YearSummary{}, err
		}
		return domain.YearSummary{Year: int64(year), IssueCount: int64(issues), JournalCount: int64(journals)}, nil
	})
}

// validateJournalFilterCount counts a supplied area even when its value is blank.
func validateJournalFilterCount(params JournalListParams) error {
	count := 0
	if params.Area != nil {
		count++
	}
	for _, group := range ratingGroups(params.Ratings) {
		count += len(group.values)
	}
	if count > 500 {
		return InvalidInput{"search filters must contain at most 500 items"}
	}
	return nil
}

// journalFilters preserves area, ratings, canonical article and issue-year clause order.
func journalFilters(params JournalListParams) (filter, error) {
	filters := filter{}
	if params.Area != nil {
		if value := strings.TrimSpace(*params.Area); value != "" {
			filters.add("j.area = ?", value)
		}
	}
	if err := filters.ratings(params.Ratings); err != nil {
		return filter{}, err
	}
	if params.HasArticles != nil {
		prefix := ""
		if !*params.HasArticles {
			prefix = "NOT "
		}
		filters.add(prefix + "EXISTS(SELECT 1 FROM articles a WHERE a.journal_id=j.journal_id)")
	}
	if params.Year != nil {
		filters.add("EXISTS(SELECT 1 FROM issues i WHERE i.journal_id=j.journal_id AND i.publication_year=?)", *params.Year)
	}
	return filters, nil
}
