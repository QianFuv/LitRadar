package storage

import (
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"golang.org/x/text/unicode/norm"
)

// Journal retains canonical catalog identity, metadata and live article availability.
type Journal struct {
	JournalID    identity.ID `json:"journal_id" nullable:"false"`
	CatalogID    string      `json:"catalog_id" nullable:"false"`
	Title        string      `json:"title" nullable:"false"`
	TitleAliases []string    `json:"title_aliases" nullable:"false"`
	Issns        []string    `json:"issns" nullable:"false"`
	Issn         *string     `json:"issn" required:"false" nullable:"true"`
	Eissn        *string     `json:"eissn" required:"false" nullable:"true"`
	Area         *string     `json:"area" required:"false" nullable:"true"`
	UtdRank      *string     `json:"utd_rank" required:"false" nullable:"true"`
	UtdRating    *string     `json:"utd_rating" required:"false" nullable:"true"`
	AbsRank      *string     `json:"abs_rank" required:"false" nullable:"true"`
	AbsRating    *string     `json:"abs_rating" required:"false" nullable:"true"`
	FmsRank      *string     `json:"fms_rank" required:"false" nullable:"true"`
	FmsRating    *string     `json:"fms_rating" required:"false" nullable:"true"`
	FmscnRank    *string     `json:"fmscn_rank" required:"false" nullable:"true"`
	FmscnRating  *string     `json:"fmscn_rating" required:"false" nullable:"true"`
	HasArticles  bool        `json:"has_articles" nullable:"false"`
}

// Issue preserves absent fields and derives date precision without rewriting stored text.
type Issue struct {
	IssueID         int64       `json:"issue_id" nullable:"false" format:"int64"`
	JournalID       identity.ID `json:"journal_id" nullable:"false"`
	PublicationYear *int64      `json:"publication_year" required:"false" nullable:"true" format:"int64"`
	Title           *string     `json:"title" required:"false" nullable:"true"`
	Volume          *string     `json:"volume" required:"false" nullable:"true"`
	Number          *string     `json:"number" required:"false" nullable:"true"`
	Date            *string     `json:"date" required:"false" nullable:"true"`
	DatePrecision   *string     `json:"date_precision" required:"false" nullable:"true" enum:"year,month,day"`
}

// PageMeta keeps optional cursor and count fields explicitly present in JSON.
type PageMeta struct {
	Total      *int64  `json:"total" required:"false" nullable:"true" format:"int64"`
	Limit      int64   `json:"limit" nullable:"false" format:"int64"`
	Offset     int64   `json:"offset" nullable:"false" format:"int64"`
	NextCursor *string `json:"next_cursor" required:"false" nullable:"true"`
	HasMore    *bool   `json:"has_more" required:"false" nullable:"true"`
}

// Page contains ordered records and the pagination contract shared by content reads.
type Page[Record any] struct {
	Items []Record `json:"items" nullable:"false"`
	Page  PageMeta `json:"page" nullable:"false"`
}

// ValueCount counts stored labels without discarding whitespace-only values.
type ValueCount struct {
	Value string `json:"value" nullable:"false"`
	Count int64  `json:"count" nullable:"false" format:"int64"`
}

// JournalRatings intersects four independent systems of exact rating values.
type JournalRatings struct {
	UtdRating   []string `json:"utd_rating"`
	AbsRating   []string `json:"abs_rating"`
	FmsRating   []string `json:"fms_rating"`
	FmscnRating []string `json:"fmscn_rating"`
}

// RatingOptions includes journals with no indexed articles in every rating group.
type RatingOptions struct {
	UtdRating   []ValueCount `json:"utd_rating" nullable:"false"`
	AbsRating   []ValueCount `json:"abs_rating" nullable:"false"`
	FmsRating   []ValueCount `json:"fms_rating" nullable:"false"`
	FmscnRating []ValueCount `json:"fmscn_rating" nullable:"false"`
}

// YearSummary counts canonical issues and their distinct journals.
type YearSummary struct {
	Year         int64 `json:"year" nullable:"false" format:"int64"`
	IssueCount   int64 `json:"issue_count" nullable:"false" format:"int64"`
	JournalCount int64 `json:"journal_count" nullable:"false" format:"int64"`
}

// JournalOption exposes the stable identifier and title used by selectors.
type JournalOption struct {
	JournalID identity.ID `json:"journal_id" nullable:"false"`
	Title     string      `json:"title" nullable:"false"`
}

// DatePrecision validates the complete partial-date grammar including year zero and real calendar dates.
func DatePrecision(value string) *string {
	value = strings.TrimSpace(norm.NFC.String(value))
	if !hasPartialDateGrammar(value) {
		return nil
	}
	year, _ := strconv.Atoi(value[:4])
	month, day := 1, 1
	precision := "year"
	if len(value) >= 7 {
		month, _ = strconv.Atoi(value[5:7])
		precision = "month"
	}
	if len(value) == 10 {
		day, _ = strconv.Atoi(value[8:])
		precision = "day"
	}
	date := time.Date(year, time.Month(month), day, 0, 0, 0, 0, time.UTC)
	if date.Year() != year || int(date.Month()) != month || date.Day() != day {
		return nil
	}
	return &precision
}

// hasPartialDateGrammar requires the complete fixed-width ASCII year/month/day shape.
func hasPartialDateGrammar(value string) bool {
	if len(value) != 4 && len(value) != 7 && len(value) != 10 {
		return false
	}
	for index := range len(value) {
		if index == 4 || index == 7 {
			if value[index] != '-' {
				return false
			}
		} else if value[index] < '0' || value[index] > '9' {
			return false
		}
	}
	return true
}
