package sources

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/query"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/querydb"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// GetArticleLocator reads only canonical access metadata in one consistent joined query.
func GetArticleLocator(ctx context.Context, configuration config.Config, name *string, id identity.Id) (resultValue1 domain.ArticleLocator, resultError error) {
	filename, err := configuration.ResolveIndexDbPath(name)
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	queries := querydb.Queries{}
	row, err := queries.ArticleLocator(ctx, database, querydb.ArticleLocatorParams{ArticleID: storage.Integer(id)})
	if errors.Is(err, sql.ErrNoRows) {
		return domain.ArticleLocator{}, query.NotFound{Message: "Article not found"}
	}
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	issns, err := search.DecodeAuthorNames(string(row.IssnsJSON))
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	authors, err := search.DecodeAuthorNames(string(row.AuthorsJSON))
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	return domain.ArticleLocator{ArticleId: identity.Id(row.ArticleID), CatalogId: string(row.CatalogID), JournalTitle: string(row.JournalTitle), JournalIssns: issns, Title: string(row.Title), PublicationYear: (*int64)(row.PublicationYear), Date: (*string)(row.Date), Authors: authors, Volume: (*string)(row.Volume), IssueNumber: (*string)(row.IssueNumber), StartPage: (*string)(row.StartPage), EndPage: (*string)(row.EndPage), Doi: (*string)(row.DOI), Pmid: (*string)(row.PMID)}, nil
}
