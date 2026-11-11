package api

import (
	"context"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/sources"
	"time"
)

type articleLocation struct {
	article domain.ArticleLocator
	stem    string
	user    identity.ID
}
type articleOutput struct {
	// Status contains the status value.
	Status int
	// Body contains the decoded operation payload.
	Body []byte `format:"binary" encoding:"binary"`
	// Location sets the Location header.
	Location string `header:"Location"`
	// CacheControl sets the Cache-Control header.
	CacheControl string `header:"Cache-Control"`
	// ContentType sets the Content-Type header.
	ContentType string `header:"Content-Type"`
	// ContentDisposition sets the Content-Disposition header.
	ContentDisposition string `header:"Content-Disposition"`
}

func (handlers *articleHandlers) locate(ctx context.Context, metadata RequestMetadata, articleID int64, database *string) (articleLocation, error) {
	current, failure := handlers.authenticator.requireUser(metadata.request)
	if failure != nil {
		return articleLocation{}, failure.problem()
	}
	value, err := admitted(ctx, handlers.pool, func() (articleLocation, error) {
		stem, err := handlers.storage.ResolveIndexCatalogStem(cleanText(database))
		if err != nil {
			return articleLocation{}, err
		}
		article, err := sources.GetArticleLocator(context.WithoutCancel(ctx), handlers.storage, cleanText(database), identity.ID(articleID))
		return articleLocation{article: article, stem: stem, user: current.authorization.User.ID}, err
	}, mapIndexError)
	if err != nil {
		return articleLocation{}, err
	}
	return value.Body, nil
}
func (handlers *articleHandlers) articleContent(ctx context.Context, metadata RequestMetadata, articleID int64, database *string, action string) (*articleOutput, error) {
	located, err := handlers.locate(ctx, metadata, articleID, database)
	if err != nil {
		return nil, err
	}
	resolved, failure := handlers.resolve(ctx, located.article, located.user, located.stem, action, time.Now().Add(30*time.Second))
	if failure != nil {
		return nil, failure.problem()
	}
	return resolvedArticleOutput(resolved), nil
}

func resolvedArticleOutput(resolved domain.ArticleFullTextResolution) *articleOutput {
	result := &articleOutput{CacheControl: "private, no-store"}
	if resolved.Redirect != nil {
		result.Status = 307
		result.Location = resolved.Redirect.Location
		return result
	}
	result.Status = 200
	result.ContentType = resolved.Document.ContentType
	result.Body = resolved.Document.Bytes
	if resolved.Document.Filename != nil {
		result.ContentDisposition = "attachment; filename*=UTF-8''" + articleFilename(*resolved.Document.Filename)
	}
	return result
}
func (handlers *articleHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/articles/{article_id}/access", ID: "get_article_access"}, func(ctx context.Context, input *GetArticleAccessInput) (*output[*articleAccess], error) {
			located, err := handlers.locate(ctx, input.RequestMetadata, input.ArticleID, input.DB.Pointer())
			if err != nil {
				return nil, err
			}
			status, failure := handlers.status(ctx, located.article, located.user, located.stem)
			if failure != nil {
				return nil, failure.problem()
			}
			return &output[*articleAccess]{Body: status}, nil
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/articles/{article_id}/abstract", ID: "redirect_article_abstract"}, func(ctx context.Context, input *RedirectArticleAbstractInput) (*articleOutput, error) {
			return handlers.articleContent(ctx, input.RequestMetadata, input.ArticleID, input.DB.Pointer(), "abstract")
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/articles/{article_id}/fulltext", ID: "redirect_article_fulltext"}, func(ctx context.Context, input *RedirectArticleFulltextInput) (*articleOutput, error) {
			return handlers.articleContent(ctx, input.RequestMetadata, input.ArticleID, input.DB.Pointer(), "fulltext")
		}),
	}
}
