package query

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

func TestOriginalRustWeeklyQueries(t *testing.T) {
	root := filepath.Join("..", "..", "..", "tests", "migration", "storage")
	raw, err := os.ReadFile(filepath.Join(root, "weekly-query-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Operation, End, Sql string
			Selected            []string
			Manifests           map[string]string
			Params              struct {
				Db        string
				JournalId int64   `json:"journal_id"`
				WindowEnd *string `json:"window_end"`
				Query     *string `json:"q"`
				Limit     int64
				Cursor    *string
			}
			Output        json.RawMessage
			Error         *string
			ErrorCategory bool `json:"error_category"`
		}
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	source, err := os.ReadFile(filepath.Join(root, "fixtures", "metadata.sqlite.fixture"))
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range fixture.Cases {
		t.Run(scenario.Operation, func(t *testing.T) {
			ctx := context.Background()
			configuration := config.FromProjectRoot(t.TempDir())
			if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
				t.Fatal(err)
			}
			filename := filepath.Join(configuration.IndexDir, "metadata.sqlite")
			if err := os.WriteFile(filename, source, 0600); err != nil {
				t.Fatal(err)
			}
			if scenario.Sql != "" {
				database, err := storage.Open(filename, false, 1)
				if err != nil {
					t.Fatal(err)
				}
				_, err = database.ExecContext(ctx, scenario.Sql)
				database.Close()
				if err != nil {
					t.Fatal(err)
				}
			}
			directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
			if err := os.MkdirAll(directory, 0700); err != nil {
				t.Fatal(err)
			}
			for name, contents := range scenario.Manifests {
				if err := os.WriteFile(filepath.Join(directory, name), []byte(contents), 0600); err != nil {
					t.Fatal(err)
				}
			}
			end, ok := weekly.ParseTimestamp(scenario.End)
			if !ok {
				t.Fatal("invalid fixture time")
			}
			for _, cache := range []*weekly.Cache{nil, {}} {
				var actual any
				var err error
				switch scenario.Operation {
				case "summary":
					actual, err = WeeklySummary(ctx, configuration, end, cache)
				case "legacy":
					actual, err = WeeklyUpdates(ctx, configuration, end)
				case "page":
					window := scenario.End
					if scenario.Params.WindowEnd != nil {
						window = *scenario.Params.WindowEnd
					}
					actual, err = WeeklyArticles(ctx, configuration, WeeklyArticlePageParams{scenario.Params.Db, scenario.Params.JournalId, window, scenario.Params.Query, scenario.Params.Limit, scenario.Params.Cursor}, cache)
				case "available":
					var manifests []weekly.Manifest
					manifests, err = weekly.LoadAvailable(ctx, configuration, end, scenario.Selected)
					values := []map[string]any{}
					for _, manifest := range manifests {
						ids := make([]string, len(manifest.ArticleIds))
						for index, id := range manifest.ArticleIds {
							ids[index] = strconv.FormatInt(id, 10)
						}
						values = append(values, map[string]any{"db_name": manifest.DbName, "run_id": manifest.RunId, "ids": ids})
					}
					actual = values
				default:
					t.Fatal("unknown operation")
				}
				if scenario.Error != nil {
					if err == nil || (!scenario.ErrorCategory && err.Error() != *scenario.Error) {
						t.Fatalf("error=%v want %s", err, *scenario.Error)
					}
					continue
				}
				if err != nil {
					t.Fatal(err)
				}
				encoded, err := json.Marshal(actual)
				if err != nil {
					t.Fatal(err)
				}
				var got, want any
				if err := json.Unmarshal(encoded, &got); err != nil {
					t.Fatal(err)
				}
				if err := json.Unmarshal(scenario.Output, &want); err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(got, want) {
					t.Fatalf("got %s\nwant %s", encoded, scenario.Output)
				}
			}
		})
	}
}
