package query

import (
	"context"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// BenchmarkListJournals measures native paged metadata reads with populated optional fields.
func BenchmarkListJournals(b *testing.B) {
	configuration, filename := benchmarkCatalogRows(b, 1)
	database, err := storage.OpenPlain(filename)
	if err != nil {
		b.Fatal(err)
	}
	_, err = database.ExecContext(context.Background(), `WITH RECURSIVE sequence(id) AS (VALUES(9) UNION ALL SELECT id+1 FROM sequence WHERE id<2000)
INSERT INTO journals(journal_id,catalog_id,title,title_aliases_json,issns_json,issn,eissn,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating)
SELECT id,printf('catalog-%d',id),printf('Journal %d',id),'["Alias"]','["1234-5679"]','1234-5679','2049-3630','Research','1','A','1','4*','1','A','1','A' FROM sequence`)
	closeError := database.Close()
	if err != nil || closeError != nil {
		b.Fatal(err, closeError)
	}
	params := JournalListParams{Limit: 200}
	b.ReportAllocs()
	for b.Loop() {
		result, err := ListJournals(context.Background(), configuration, nil, params)
		if err != nil || len(result.Items) != 200 || result.Page.Total == nil || *result.Page.Total != 2000 {
			b.Fatal("journal metadata page changed", result, err)
		}
	}
}
