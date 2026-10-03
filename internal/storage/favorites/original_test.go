package favorites

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestOriginalRustFavoriteQueries(t *testing.T) {
	root := filepath.Join("..", "..", "..", "tests", "migration", "storage")
	raw, err := os.ReadFile(filepath.Join(root, "favorites-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Operation, Sql, Db    string
			Owner                 *int64
			Folder, Limit, Offset *int64
			Id                    int64
			Ids                   []int64
			Cursor                *string
			References            []struct {
				Id int64
				Db string
			}
			IndexSql      string `json:"index_sql"`
			Ambiguous     bool
			Output        json.RawMessage
			Error         *string
			ErrorCategory bool `json:"error_category"`
		}
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	source, err := os.ReadFile(filepath.Join(root, "fixtures", "favorites-auth.sqlite.fixture"))
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range fixture.Cases {
		t.Run(scenario.Operation, func(t *testing.T) {
			ctx := context.Background()
			configuration := config.FromProjectRoot(t.TempDir())
			installMetadata(t, configuration)
			if err := os.WriteFile(configuration.AuthDbPath, source, 0600); err != nil {
				t.Fatal(err)
			}
			pool, err := auth.Open(configuration.AuthDbPath)
			if err != nil {
				t.Fatal(err)
			}
			defer pool.Close()
			repository := New(pool)
			if scenario.Ambiguous {
				raw, err := os.ReadFile(filepath.Join(configuration.IndexDir, "metadata.sqlite"))
				if err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(filepath.Join(configuration.IndexDir, "other.sqlite"), raw, 0600); err != nil {
					t.Fatal(err)
				}
			}
			if scenario.Sql != "" {
				runSql(t, repository, scenario.Sql)
			}
			if scenario.IndexSql != "" {
				database, err := storage.Open(filepath.Join(configuration.IndexDir, "metadata.sqlite"), false, 1)
				if err != nil {
					t.Fatal(err)
				}
				_, err = database.ExecContext(ctx, scenario.IndexSql)
				database.Close()
				if err != nil {
					t.Fatal(err)
				}
			}
			owner := identity.Id(1)
			if scenario.Owner != nil {
				owner = identity.Id(*scenario.Owner)
			}
			folder, limit, offset := int64(10), int64(50), int64(0)
			if scenario.Folder != nil {
				folder = *scenario.Folder
			}
			if scenario.Limit != nil {
				limit = *scenario.Limit
			}
			if scenario.Offset != nil {
				offset = *scenario.Offset
			}
			var actual any
			switch scenario.Operation {
			case "folders":
				actual, err = repository.ListFolders(ctx, owner)
			case "tracking":
				actual, err = repository.TrackingFolder(ctx, owner)
			case "count":
				actual, err = repository.CountFavorites(ctx, owner, scenario.Folder)
			case "list":
				actual, err = repository.ListArticles(ctx, configuration, owner, scenario.Folder, limit, offset)
			case "page":
				actual, err = repository.ArticlePage(ctx, configuration, owner, folder, limit, scenario.Cursor)
			case "snapshot":
				var result CitationSnapshot
				result, err = repository.LoadCitationSnapshot(ctx, owner, folder, uint64(limit))
				actual = map[string]any{"folder_name": result.FolderName, "references": result.References, "has_more": result.HasMore}
			case "citation":
				references := []Reference{}
				for _, item := range scenario.References {
					references = append(references, Reference{identity.Id(item.Id), item.Db})
				}
				actual, err = LoadCitationRecords(ctx, configuration, references)
			case "check":
				actual, err = repository.IsFavorited(ctx, owner, Reference{identity.Id(scenario.Id), scenario.Db})
			case "batch":
				actual, err = repository.BatchIsFavorited(ctx, owner, scenario.Ids, scenario.Db)
			default:
				t.Fatal("unknown operation")
			}
			if scenario.Error != nil {
				if err == nil || (!scenario.ErrorCategory && err.Error() != *scenario.Error) {
					t.Fatalf("error=%v want %s", err, *scenario.Error)
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
		})
	}
}
