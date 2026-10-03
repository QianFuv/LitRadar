package provider

import (
	"context"
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"testing"

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

func TestFrozenRustProviderContracts(t *testing.T) {
	body, err := os.ReadFile("../../tests/migration/sources/provider-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Observations []struct {
			Kind, Type    string
			Input, Output json.RawMessage
		}
	}
	if err := json.Unmarshal(body, &fixture); err != nil {
		t.Fatal(err)
	}
	for index, observation := range fixture.Observations {
		t.Run(observation.Kind+"/"+strconv.Itoa(index), func(t *testing.T) {
			var actual any
			var err error
			decode := func(target any) {
				t.Helper()
				if err := json.Unmarshal(observation.Input, target); err != nil {
					t.Fatal(err)
				}
			}
			switch observation.Kind {
			case "normalize":
				var input string
				decode(&input)
				actual = map[string]any{"text": domain.NormalizeText(input), "date": domain.NormalizeDate(input), "doi": domain.NormalizeDoi(input), "pmid": domain.NormalizePmid(input), "issn": domain.NormalizeIssn(input), "bibliographic": domain.NormalizeBibliographicText(input), "label": domain.NormalizeBibliographicLabel(input), "lowercase": domain.Lowercase(input)}
			case "catalog":
				var input domain.JournalCatalogEntry
				decode(&input)
				err = ValidateCatalogEntry(input)
			case "batch":
				var input struct {
					Catalog domain.JournalCatalogEntry
					Batch   domain.ProviderBatch
				}
				decode(&input)
				err = ValidateProviderBatch(input.Catalog, input.Batch)
			case "redirect":
				var input string
				decode(&input)
				err = ValidateArticleRedirect(domain.ArticleRedirect{Location: input})
			case "document":
				var input struct {
					ContentType   string `json:"content_type"`
					Filename      *string
					Size, Maximum int
				}
				decode(&input)
				err = ValidateFullTextResolution(domain.ArticleFullTextResolution{Document: &domain.ArticleFullTextDocument{ContentType: input.ContentType, Filename: input.Filename, Bytes: make([]byte, input.Size)}}, input.Maximum)
			case "registry":
				var input struct {
					Name                          string
					Capabilities, Implementations []bool
					Hosts                         []string
				}
				decode(&input)
				implementations := Implementations{}
				if input.Implementations[0] {
					implementations.IndexContent = noExecution{}
				}
				if input.Implementations[1] {
					implementations.ArticleAbstract = noExecution{}
				}
				if input.Implementations[2] {
					implementations.ArticleFullText = noExecution{}
				}
				_, err = NewRegistration(Descriptor{Name: input.Name, Capabilities: Capabilities{input.Capabilities[0], input.Capabilities[1], input.Capabilities[2]}, AllowedRedirectHosts: input.Hosts}, implementations)
			case "decode":
				var input string
				decode(&input)
				var target any
				switch observation.Type {
				case "catalog":
					target = &domain.JournalCatalogEntry{}
				case "rankings":
					target = &domain.JournalRankings{}
				case "journal":
					target = &domain.JournalDraft{}
				case "issue":
					target = &domain.IssueDraft{}
				case "author":
					target = &domain.ArticleAuthorDraft{}
				case "article":
					target = &domain.ArticleDraft{}
				case "batch":
					target = &domain.ProviderBatch{}
				case "progress":
					target = &domain.ProviderProgress{}
				case "mode":
					target = new(domain.IndexSyncMode)
				default:
					t.Fatal("unknown contract")
				}
				if decodeErr := json.Unmarshal([]byte(input), target); decodeErr != nil {
					actual = map[string]any{"valid": false}
				} else {
					actual = map[string]any{"valid": true, "value": target}
				}
			default:
				t.Fatal("unknown observation")
			}
			if actual == nil {
				if err == nil {
					actual = map[string]any{"error": nil}
				} else {
					actual = map[string]any{"error": err.Error()}
				}
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var actualValue, expectedValue any
			if err := json.Unmarshal(encoded, &actualValue); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(observation.Output, &expectedValue); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(actualValue, expectedValue) {
				input := string(observation.Input)
				if len(input) > 512 {
					input = input[:512]
				}
				t.Fatalf("input %s\nactual %s\nexpected %s", input, encoded, observation.Output)
			}
		})
	}
}
