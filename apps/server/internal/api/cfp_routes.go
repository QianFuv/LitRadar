package api

import (
	"context"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"slices"
	"time"
	"unicode/utf8"
)

func (handlers *cfpHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/cfp/journals", ID: "list_cfp_journals"}, func(ctx context.Context, input *ListCFPJournalsInput) (*output[*cfpCatalogResponse], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			if input.Q.Pointer() != nil && utf8.RuneCountInString(*input.Q.Pointer()) > 256 {
				return nil, badRequest("CFP journal search exceeds 256 characters").problem()
			}
			return admitted(ctx, handlers.pool, func() (*cfpCatalogResponse, error) {
				entries, failure := handlers.catalogMembers(input.DB)
				if failure != nil {
					return nil, failure
				}
				snapshots, err := handlers.repository.LoadJournals(context.WithoutCancel(ctx))
				if err != nil {
					return nil, internalError()
				}
				value, failure := cfpCatalog(input.DB, cleanText(input.Q.Pointer()), entries, snapshots, time.Now().Unix())
				if failure != nil {
					return nil, failure
				}
				return value, nil
			}, mapBusinessError)
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/cfp/journals/{catalog_id}/notices", ID: "list_cfp_notices"}, func(ctx context.Context, input *ListCFPNoticesInput) (*output[*cfpNoticePage], error) {
			if _, failure := handlers.authenticator.requireUser(input.request); failure != nil {
				return nil, failure.problem()
			}
			limit := numberOr(input.Limit.Pointer(), 50)
			if limit < 1 || limit > 200 {
				return nil, badRequest("CFP page limit must be between 1 and 200").problem()
			}
			cursor := cleanText(input.Cursor.Pointer())
			if cursor != nil && len(*cursor) > 4096 {
				return nil, cfpCursorError().problem()
			}
			return admitted(ctx, handlers.pool, func() (*cfpNoticePage, error) {
				entries, failure := handlers.catalogMembers(input.DB)
				if failure != nil {
					return nil, failure
				}
				var entry *sources.JournalCatalogEntry
				for index := range entries {
					if entries[index].CatalogId == input.CatalogID || slices.Contains(entries[index].CatalogAliases, input.CatalogID) {
						entry = &entries[index]
						break
					}
				}
				if entry == nil {
					return nil, &apiError{status: 404, detail: "CFP journal catalog member not found"}
				}
				snapshots, err := handlers.repository.LoadJournalsForCatalogIds(context.WithoutCancel(ctx), append([]string{entry.CatalogId}, entry.CatalogAliases...))
				if err != nil {
					return nil, internalError()
				}
				snapshot, failure := cfpFindSnapshot(*entry, snapshots)
				if failure != nil {
					return nil, failure
				}
				includeClosed := input.IncludeClosed.Pointer() != nil && *input.IncludeClosed.Pointer()
				value, failure := cfpPage(handlers.codec, input.DB, *entry, snapshot, includeClosed, cursor, uint64(limit), time.Now().Unix())
				if failure != nil {
					return nil, failure
				}
				return value, nil
			}, mapBusinessError)
		}),
	}
}
