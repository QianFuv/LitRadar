package query

import (
	"context"
	"database/sql"
	"errors"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/querydb"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

var queries querydb.Queries

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

// journalFromRow maps canonical journal metadata using one fixed-width scan buffer.
func journalFromRow(scanner scanner) (domain.Journal, error) {
	var row querydb.JournalByIDRow
	var hasArticles storage.Integer
	if err := scanner.Scan(&row.JournalID, &row.CatalogID, &row.Title, &row.TitleAliasesJSON, &row.IssnsJSON, &row.ISSN, &row.EISSN, &row.Area, &row.UTDRank, &row.UTDRating, &row.ABSRank, &row.ABSRating, &row.FMSRank, &row.FMSRating, &row.FMSCNRank, &row.FMSCNRating, &hasArticles); err != nil {
		return domain.Journal{}, err
	}
	row.Exists = hasArticles != 0
	return decodeJournal(row)
}

func decodeJournal(row querydb.JournalByIDRow) (domain.Journal, error) {
	names, err := search.DecodeAuthorNames(string(row.TitleAliasesJSON))
	if err != nil {
		return domain.Journal{}, err
	}
	identifiers, err := search.DecodeAuthorNames(string(row.IssnsJSON))
	if err != nil {
		return domain.Journal{}, err
	}
	return domain.Journal{JournalId: identity.Id(row.JournalID), CatalogId: string(row.CatalogID), Title: string(row.Title), TitleAliases: names, Issns: identifiers, Issn: (*string)(row.ISSN), Eissn: (*string)(row.EISSN), Area: (*string)(row.Area), UtdRank: (*string)(row.UTDRank), UtdRating: (*string)(row.UTDRating), AbsRank: (*string)(row.ABSRank), AbsRating: (*string)(row.ABSRating), FmsRank: (*string)(row.FMSRank), FmsRating: (*string)(row.FMSRating), FmscnRank: (*string)(row.FMSCNRank), FmscnRating: (*string)(row.FMSCNRating), HasArticles: row.Exists}, nil
}

func issueFromRow(scanner scanner) (domain.Issue, error) {
	var row querydb.Issue
	if err := scanner.Scan(&row.IssueID, &row.JournalID, &row.PublicationYear, &row.Title, &row.Volume, &row.Number, &row.Date); err != nil {
		return domain.Issue{}, err
	}
	return decodeIssue(row), nil
}

func decodeIssue(row querydb.Issue) domain.Issue {
	result := domain.Issue{IssueId: int64(row.IssueID), JournalId: identity.Id(row.JournalID), PublicationYear: (*int64)(row.PublicationYear), Title: (*string)(row.Title), Volume: (*string)(row.Volume), Number: (*string)(row.Number), Date: (*string)(row.Date)}
	if result.Date != nil {
		result.DatePrecision = domain.DatePrecision(*result.Date)
	}
	return result
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
	row, err := queries.JournalByID(ctx, database, querydb.JournalByIDParams{JournalID: storage.Integer(id)})
	if errors.Is(err, sql.ErrNoRows) {
		return domain.Journal{}, NotFound{"Journal not found"}
	}
	if err != nil {
		return domain.Journal{}, err
	}
	return decodeJournal(row)
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
	row, err := queries.IssueByID(ctx, database, querydb.IssueByIDParams{IssueID: storage.Integer(id)})
	if errors.Is(err, sql.ErrNoRows) {
		return domain.Issue{}, NotFound{"Issue not found"}
	}
	if err != nil {
		return domain.Issue{}, err
	}
	return decodeIssue(row), nil
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
	rows, err := queries.Areas(ctx, database)
	if err != nil {
		return nil, err
	}
	items := make([]domain.ValueCount, 0, len(rows))
	for _, row := range rows {
		if row.Area == nil {
			return nil, errors.New("invalid SQLite text column")
		}
		items = append(items, domain.ValueCount{Value: string(*row.Area), Count: row.Count})
	}
	return items, nil
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
	rows, err := queries.JournalOptions(ctx, database)
	if err != nil {
		return nil, err
	}
	items := make([]domain.JournalOption, 0, len(rows))
	for _, row := range rows {
		items = append(items, domain.JournalOption{JournalId: identity.Id(row.JournalID), Title: string(row.Title)})
	}
	return items, nil
}

// ListYears counts distinct issues and journals grouped by stored publication year.
func ListYears(ctx context.Context, configuration config.Config, name *string) ([]domain.YearSummary, error) {
	database, err := open(configuration, name)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	rows, err := queries.Years(ctx, database)
	if err != nil {
		return nil, err
	}
	items := make([]domain.YearSummary, 0, len(rows))
	for _, row := range rows {
		if row.PublicationYear == nil {
			return nil, errors.New("invalid SQLite integer column")
		}
		items = append(items, domain.YearSummary{Year: int64(*row.PublicationYear), IssueCount: row.Count, JournalCount: row.Count_2})
	}
	return items, nil
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
