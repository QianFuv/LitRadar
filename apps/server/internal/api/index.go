package api

import (
	"context"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/query"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/weekly"
	"github.com/danielgtaylor/huma/v2"
	"path/filepath"
	"time"
)

type route struct {
	operation openapi.Operation
	register  func(huma.API) error
}
type indexHandlers struct {
	storage       config.Config
	authenticator *Authenticator
	pool          *executor.Pool
	cache         *weekly.Cache
}

func (handlers *indexHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/meta/databases", ID: "list_databases"}, func(ctx context.Context, input *ListDatabasesInput) (*output[[]string], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() ([]string, error) {
				paths, err := handlers.storage.ListIndexDatabases()
				for index, path := range paths {
					paths[index] = filepath.Base(path)
				}
				return paths, err
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/meta/areas", ID: "list_areas"}, func(ctx context.Context, input *ListAreasInput) (*output[[]domain.ValueCount], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() ([]domain.ValueCount, error) {
				return query.ListAreas(ctx, handlers.storage, cleanText(input.DB.Pointer()))
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/meta/ratings", ID: "list_journal_ratings"}, func(ctx context.Context, input *ListJournalRatingsInput) (*output[domain.RatingOptions], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.RatingOptions, error) {
				return query.ListJournalRatings(ctx, handlers.storage, cleanText(input.DB.Pointer()))
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/meta/journals", ID: "list_journal_options"}, func(ctx context.Context, input *ListJournalOptionsInput) (*output[[]domain.JournalOption], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() ([]domain.JournalOption, error) {
				return query.ListJournalOptions(ctx, handlers.storage, cleanText(input.DB.Pointer()))
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/years", ID: "list_years"}, func(ctx context.Context, input *ListYearsInput) (*output[[]domain.YearSummary], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() ([]domain.YearSummary, error) {
				return query.ListYears(ctx, handlers.storage, cleanText(input.DB.Pointer()))
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/journals/{journal_id}", ID: "get_journal"}, func(ctx context.Context, input *GetJournalInput) (*output[domain.Journal], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.Journal, error) {
				return query.GetJournal(ctx, handlers.storage, cleanText(input.DB.Pointer()), input.JournalID)
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/issues/{issue_id}", ID: "get_issue"}, func(ctx context.Context, input *GetIssueInput) (*output[domain.Issue], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.Issue, error) {
				return query.GetIssue(ctx, handlers.storage, cleanText(input.DB.Pointer()), input.IssueID)
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/articles/{article_id}", ID: "get_article"}, func(ctx context.Context, input *GetArticleInput) (*output[domain.Article], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.Article, error) {
				return query.GetArticle(ctx, handlers.storage, cleanText(input.DB.Pointer()), input.ArticleID)
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/articles/{article_id}/related", ID: "list_related_articles"}, func(ctx context.Context, input *ListRelatedArticlesInput) (*output[[]domain.Article], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() ([]domain.Article, error) {
				return query.RelatedArticles(ctx, handlers.storage, cleanText(input.DB.Pointer()), input.ArticleID, numberOr(input.Limit.Pointer(), 10))
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/journals", ID: "list_journals"}, func(ctx context.Context, input *ListJournalsInput) (*output[domain.Page[domain.Journal]], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.Page[domain.Journal], error) {
				params := query.JournalListParams{Area: cleanText(input.Area.Pointer()), Ratings: domain.JournalRatings{UtdRating: input.UTDRating, AbsRating: input.ABSRating, FmsRating: input.FMSRating, FmscnRating: input.FMSCNRating}, HasArticles: input.HasArticles.Pointer(), Year: input.Year.Pointer(), Sort: cleanText(input.Sort.Pointer()), Limit: numberOr(input.Limit.Pointer(), 50), Offset: numberOr(input.Offset.Pointer(), 0)}
				return query.ListJournals(ctx, handlers.storage, cleanText(input.DB.Pointer()), params)
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/issues", ID: "list_issues"}, func(ctx context.Context, input *ListIssuesInput) (*output[domain.Page[domain.Issue]], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.Page[domain.Issue], error) {
				params := query.IssueListParams{JournalID: input.JournalID.Pointer(), Year: input.Year.Pointer(), Sort: cleanText(input.Sort.Pointer()), Limit: numberOr(input.Limit.Pointer(), 50), Offset: numberOr(input.Offset.Pointer(), 0)}
				return query.ListIssues(ctx, handlers.storage, cleanText(input.DB.Pointer()), params)
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/articles", ID: "list_articles"}, func(ctx context.Context, input *ListArticlesInput) (*output[domain.Page[domain.Article]], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.Page[domain.Article], error) {
				params := query.DefaultArticleListParams()
				params.JournalID = input.JournalID
				params.Area = input.Area
				params.Ratings = domain.JournalRatings{UtdRating: input.UTDRating, AbsRating: input.ABSRating, FmsRating: input.FMSRating, FmscnRating: input.FMSCNRating}
				params.IssueID = input.IssueID.Pointer()
				params.Year = input.Year.Pointer()
				params.InPress = input.InPress.Pointer()
				params.OpenAccess = input.OpenAccess.Pointer()
				params.DateFrom = cleanText(input.DateFrom.Pointer())
				params.DateTo = cleanText(input.DateTo.Pointer())
				params.Doi = cleanText(input.DOI.Pointer())
				params.Pmid = cleanText(input.PMID.Pointer())
				params.Query = cleanText(input.Q.Pointer())
				if input.SearchMode.Pointer() != nil {
					params.SearchMode = *input.SearchMode.Pointer()
				}
				if value := cleanText(input.Sort.Pointer()); value != nil {
					params.Sort = value
				}
				params.Limit = numberOr(input.Limit.Pointer(), 50)
				params.Offset = numberOr(input.Offset.Pointer(), 0)
				params.Cursor = cleanText(input.Cursor.Pointer())
				params.IncludeTotal = input.IncludeTotal.Pointer()
				return query.ListArticles(ctx, handlers.storage, cleanText(input.DB.Pointer()), params)
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/weekly-updates", ID: "get_weekly_updates"}, func(ctx context.Context, input *GetWeeklyUpdatesInput) (*output[domain.WeeklyResponse[domain.WeeklyJournalUpdate]], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.WeeklyResponse[domain.WeeklyJournalUpdate], error) {
				return query.WeeklyUpdates(ctx, handlers.storage, weekly.FromTime(time.Now()))
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/weekly-updates/summary", ID: "get_weekly_updates_summary"}, func(ctx context.Context, input *GetWeeklyUpdatesSummaryInput) (*output[domain.WeeklyResponse[domain.WeeklyJournalSummary]], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.WeeklyResponse[domain.WeeklyJournalSummary], error) {
				return query.WeeklySummary(ctx, handlers.storage, weekly.FromTime(time.Now()), handlers.cache)
			}, mapIndexError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/weekly-updates/articles", ID: "get_weekly_update_articles"}, func(ctx context.Context, input *GetWeeklyUpdateArticlesInput) (*output[domain.Page[domain.WeeklyArticle]], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			return admitted(ctx, handlers.pool, func() (domain.Page[domain.WeeklyArticle], error) {
				params := query.WeeklyArticlePageParams{DBName: input.DB, JournalID: input.JournalID, WindowEnd: input.WindowEnd, Query: cleanText(input.Q.Pointer()), Limit: numberOr(input.Limit.Pointer(), 50), Cursor: cleanText(input.Cursor.Pointer())}
				return query.WeeklyArticles(ctx, handlers.storage, params, handlers.cache)
			}, mapIndexError)
		}),
	}
}
