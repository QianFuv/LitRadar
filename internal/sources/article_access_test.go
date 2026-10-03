package sources

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

func TestOriginalArticleAccessAdapters(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/sources/access-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Id, Kind string
			Article  struct {
				Title           string   `json:"title"`
				JournalTitle    string   `json:"journal_title"`
				JournalIssns    []string `json:"journal_issns"`
				PublicationYear *int64   `json:"publication_year"`
				IssueNumber     *string  `json:"issue_number"`
				Doi             *string  `json:"doi"`
				Pmid            *string  `json:"pmid"`
			}
			Fixture cnki.FixtureData
			Output  json.RawMessage
		} `json:"observations"`
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, observation := range corpus.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			input := observation.Article
			article := domain.ArticleLocator{Title: input.Title, JournalTitle: input.JournalTitle, JournalIssns: input.JournalIssns, PublicationYear: input.PublicationYear, IssueNumber: input.IssueNumber, Doi: input.Doi, Pmid: input.Pmid}
			var access provider.ArticleAbstract = ScholarlyArticleAccess{}
			if observation.Kind == "cnki" {
				access = NewCnkiArticleAccess(cnki.NewFixtureTransport(observation.Fixture))
			}
			supports := access.SupportsAbstract(article)
			result, err := access.ResolveAbstract(context.Background(), article, domain.ArticleAccessContext{})
			outcome := map[string]any{"location": result.Location}
			if err != nil {
				var failure *provider.Error
				if !errors.As(err, &failure) {
					t.Fatal(err)
				}
				outcome = map[string]any{"kind": failure.Kind, "error": failure.Message}
			}
			encoded, err := json.Marshal(map[string]any{"supports": supports, "outcome": outcome})
			if err != nil {
				t.Fatal(err)
			}
			var actual, expected any
			if json.Unmarshal(encoded, &actual) != nil || json.Unmarshal(observation.Output, &expected) != nil {
				t.Fatal("invalid oracle output")
			}
			if !reflect.DeepEqual(actual, expected) {
				t.Fatalf("want %s\ngot %s", observation.Output, encoded)
			}
		})
	}
}

type accessTransport struct {
	cnki.Transport
	page       cnki.IssueArticlePage
	details    map[string]any
	failures   map[string]error
	requested  []string
	wasDrained bool
}

func (*accessTransport) ResolveJournal(context.Context, cnki.JournalLocator) (any, error) {
	return map[string]any{}, nil
}
func (*accessTransport) YearIssues(context.Context, any) ([]any, error) {
	return []any{map[string]any{"year": "2026", "number": "1"}}, nil
}
func (fixture *accessTransport) IssueArticles(context.Context, any, any, uint64) (cnki.IssueArticlePage, error) {
	return fixture.page, nil
}
func (fixture *accessTransport) ArticleDetail(_ context.Context, url string, _ *string) (any, error) {
	fixture.requested = append(fixture.requested, url)
	return fixture.details[url], fixture.failures[url]
}
func (fixture *accessTransport) DrainAttempts() []scholarly.Attempt {
	fixture.wasDrained = true
	return nil
}

func TestCnkiAccessRequiresExactMetadataAndSkipsOnlyPermanentMissing(t *testing.T) {
	for _, kind := range []string{"PermanentArticleMissing", "404", "410", "429", "Parse"} {
		t.Run(kind, func(t *testing.T) {
			failure := &cnki.Error{Kind: kind, Message: "private upstream response"}
			if kind == "404" || kind == "410" || kind == "429" {
				failure.Kind = "Request"
				failure.Message = "domestic CNKI HTTP status " + kind
			}
			transport := &accessTransport{details: map[string]any{"wrong": map[string]any{"title": "Expected", "doi": "10.1234/wrong"}, "match": map[string]any{"title": "EXPECTED!", "permalink": "https://kns.cnki.net/article"}}, failures: map[string]error{"missing": failure}}
			transport.page = cnki.IssueArticlePage{Articles: []any{map[string]any{"title": "Expected", "article_url": "missing"}, map[string]any{"title": "Expected", "article_url": "wrong"}, map[string]any{"title": "Expected", "article_url": "match"}}, ArticleCount: 3}
			access := NewCnkiArticleAccess(transport)
			result, err := access.ResolveAbstract(context.Background(), domain.ArticleLocator{Title: "Expected", JournalTitle: "Journal", Doi: new("10.1234/right")}, domain.ArticleAccessContext{})
			if kind == "429" || kind == "Parse" {
				if err == nil || err.Error() != "domestic CNKI provider request failed" || len(transport.requested) != 1 {
					t.Fatalf("transient failure was skipped: %+v %v", result, err)
				}
			} else if err != nil || result.Location != "https://kns.cnki.net/article" || len(transport.requested) != 3 {
				t.Fatalf("exact fallback failed: %+v %v %v", result, err, transport.requested)
			}
			if !transport.wasDrained {
				t.Fatal("attempts not drained on outcome")
			}
		})
	}
}

func TestCnkiAccessValidatesPageAndDestination(t *testing.T) {
	for _, location := range []string{"https://kns.cnki.net/article", "https://kns.cnki.net.evil.invalid/article", "http://kns.cnki.net/article", "https://navi.cnki.net/article?redirect=OVERSEA.CNKI.NET"} {
		t.Run(location, func(t *testing.T) {
			transport := &accessTransport{details: map[string]any{"detail": map[string]any{"title": "Title", "permalink": location}}}
			transport.page = cnki.IssueArticlePage{Articles: []any{map[string]any{"title": "Title", "article_url": "detail"}}, ArticleCount: 1}
			access := NewCnkiArticleAccess(transport)
			_, err := access.ResolveAbstract(context.Background(), domain.ArticleLocator{Title: "Title"}, domain.ArticleAccessContext{})
			if (err == nil) != (location == "https://kns.cnki.net/article") {
				t.Fatal(err)
			}
			transport.page.ArticleCount = 2
			_, err = access.ResolveAbstract(context.Background(), domain.ArticleLocator{Title: "Title"}, domain.ArticleAccessContext{})
			if err == nil || err.Error() != "domestic CNKI issue article page metadata is inconsistent" {
				t.Fatal(err)
			}
		})
	}
	for _, registration := range []func() (*provider.Registration, error){ScholarlyAccessRegistration, func() (*provider.Registration, error) {
		return CnkiAccessRegistration(NewCnkiArticleAccess(&accessTransport{}))
	}} {
		value, err := registration()
		if err != nil {
			t.Fatal(err)
		}
		if value.IndexContent() != nil || value.ArticleFullText() != nil || value.ArticleAbstract() == nil {
			t.Fatal("registration capability mismatch")
		}
	}
}
