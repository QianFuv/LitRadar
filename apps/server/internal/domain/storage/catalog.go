// Package storage defines persisted catalog and query values shared by storage consumers.
package storage

import "github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"

// ProviderCatalog describes a safe catalog without exposing filesystem paths.
type ProviderCatalog struct {
	Stem             string  `json:"stem" nullable:"false"`
	CsvFilename      *string `json:"csv_filename" required:"false" nullable:"true"`
	DatabaseFilename *string `json:"database_filename" required:"false" nullable:"true"`
}

// FavoriteCitation retains absent metadata separately from a known empty article title.
type FavoriteCitation struct {
	ArticleID    identity.ID `json:"article_id"`
	DBName       string      `json:"db_name"`
	Title        *string     `json:"title"`
	Authors      []string    `json:"authors"`
	JournalTitle *string     `json:"journal_title"`
	Date         *string     `json:"date"`
	Doi          *string     `json:"doi"`
}

// SearchMode selects escaped literal search or the original FTS5 expression grammar.
type SearchMode string

const (
	SearchSimple   SearchMode = "simple"
	SearchAdvanced SearchMode = "advanced"
)
