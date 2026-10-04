package index

import (
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func TestOriginalRustIdentityAndCatalog(t *testing.T) {
	bytes, err := os.ReadFile("../../tests/migration/index/identity-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input    json.RawMessage
			Expected json.RawMessage
		}
	}
	if err := json.Unmarshal(bytes, &corpus); err != nil {
		t.Fatal(err)
	}
	for position, item := range corpus.Observations {
		t.Run(strconv.Itoa(position), func(t *testing.T) {
			var input struct {
				Op          string
				CatalogId   string `json:"catalog_id"`
				JournalId   string `json:"journal_id"`
				Issue       domain.IssueDraft
				Article     domain.ArticleDraft
				Aliases     []struct{ Kind, Value, Owner string }
				Left, Right domain.ArticleDraft
				Resolved    bool
				Csv         string
				Path        string
				Rows        []map[string]string
			}
			if err := json.Unmarshal(item.Input, &input); err != nil {
				t.Fatal(err)
			}
			var actual any
			switch input.Op {
			case "catalog_path":
				name := frozenCatalogBasename(input.Path)
				actual = map[string]any{"filename": name, "csv": hasCsvExtension(name)}
			case "journal":
				actual = strconv.FormatInt(storage.JournalId(input.CatalogId), 10)
			case "issue":
				journal, _ := strconv.ParseInt(input.JournalId, 10, 64)
				var id *string
				if value := storage.IssueId(journal, input.Issue); value != nil {
					text := strconv.FormatInt(*value, 10)
					id = &text
				}
				actual = map[string]any{"key": storage.IssueIdentityValue(journal, input.Issue), "id": id}
			case "identity":
				aliases := map[storage.ArticleIdentityKey]int64{}
				for _, alias := range input.Aliases {
					owner, err := strconv.ParseInt(alias.Owner, 10, 64)
					if err != nil {
						t.Fatal(err)
					}
					aliases[storage.ArticleIdentityKey{Kind: alias.Kind, Value: alias.Value}] = owner
				}
				value, err := storage.ResolveArticleIdentity(input.Article, aliases)
				var resolution any
				if err != nil {
					resolution = map[string]any{"error": err.Error()}
				} else {
					resolution = map[string]any{"article_id": strconv.FormatInt(value.ArticleId, 10), "is_existing": value.IsExisting, "identity_key": value.IdentityKey}
				}
				actual = map[string]any{"keys": storage.ArticleIdentityKeys(input.Article), "resolution": resolution}
			case "merge":
				var value domain.ArticleDraft
				var err error
				if input.Resolved {
					value, err = storage.MergeResolvedArticleDrafts(input.Left, input.Right)
				} else {
					value, err = storage.MergeArticleDrafts(input.Left, input.Right)
				}
				if err != nil {
					actual = map[string]any{"error": err.Error()}
				} else {
					actual = map[string]any{"article": value}
				}
			case "catalog", "catalog_rows":
				var entries []domain.JournalCatalogEntry
				var err error
				if input.Op == "catalog" {
					entries, err = ParseCatalogCsv(input.Csv)
				} else {
					entries, err = BuildCatalogEntries(input.Rows)
				}
				if err != nil {
					actual = map[string]any{"error": err.Error()}
				} else {
					actual = map[string]any{"entries": entries}
				}
			default:
				t.Fatal(input.Op)
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var got, want any
			if err := json.Unmarshal(encoded, &got); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(item.Expected, &want); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("input=%s\nactual=%s\nexpected=%s", item.Input, encoded, item.Expected)
			}
		})
	}
}
