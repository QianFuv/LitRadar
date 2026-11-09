package storage

import "log/slog"

// ArticleCandidate supplies notification ranking without changing canonical numeric identifiers.
type ArticleCandidate struct {
	ArticleId    int64   `json:"article_id"`
	JournalId    int64   `json:"journal_id"`
	IssueId      *int64  `json:"issue_id"`
	Title        string  `json:"title"`
	Abstract     string  `json:"abstract_text"`
	Date         *string `json:"date"`
	JournalTitle string  `json:"journal_title"`
	Doi          *string `json:"doi"`
	OpenAccess   bool    `json:"open_access"`
	InPress      bool    `json:"in_press"`
}

// String redacts article content from ordinary formatted diagnostics.
func (candidate ArticleCandidate) String() string { return "ArticleCandidate([REDACTED])" }

// GoString also redacts content from Go-syntax diagnostic formatting.
func (candidate ArticleCandidate) GoString() string { return candidate.String() }

// LogValue prevents structured logs from expanding candidate fields.
func (candidate ArticleCandidate) LogValue() slog.Value { return slog.StringValue(candidate.String()) }
