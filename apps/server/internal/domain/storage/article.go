package storage

import "github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"

// Article retains canonical fields independently from the listing and full-text projections.
type Article struct {
	ArticleId       identity.Id `json:"article_id" nullable:"false"`
	JournalId       identity.Id `json:"journal_id" nullable:"false"`
	IssueId         *int64      `json:"issue_id" required:"false" nullable:"true" format:"int64"`
	Title           string      `json:"title" nullable:"false"`
	PublicationYear *int64      `json:"publication_year" required:"false" nullable:"true" format:"int64"`
	Date            *string     `json:"date" required:"false" nullable:"true"`
	DatePrecision   *string     `json:"date_precision" required:"false" nullable:"true" enum:"year,month,day"`
	Authors         []string    `json:"authors" nullable:"false"`
	StartPage       *string     `json:"start_page" required:"false" nullable:"true"`
	EndPage         *string     `json:"end_page" required:"false" nullable:"true"`
	Abstract        *string     `json:"abstract" required:"false" nullable:"true"`
	Doi             *string     `json:"doi" required:"false" nullable:"true"`
	Pmid            *string     `json:"pmid" required:"false" nullable:"true"`
	InPress         *bool       `json:"in_press" required:"false" nullable:"true"`
	OpenAccess      *bool       `json:"open_access" required:"false" nullable:"true"`
	RetractionDois  []string    `json:"retraction_dois" nullable:"false"`
	JournalTitle    string      `json:"journal_title" nullable:"false"`
	Volume          *string     `json:"volume" required:"false" nullable:"true"`
	Number          *string     `json:"number" required:"false" nullable:"true"`
}
