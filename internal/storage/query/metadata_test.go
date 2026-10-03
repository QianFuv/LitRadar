package query

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
)

func TestOriginalRustMetadataQueries(t *testing.T) {
	root := filepath.Join("..", "..", "..", "tests", "migration", "storage")
	data, err := os.ReadFile(filepath.Join(root, "metadata-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Operation string
			Db        *string
			Id        int64
			Keys      []string
			Ids       []int64
			Params    json.RawMessage
			Output    json.RawMessage
			Error     *string
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	database, err := os.ReadFile(filepath.Join(root, "fixtures", "metadata.sqlite.fixture"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(configuration.IndexDir, "metadata.sqlite"), database, 0600); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range fixture.Cases {
		t.Run(scenario.Operation, func(t *testing.T) {
			ctx := context.Background()
			var actual any
			var err error
			var params struct {
				Area, Sort    *string
				Ratings       domain.JournalRatings
				HasArticles   *bool  `json:"has_articles"`
				JournalId     *int64 `json:"journal_id"`
				Year          *int64
				Limit, Offset int64
			}
			if scenario.Operation != "articles" && len(scenario.Params) > 0 {
				if err := json.Unmarshal(scenario.Params, &params); err != nil {
					t.Fatal(err)
				}
			}
			switch scenario.Operation {
			case "issue_counts":
				actual, err = CollectIssueArticleCounts(ctx, filepath.Join(configuration.IndexDir, "metadata.sqlite"))
			case "inpress_counts":
				actual, err = CollectInPressArticleCounts(ctx, filepath.Join(configuration.IndexDir, "metadata.sqlite"))
			case "issue_candidates":
				actual, err = FetchCandidatesForIssueKeys(ctx, filepath.Join(configuration.IndexDir, "metadata.sqlite"), scenario.Keys)
			case "inpress_candidates":
				actual, err = FetchCandidatesForInPressKeys(ctx, filepath.Join(configuration.IndexDir, "metadata.sqlite"), scenario.Keys)
			case "article_candidates":
				actual, err = FetchCandidatesForArticleIds(ctx, filepath.Join(configuration.IndexDir, "metadata.sqlite"), scenario.Ids)
			case "articles":
				selected := DefaultArticleListParams()
				if err := json.Unmarshal(scenario.Params, &selected); err != nil {
					t.Fatal(err)
				}
				actual, err = ListArticles(ctx, configuration, scenario.Db, selected)
			case "article":
				actual, err = GetArticle(ctx, configuration, scenario.Db, scenario.Id)
			case "journals":
				actual, err = ListJournals(ctx, configuration, scenario.Db, JournalListParams{Area: params.Area, Sort: params.Sort, Ratings: params.Ratings, HasArticles: params.HasArticles, Year: params.Year, Limit: params.Limit, Offset: params.Offset})
			case "issues":
				actual, err = ListIssues(ctx, configuration, scenario.Db, IssueListParams{Sort: params.Sort, JournalId: params.JournalId, Year: params.Year, Limit: params.Limit, Offset: params.Offset})
			case "journal":
				actual, err = GetJournal(ctx, configuration, scenario.Db, scenario.Id)
			case "issue":
				actual, err = GetIssue(ctx, configuration, scenario.Db, scenario.Id)
			case "areas":
				actual, err = ListAreas(ctx, configuration, scenario.Db)
			case "ratings":
				actual, err = ListJournalRatings(ctx, configuration, scenario.Db)
			case "options":
				actual, err = ListJournalOptions(ctx, configuration, scenario.Db)
			case "years":
				actual, err = ListYears(ctx, configuration, scenario.Db)
			default:
				t.Fatal("unknown operation")
			}
			if scenario.Error != nil {
				if err == nil || err.Error() != *scenario.Error {
					t.Fatalf("error=%v expected=%s", err, *scenario.Error)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var expected, decoded any
			if err := json.Unmarshal(scenario.Output, &expected); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(encoded, &decoded); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(decoded, expected) {
				t.Fatalf("Go: %s\nRust: %s", encoded, scenario.Output)
			}
		})
	}
}
