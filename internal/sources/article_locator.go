package sources

import (
	"context"
	"database/sql"
	"errors"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/query"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

type locatorNames []string

func (names *locatorNames) Scan(value any) error {
	var text storage.Text
	if err := text.Scan(value); err != nil {
		return err
	}
	decoded, err := search.DecodeAuthorNames(string(text))
	if err != nil {
		return err
	}
	*names = decoded
	return nil
}

// GetArticleLocator reads only canonical access metadata in one consistent joined query.
func GetArticleLocator(ctx context.Context, configuration config.Config, name *string, id identity.Id) (domain.ArticleLocator, error) {
	filename, err := configuration.ResolveIndexDbPath(name)
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	defer database.Close()
	var articleId storage.Integer
	var catalog, title, journal storage.Text
	var issns, authors locatorNames
	var year storage.OptionalInteger
	var date, volume, number, start, end, doi, pmid storage.OptionalText
	err = database.QueryRowContext(ctx, `SELECT a.article_id,j.catalog_id,j.title,j.issns_json,a.title,a.publication_year,a.date,a.authors_json,i.volume,i.number,a.start_page,a.end_page,a.doi,a.pmid FROM articles a JOIN journals j ON j.journal_id=a.journal_id LEFT JOIN issues i ON i.issue_id=a.issue_id WHERE a.article_id=?`, int64(id)).Scan(&articleId, &catalog, &journal, &issns, &title, &year, &date, &authors, &volume, &number, &start, &end, &doi, &pmid)
	if errors.Is(err, sql.ErrNoRows) {
		return domain.ArticleLocator{}, query.NotFound{Message: "Article not found"}
	}
	if err != nil {
		return domain.ArticleLocator{}, err
	}
	return domain.ArticleLocator{ArticleId: identity.Id(articleId), CatalogId: string(catalog), JournalTitle: string(journal), JournalIssns: []string(issns), Title: string(title), PublicationYear: year.Value, Date: date.Value, Authors: []string(authors), Volume: volume.Value, IssueNumber: number.Value, StartPage: start.Value, EndPage: end.Value, Doi: doi.Value, Pmid: pmid.Value}, nil
}
