package query

import (
	"context"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// pointerCandidateFromRow retains the original nullable-pointer mapping as a performance reference.
func pointerCandidateFromRow(row scanner) (domain.ArticleCandidate, error) {
	var id, journal storage.Integer
	var issue, openAccess, inPress storage.OptionalInteger
	var title, journalTitle storage.Text
	var abstract, date, doi storage.OptionalText
	if err := row.Scan(&id, &journal, &issue, &title, &abstract, &date, &openAccess, &inPress, &doi, &journalTitle); err != nil {
		return domain.ArticleCandidate{}, err
	}
	text := ""
	if abstract.Value != nil {
		text = *abstract.Value
	}
	return domain.ArticleCandidate{ArticleId: int64(id), JournalId: int64(journal), IssueId: issue.Value, Title: string(title), Abstract: text, Date: date.Value, JournalTitle: string(journalTitle), Doi: doi.Value, OpenAccess: openAccess.Value != nil && *openAccess.Value != 0, InPress: inPress.Value != nil && *inPress.Value != 0}, nil
}

// BenchmarkCandidateRowDecoding compares native reads with equivalent pointer and inline nullable mappings.
func BenchmarkCandidateRowDecoding(b *testing.B) {
	_, filename := benchmarkCatalog(b)
	database, err := storage.OpenPlain(filename)
	if err != nil {
		b.Fatal(err)
	}
	defer database.Close()
	for _, variant := range []struct {
		name   string
		decode func(scanner) (domain.ArticleCandidate, error)
	}{{"pointer", pointerCandidateFromRow}, {"inline", candidateFromRow}} {
		b.Run(variant.name, func(b *testing.B) {
			b.ReportAllocs()
			for b.Loop() {
				items, err := collect(context.Background(), database, `SELECT a.article_id,a.journal_id,a.issue_id,a.title,a.abstract_text,a.date,a.open_access,a.in_press,a.doi,j.title FROM articles a JOIN journals j ON j.journal_id=a.journal_id WHERE a.article_id<=32766 ORDER BY a.date DESC,a.article_id DESC`, nil, variant.decode)
				if err != nil || len(items) != 32766 {
					b.Fatal("candidate rows changed", len(items), err)
				}
			}
		})
	}
}
