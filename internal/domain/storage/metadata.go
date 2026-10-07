package storage

import (
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"golang.org/x/text/unicode/norm"
)

// Journal retains canonical catalog identity, metadata and live article availability.
type Journal struct {
	JournalId    identity.Id `json:"journal_id"`
	CatalogId    string      `json:"catalog_id"`
	Title        string      `json:"title"`
	TitleAliases []string    `json:"title_aliases"`
	Issns        []string    `json:"issns"`
	Issn         *string     `json:"issn"`
	Eissn        *string     `json:"eissn"`
	Area         *string     `json:"area"`
	UtdRank      *string     `json:"utd_rank"`
	UtdRating    *string     `json:"utd_rating"`
	AbsRank      *string     `json:"abs_rank"`
	AbsRating    *string     `json:"abs_rating"`
	FmsRank      *string     `json:"fms_rank"`
	FmsRating    *string     `json:"fms_rating"`
	FmscnRank    *string     `json:"fmscn_rank"`
	FmscnRating  *string     `json:"fmscn_rating"`
	HasArticles  bool        `json:"has_articles"`
}

// Issue preserves absent fields and derives date precision without rewriting stored text.
type Issue struct {
	IssueId         int64       `json:"issue_id"`
	JournalId       identity.Id `json:"journal_id"`
	PublicationYear *int64      `json:"publication_year"`
	Title           *string     `json:"title"`
	Volume          *string     `json:"volume"`
	Number          *string     `json:"number"`
	Date            *string     `json:"date"`
	DatePrecision   *string     `json:"date_precision"`
}

// PageMeta keeps optional cursor and count fields explicitly present in JSON.
type PageMeta struct {
	Total      *int64  `json:"total"`
	Limit      int64   `json:"limit"`
	Offset     int64   `json:"offset"`
	NextCursor *string `json:"next_cursor"`
	HasMore    *bool   `json:"has_more"`
}

// Page contains ordered records and the pagination contract shared by content reads.
type Page[Record any] struct {
	Items []Record `json:"items"`
	Page  PageMeta `json:"page"`
}

// ValueCount counts stored labels without discarding whitespace-only values.
type ValueCount struct {
	Value string `json:"value"`
	Count int64  `json:"count"`
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
	UtdRating   []ValueCount `json:"utd_rating"`
	AbsRating   []ValueCount `json:"abs_rating"`
	FmsRating   []ValueCount `json:"fms_rating"`
	FmscnRating []ValueCount `json:"fmscn_rating"`
}

// YearSummary counts canonical issues and their distinct journals.
type YearSummary struct {
	Year         int64 `json:"year"`
	IssueCount   int64 `json:"issue_count"`
	JournalCount int64 `json:"journal_count"`
}

// JournalOption exposes the stable identifier and title used by selectors.
type JournalOption struct {
	JournalId identity.Id `json:"journal_id"`
	Title     string      `json:"title"`
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
