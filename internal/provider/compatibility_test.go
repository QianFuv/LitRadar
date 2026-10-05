package provider

import (
	"context"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

type noExecution struct{}

func (noExecution) Fetch(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error) {
	panic("registry test must not fetch")
}
func (noExecution) SupportsAbstract(domain.ArticleLocator) bool {
	panic("registry test must not resolve")
}
func (noExecution) ResolveAbstract(context.Context, domain.ArticleLocator, domain.ArticleAccessContext) (domain.ArticleRedirect, error) {
	panic("registry test must not resolve")
}
func (noExecution) SupportsFullText(domain.ArticleLocator) bool {
	panic("registry test must not resolve")
}
func (noExecution) ResolveFullText(context.Context, domain.ArticleLocator, domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
	panic("registry test must not resolve")
}
