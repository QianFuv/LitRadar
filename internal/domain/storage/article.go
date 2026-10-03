package storage

import "github.com/QianFuv/LitRadar/internal/domain/identity"

// Article retains canonical fields independently from the listing and full-text projections.
type Article struct {
	ArticleId       identity.Id `json:"article_id"`
	JournalId       identity.Id `json:"journal_id"`
	IssueId         *int64      `json:"issue_id"`
	Title           string      `json:"title"`
	PublicationYear *int64      `json:"publication_year"`
	Date            *string     `json:"date"`
	DatePrecision   *string     `json:"date_precision"`
	Authors         []string    `json:"authors"`
	StartPage       *string     `json:"start_page"`
	EndPage         *string     `json:"end_page"`
	Abstract        *string     `json:"abstract"`
	Doi             *string     `json:"doi"`
	Pmid            *string     `json:"pmid"`
	InPress         *bool       `json:"in_press"`
	OpenAccess      *bool       `json:"open_access"`
	RetractionDois  []string    `json:"retraction_dois"`
	JournalTitle    string      `json:"journal_title"`
	Volume          *string     `json:"volume"`
	Number          *string     `json:"number"`
}
