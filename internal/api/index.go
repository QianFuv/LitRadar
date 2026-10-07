package api

import (
	"context"
	"net/http"
	"path/filepath"
	"time"

	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/platform/httpwire"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/query"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

type route struct {
	operation openapi.Operation
	handler   http.HandlerFunc
}
type indexHandlers struct {
	storage       config.Config
	authenticator *Authenticator
	pool          *executor.Pool
	cache         *weekly.Cache
}

func (handlers *indexHandlers) routes() []route {
	result := []route{}
	for _, operation := range []struct{ path, id string }{
		{"/api/meta/databases", "list_databases"}, {"/api/meta/areas", "list_areas"}, {"/api/meta/ratings", "list_journal_ratings"}, {"/api/meta/journals", "list_journal_options"}, {"/api/years", "list_years"},
		{"/api/journals", "list_journals"}, {"/api/journals/{journal_id}", "get_journal"}, {"/api/issues", "list_issues"}, {"/api/issues/{issue_id}", "get_issue"},
		{"/api/articles", "list_articles"}, {"/api/articles/{article_id}", "get_article"}, {"/api/weekly-updates", "get_weekly_updates"}, {"/api/weekly-updates/summary", "get_weekly_updates_summary"}, {"/api/weekly-updates/articles", "get_weekly_update_articles"},
	} {
		result = append(result, route{openapi.Operation{Method: "GET", Path: operation.path, Id: operation.id}, func(writer http.ResponseWriter, request *http.Request) {
			handlers.handle(writer, request, operation.id)
		}})
	}
	return result
}

// handle extracts typed syntax before authentication and business queries afterward.
func (handlers *indexHandlers) handle(writer http.ResponseWriter, request *http.Request, name string) {
	recordId, failure := extractIndexRecord(request, name)
	if failure != nil {
		failure.write(writer)
		return
	}
	values, failure := extractIndexQuery(request, name)
	if failure != nil {
		failure.write(writer)
		return
	}
	if _, failure = handlers.authenticator.requireUser(request); failure != nil {
		failure.write(writer)
		return
	}
	work, failure := handlers.indexWork(request, name, recordId, values)
	if failure != nil {
		failure.write(writer)
		return
	}
	type outcome struct {
		value any
		err   error
	}
	result, err := executor.Run(request.Context(), handlers.pool, func() (outcome, error) { value, err := work(); return outcome{value, err}, nil })
	if err != nil {
		mapExecutorError(err).write(writer)
		return
	}
	if result.err != nil {
		mapIndexError(result.err).write(writer)
		return
	}
	if err := httpwire.JSON(writer, 200, result.value); err != nil {
		internalError().write(writer)
	}
}

// extractIndexRecord preserves path admission before query and authentication checks.
func extractIndexRecord(request *http.Request, name string) (int64, *apiError) {
	pathField := ""
	switch name {
	case "get_journal":
		pathField = "journal_id"
	case "get_issue":
		pathField = "issue_id"
	case "get_article":
		pathField = "article_id"
	}

	if pathField == "" {
		return 0, nil
	}
	return extractPathInteger(request, pathField)
}

// indexQueryFields distinguishes typed endpoints from post-authentication search queries.
func indexQueryFields(name string) (map[string]queryKind, bool) {
	fields := map[string]queryKind{"db": queryText}
	isTyped := true
	switch name {
	case "list_databases", "list_articles", "get_weekly_updates", "get_weekly_updates_summary", "get_weekly_update_articles":
		isTyped = false
	case "list_journals":
		fields = map[string]queryKind{"db": queryText, "area": queryText, "has_articles": queryBoolean, "year": queryInteger, "sort": queryText, "limit": queryInteger, "offset": queryInteger}
	case "list_issues":
		fields = map[string]queryKind{"db": queryText, "journal_id": queryInteger, "year": queryInteger, "sort": queryText, "limit": queryInteger, "offset": queryInteger}
	}

	return fields, isTyped
}

// extractIndexQuery leaves untyped query validation to authenticated operation selection.
func extractIndexQuery(request *http.Request, name string) (typedQuery, *apiError) {
	fields, isTyped := indexQueryFields(name)
	if !isTyped {
		return typedQuery{}, nil
	}
	return extractQuery(request.URL.RawQuery, fields)
}

// indexWork builds cancellable index work after authentication succeeds.
func (handlers *indexHandlers) indexWork(request *http.Request, name string, recordId int64, values typedQuery) (func() (any, error), *apiError) {
	if work := handlers.indexMetadataWork(request.Context(), name, recordId, values.database()); work != nil {
		return work, nil
	}
	if name == "list_journals" || name == "list_issues" || name == "list_articles" {
		return handlers.indexListWork(request, name, values)
	}
	return handlers.indexWeeklyWork(request, name)
}

// indexMetadataWork retains metadata and record lookups on the request context.
func (handlers *indexHandlers) indexMetadataWork(workContext context.Context, name string, recordId int64, database *string) func() (any, error) {
	var work func() (any, error)
	switch name {
	case "list_databases":
		work = func() (any, error) {
			paths, err := handlers.storage.ListIndexDatabases()
			for index, path := range paths {
				paths[index] = filepath.Base(path)
			}
			return paths, err
		}
	case "list_areas":
		work = func() (any, error) { return query.ListAreas(workContext, handlers.storage, database) }
	case "list_journal_ratings":
		work = func() (any, error) { return query.ListJournalRatings(workContext, handlers.storage, database) }
	case "list_journal_options":
		work = func() (any, error) { return query.ListJournalOptions(workContext, handlers.storage, database) }
	case "list_years":
		work = func() (any, error) { return query.ListYears(workContext, handlers.storage, database) }
	case "get_journal":
		work = func() (any, error) { return query.GetJournal(workContext, handlers.storage, database, recordId) }
	case "get_issue":
		work = func() (any, error) { return query.GetIssue(workContext, handlers.storage, database, recordId) }
	case "get_article":
		work = func() (any, error) { return query.GetArticle(workContext, handlers.storage, database, recordId) }

	}
	return work
}

// indexListWork preserves endpoint-specific filter conversion and pagination defaults.
func (handlers *indexHandlers) indexListWork(request *http.Request, name string, values typedQuery) (func() (any, error), *apiError) {
	workContext := request.Context()
	database := values.database()
	var work func() (any, error)
	switch name {
	case "list_journals":
		pairs, err := parseQueryPairs(request.URL.RawQuery)
		if err != nil {
			return nil, err
		}
		params := query.JournalListParams{Area: values.text("area"), Ratings: pairs.ratings(), HasArticles: values.boolean("has_articles"), Year: values.integer("year"), Sort: values.text("sort"), Limit: values.integerDefault("limit", 50), Offset: values.integerDefault("offset", 0)}
		work = func() (any, error) { return query.ListJournals(workContext, handlers.storage, database, params) }
	case "list_issues":
		params := query.IssueListParams{JournalId: values.integer("journal_id"), Year: values.integer("year"), Sort: values.text("sort"), Limit: values.integerDefault("limit", 50), Offset: values.integerDefault("offset", 0)}
		work = func() (any, error) { return query.ListIssues(workContext, handlers.storage, database, params) }
	case "list_articles":
		selected, params, err := parseArticleQuery(request.URL.RawQuery)
		if err != nil {
			return nil, err
		}
		work = func() (any, error) { return query.ListArticles(workContext, handlers.storage, selected, params) }

	}
	return work, nil
}

// indexWeeklyWork evaluates the current week inside admitted work where applicable.
func (handlers *indexHandlers) indexWeeklyWork(request *http.Request, name string) (func() (any, error), *apiError) {
	workContext := request.Context()
	var work func() (any, error)
	switch name {
	case "get_weekly_updates":
		work = func() (any, error) {
			return query.WeeklyUpdates(workContext, handlers.storage, weekly.FromTime(time.Now()))
		}
	case "get_weekly_updates_summary":
		work = func() (any, error) {
			return query.WeeklySummary(workContext, handlers.storage, weekly.FromTime(time.Now()), handlers.cache)
		}
	case "get_weekly_update_articles":
		params, err := parseWeeklyArticleQuery(request.URL.RawQuery)
		if err != nil {
			return nil, err
		}
		work = func() (any, error) {
			return query.WeeklyArticles(workContext, handlers.storage, params, handlers.cache)
		}

	}
	return work, nil
}
