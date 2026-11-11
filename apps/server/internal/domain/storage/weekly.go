package storage

import "github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"

// WeeklyArticle is the fixed public subset of canonical article metadata.
type WeeklyArticle struct {
	ArticleID       identity.ID `json:"article_id" nullable:"false"`
	JournalID       identity.ID `json:"journal_id" nullable:"false"`
	IssueID         *int64      `json:"issue_id" required:"false" nullable:"true" format:"int64"`
	Title           string      `json:"title" nullable:"false"`
	PublicationYear *int64      `json:"publication_year" required:"false" nullable:"true" format:"int64"`
	Date            *string     `json:"date" required:"false" nullable:"true"`
	DatePrecision   *string     `json:"date_precision" required:"false" nullable:"true" enum:"year,month,day"`
	Authors         []string    `json:"authors" nullable:"false"`
	Abstract        *string     `json:"abstract" required:"false" nullable:"true"`
	Doi             *string     `json:"doi" required:"false" nullable:"true"`
	JournalTitle    string      `json:"journal_title" nullable:"false"`
	OpenAccess      *bool       `json:"open_access" required:"false" nullable:"true"`
	InPress         *bool       `json:"in_press" required:"false" nullable:"true"`
	Volume          *string     `json:"volume" required:"false" nullable:"true"`
	Number          *string     `json:"number" required:"false" nullable:"true"`
}

// WeeklyJournalSummary supports bounded navigation without loading article bodies.
type WeeklyJournalSummary struct {
	JournalID       identity.ID `json:"journal_id" nullable:"false"`
	JournalTitle    *string     `json:"journal_title" required:"false" nullable:"true"`
	NewArticleCount int         `json:"new_article_count" nullable:"false" minimum:"0" format:"int64"`
}

// WeeklyJournalUpdate adds ordered article bodies only for the bounded legacy aggregate.
type WeeklyJournalUpdate struct {
	WeeklyJournalSummary
	Articles []WeeklyArticle `json:"articles" nullable:"false"`
}

// WeeklyDatabase keeps the original newest publication identity after combining membership.
type WeeklyDatabase[Journal any] struct {
	DBName          string    `json:"db_name" nullable:"false"`
	RunID           *string   `json:"run_id" required:"false" nullable:"true"`
	GeneratedAt     string    `json:"generated_at" nullable:"false"`
	NewArticleCount int       `json:"new_article_count" nullable:"false" minimum:"0" format:"int64"`
	Journals        []Journal `json:"journals" nullable:"false"`
}

// WeeklyResponse carries one fixed inclusive window across all database groups.
type WeeklyResponse[Journal any] struct {
	GeneratedAt string                    `json:"generated_at" nullable:"false"`
	WindowStart string                    `json:"window_start" nullable:"false"`
	WindowEnd   string                    `json:"window_end" nullable:"false"`
	Databases   []WeeklyDatabase[Journal] `json:"databases" nullable:"false"`
}
