package storage

import "github.com/QianFuv/LitRadar/internal/domain/identity"

// WeeklyArticle is the fixed public subset of canonical article metadata.
type WeeklyArticle struct {
	ArticleId       identity.Id `json:"article_id"`
	JournalId       identity.Id `json:"journal_id"`
	IssueId         *int64      `json:"issue_id"`
	Title           string      `json:"title"`
	PublicationYear *int64      `json:"publication_year"`
	Date            *string     `json:"date"`
	DatePrecision   *string     `json:"date_precision"`
	Authors         []string    `json:"authors"`
	Abstract        *string     `json:"abstract"`
	Doi             *string     `json:"doi"`
	JournalTitle    string      `json:"journal_title"`
	OpenAccess      *bool       `json:"open_access"`
	InPress         *bool       `json:"in_press"`
	Volume          *string     `json:"volume"`
	Number          *string     `json:"number"`
}

// WeeklyJournalSummary supports bounded navigation without loading article bodies.
type WeeklyJournalSummary struct {
	JournalId       identity.Id `json:"journal_id"`
	JournalTitle    *string     `json:"journal_title"`
	NewArticleCount int         `json:"new_article_count"`
}

// WeeklyJournalUpdate adds ordered article bodies only for the bounded legacy aggregate.
type WeeklyJournalUpdate struct {
	WeeklyJournalSummary
	Articles []WeeklyArticle `json:"articles"`
}

// WeeklyDatabase keeps the original newest publication identity after combining membership.
type WeeklyDatabase[Journal any] struct {
	DbName          string    `json:"db_name"`
	RunId           *string   `json:"run_id"`
	GeneratedAt     string    `json:"generated_at"`
	NewArticleCount int       `json:"new_article_count"`
	Journals        []Journal `json:"journals"`
}

// WeeklyResponse carries one fixed inclusive window across all database groups.
type WeeklyResponse[Journal any] struct {
	GeneratedAt string                    `json:"generated_at"`
	WindowStart string                    `json:"window_start"`
	WindowEnd   string                    `json:"window_end"`
	Databases   []WeeklyDatabase[Journal] `json:"databases"`
}
