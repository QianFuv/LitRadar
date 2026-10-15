package favorites

import (
	"context"
	"fmt"
	"path/filepath"
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	contentfixture "github.com/QianFuv/LitRadar/internal/testkit/content"
)

// metadataBenchmarkFixture creates real catalog groups with distinct titles and overlapping numeric article IDs.
func metadataBenchmarkFixture(t testing.TB, count, groups int) (config.Config, []Reference) {
	t.Helper()
	configuration, first, _ := favoriteMetadataFixture(t)
	for group := range groups {
		database := first
		name := "metadata"
		if group > 0 {
			name = "second"
			filename := filepath.Join(configuration.IndexDir, name+".sqlite")
			if err := contentfixture.Create(filename, 9); err != nil {
				t.Fatal(err)
			}
			var err error
			database, err = storage.OpenPlain(filename)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { database.Close() })
		}
		if _, err := database.Exec(`WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM ids WHERE id<?) INSERT INTO articles(article_id,journal_id,title,authors_json,date,doi,abstract_text) SELECT id+2000,1,?||id,'[{"display_name":"Researcher"}]','2026-01-02','10.1234/metadata.'||id,'Research abstract' FROM ids`, count, name+" article "); err != nil {
			t.Fatal(err)
		}
	}
	references := make([]Reference, count)
	for index := range references {
		name := "metadata"
		if index%groups != 0 {
			name = "second"
		}
		references[index] = Reference{identity.Id(2000 + count - index), name}
	}
	return configuration, references
}

// TestMetadataGroupMapsPreserveOrderAndPerCallOwnership checks equal IDs from separate catalogs and duplicate references.
func TestMetadataGroupMapsPreserveOrderAndPerCallOwnership(t *testing.T) {
	configuration, _ := metadataBenchmarkFixture(t, 1, 2)
	references := []Reference{{2001, "metadata"}, {2001, "second"}, {2001, "metadata"}, {2001, "missing"}}
	favorites := make([]Favorite, len(references))
	for index, reference := range references {
		favorites[index] = Favorite{Reference: reference, Note: fmt.Sprint(index)}
	}
	ctx := context.Background()
	items := Enrich(ctx, configuration, favorites)
	records, err := LoadCitationRecords(ctx, configuration, references)
	if err != nil || len(items) != 4 || len(records) != 4 {
		t.Fatal("metadata lost references", len(items), len(records), err)
	}
	for index, title := range []string{"metadata article 1", "second article 1", "metadata article 1"} {
		if items[index].Title == nil || *items[index].Title != title || items[index].Note != fmt.Sprint(index) || records[index].Title == nil || *records[index].Title != title || records[index].DbName != references[index].DbName {
			t.Fatal("metadata group ownership or request order changed", index, items[index], records[index])
		}
	}
	if items[3].MetadataStatus != "missing" || records[3].Title != nil || records[3].Authors == nil {
		t.Fatal("missing group projection changed", items[3], records[3])
	}
	if items[0].Title != items[2].Title || items[0].Title == items[1].Title {
		t.Fatal("duplicate and cross-database pointer ownership changed")
	}
	*items[0].Title = "mutated projection"
	if fresh := Enrich(ctx, configuration, favorites); fresh[0].Title == nil || *fresh[0].Title != "metadata article 1" {
		t.Fatal("metadata output leaked across calls", fresh)
	}
}

// BenchmarkFavoriteMetadata measures real single- and multiple-catalog enrichments and citation exports.
func BenchmarkFavoriteMetadata(b *testing.B) {
	for _, scenario := range []struct {
		name       string
		count      int
		isCitation bool
	}{{"enrich", 500, false}, {"citations", 1000, true}, {"citations", 10000, true}} {
		for _, groups := range []int{1, 2} {
			b.Run(fmt.Sprintf("%s=%d/groups=%d", scenario.name, scenario.count, groups), func(b *testing.B) {
				configuration, references := metadataBenchmarkFixture(b, scenario.count, groups)
				favorites := make([]Favorite, len(references))
				for index, reference := range references {
					favorites[index] = Favorite{Reference: reference}
				}
				b.ReportAllocs()
				for b.Loop() {
					if scenario.isCitation {
						records, err := LoadCitationRecords(context.Background(), configuration, references)
						if err != nil || len(records) != scenario.count || records[0].Title == nil {
							b.Fatal("citation metadata changed", len(records), err)
						}
					} else {
						items := Enrich(context.Background(), configuration, favorites)
						if len(items) != scenario.count || items[0].MetadataStatus != "available" {
							b.Fatal("favorite metadata changed", len(items))
						}
					}
				}
			})
		}
	}
}
