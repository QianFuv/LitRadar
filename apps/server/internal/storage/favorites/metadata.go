package favorites

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"maps"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// Article distinguishes missing content from temporarily unreadable metadata without losing the favorite.
type Article struct {
	Favorite
	MetadataStatus  string       `json:"metadata_status" nullable:"false" enum:"available,missing,unavailable"`
	JournalID       *identity.ID `json:"journal_id" required:"false" nullable:"true"`
	IssueID         *identity.ID `json:"issue_id" required:"false" nullable:"true" pattern:"^[1-9][0-9]*$"`
	Title           *string      `json:"title" required:"false" nullable:"true"`
	PublicationYear *int64       `json:"publication_year" required:"false" nullable:"true" format:"int64"`
	Date            *string      `json:"date" required:"false" nullable:"true"`
	Authors         *[]string    `json:"authors" required:"false" nullable:"true"`
	Abstract        *string      `json:"abstract" required:"false" nullable:"true"`
	Doi             *string      `json:"doi" required:"false" nullable:"true"`
	JournalTitle    *string      `json:"journal_title" required:"false" nullable:"true"`
	OpenAccess      *bool        `json:"open_access" required:"false" nullable:"true"`
	InPress         *bool        `json:"in_press" required:"false" nullable:"true"`
	Volume          *string      `json:"volume" required:"false" nullable:"true"`
	Number          *string      `json:"number" required:"false" nullable:"true"`
	Issn            *string      `json:"issn" required:"false" nullable:"true"`
	Eissn           *string      `json:"eissn" required:"false" nullable:"true"`
}

func resolve(configuration config.Config, name string) (string, error) {
	var selected *string
	if name != "" {
		selected = &name
	}
	return configuration.ResolveIndexDBPath(selected)
}
func isMissing(err error) bool {
	return errors.Is(err, config.ErrNoDatabases) || errors.Is(err, config.ErrNotFound) || errors.Is(err, config.ErrInvalidName)
}

// safeName redacts unsafe database labels while preserving accepted byte spelling.
func safeName(name string) string {
	name = strings.TrimSpace(name)
	if name == "" {
		return "default"
	}
	if len(name) > 255 {
		return "invalid"
	}
	for index, character := range []byte(name) {
		if !isSafeMetadataNameByte(character, index) {
			return "invalid"
		}
	}
	return name
}
func boolValue(value *int64) *bool {
	if value == nil {
		return nil
	}
	result := *value != 0
	return &result
}
func queryIds(ids []int64) (string, []any) {
	values := []any{}
	seen := map[int64]bool{}
	for _, id := range ids {
		if id > 0 && !seen[id] {
			values = append(values, id)
			seen[id] = true
		}
	}
	return strings.TrimSuffix(strings.Repeat("?,", len(values)), ","), values
}

// Enrich isolates lookup failure to the entire original database-name group and never logs sensitive paths.
func Enrich(ctx context.Context, configuration config.Config, favorites []Favorite) []Article {
	groups := map[string][]int64{}
	for _, favorite := range favorites {
		groups[favorite.DBName] = append(groups[favorite.DBName], int64(favorite.ArticleID))
	}
	var metadata map[Reference]Article
	unavailable := map[string]bool{}
	for name, ids := range groups {
		items, category := favoriteMetadataGroup(ctx, configuration, name, ids)
		if category == "" {
			if metadata == nil {
				metadata = items
				continue
			}
			maps.Copy(metadata, items)
			continue
		}
		unavailable[name] = true
		slog.WarnContext(ctx, "Favorite metadata lookup is unavailable", "event", "favorites.metadata_unavailable", "database", safeName(name), "error_category", category)
	}
	return projectFavoriteMetadata(favorites, metadata, unavailable)
}

// loadMetadata admits all 500-ID chunks atomically while retaining one plain database owner.
func loadMetadata(ctx context.Context, filename, name string, ids []int64) (resultValue1 map[Reference]Article, resultError error) {
	_, values := queryIds(ids)
	result := map[Reference]Article{}
	if len(values) == 0 {
		return result, nil
	}
	database, err := storage.OpenPlain(filename)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	for start := 0; start < len(values); start += 500 {
		chunk := values[start:min(start+500, len(values))]
		if err := loadMetadataChunk(ctx, database, name, chunk, result); err != nil {
			return nil, err
		}
	}
	return result, nil
}

// LoadCitationRecords preserves input order and duplicates, returning operational errors instead of partial exports.
func LoadCitationRecords(ctx context.Context, configuration config.Config, references []Reference) ([]domain.FavoriteCitation, error) {
	groups := map[string][]int64{}
	for _, reference := range references {
		groups[reference.DBName] = append(groups[reference.DBName], int64(reference.ArticleID))
	}
	var metadata map[Reference]domain.FavoriteCitation
	for name, ids := range groups {
		filename, err := resolve(configuration, name)
		if isMissing(err) || errors.Is(err, config.ErrMultipleDatabases) {
			continue
		}
		if err != nil {
			return nil, err
		}
		items, err := loadCitations(ctx, filename, name, ids)
		if err != nil {
			return nil, err
		}
		if metadata == nil {
			metadata = items
			continue
		}
		maps.Copy(metadata, items)
	}
	result := make([]domain.FavoriteCitation, 0, len(references))
	for _, reference := range references {
		item, exists := metadata[reference]
		if !exists {
			item = domain.FavoriteCitation{ArticleID: reference.ArticleID, DBName: reference.DBName, Authors: []string{}}
		}
		result = append(result, item)
	}
	return result, nil
}

func loadCitations(ctx context.Context, filename, name string, ids []int64) (resultValue1 map[Reference]domain.FavoriteCitation, resultError error) {
	marks, values := queryIds(ids)
	result := map[Reference]domain.FavoriteCitation{}
	if len(values) == 0 {
		return result, nil
	}
	database, err := storage.OpenPlain(filename)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	rows, err := database.QueryContext(ctx, "SELECT a.article_id,a.title,a.authors_json,j.title,a.date,a.doi FROM articles a JOIN journals j ON j.journal_id=a.journal_id WHERE a.article_id IN ("+marks+")", values...)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := rows.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close rows: %w", closeError))
		}
	}()
	for rows.Next() {
		var id storage.Integer
		var title, journal, date, doi storage.OptionalText
		var authors storage.Text
		if err := rows.Scan(&id, &title, &authors, &journal, &date, &doi); err != nil {
			return nil, err
		}
		names, err := search.DecodeAuthorNames(string(authors))
		if err != nil {
			return nil, err
		}
		result[Reference{identity.ID(id), name}] = domain.FavoriteCitation{ArticleID: identity.ID(id), DBName: name, Title: title.Value, Authors: names, JournalTitle: journal.Value, Date: date.Value, Doi: doi.Value}
	}
	return result, rows.Err()
}

// isSafeMetadataNameByte preserves the byte-only logging label grammar.
func isSafeMetadataNameByte(character byte, index int) bool {
	return character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || index > 0 && (character == '.' || character == '_' || character == '-')
}

// favoriteMetadataGroup admits metadata only after the whole original name group succeeds.
func favoriteMetadataGroup(ctx context.Context, configuration config.Config, name string, ids []int64) (map[Reference]Article, string) {
	filename, err := resolve(configuration, name)
	if isMissing(err) {
		return nil, ""
	}
	category := "filesystem"
	if errors.Is(err, config.ErrMultipleDatabases) {
		category = "ambiguous_database"
	}
	if err == nil {
		var items map[Reference]Article
		items, err = loadMetadata(ctx, filename, name, ids)
		if err == nil {
			return items, ""
		}
		category = "sqlite"
		if errors.Is(err, search.ErrInvalidAuthors) {
			category = "json"
		}
	}
	return nil, category
}

// projectFavoriteMetadata preserves request order, duplicate pointer sharing and missing/unavailable status.
func projectFavoriteMetadata(favorites []Favorite, metadata map[Reference]Article, unavailable map[string]bool) []Article {
	result := make([]Article, 0, len(favorites))
	for _, favorite := range favorites {
		item, exists := metadata[favorite.Reference]
		if !exists {
			item.MetadataStatus = "missing"
			if unavailable[favorite.DBName] {
				item.MetadataStatus = "unavailable"
			}
		}
		item.Favorite = favorite
		result = append(result, item)
	}
	return result
}

// loadMetadataChunk closes every row owner explicitly while retaining whole-group failure admission.
func loadMetadataChunk(ctx context.Context, database *sql.DB, name string, chunk []any, result map[Reference]Article) error {
	marks := strings.TrimSuffix(strings.Repeat("?,", len(chunk)), ",")
	rows, err := database.QueryContext(ctx, `SELECT a.article_id,a.journal_id,a.issue_id,a.title,a.publication_year,a.date,a.authors_json,a.abstract_text,a.doi,a.open_access,a.in_press,j.title,j.issn,j.eissn,i.volume,i.number FROM articles a LEFT JOIN issues i ON i.issue_id=a.issue_id JOIN journals j ON j.journal_id=a.journal_id WHERE a.article_id IN (`+marks+")", chunk...)
	if err != nil {
		return err
	}
	for rows.Next() {
		id, item, err := metadataArticleFromRow(rows)
		if err != nil {
			if err := rows.Close(); err != nil {
				slog.Warn("cleanup.failed", "operation", "rows.Close", "error", err)
			}
			return err
		}
		result[Reference{id, name}] = item
	}
	err = rows.Err()
	if err := rows.Close(); err != nil {
		slog.Warn("cleanup.failed", "operation", "rows.Close", "error", err)
	}
	return err
}

// metadataArticleFromRow preserves strict whole-row scan before authors and row-local optional pointers.
func metadataArticleFromRow(row scanner) (identity.ID, Article, error) {
	var id storage.Integer
	var journal, issue, year, openAccess, inPress storage.OptionalInteger
	var title, date, abstract, doi, journalTitle, issn, eissn, volume, number storage.OptionalText
	var authors storage.Text
	if err := row.Scan(&id, &journal, &issue, &title, &year, &date, &authors, &abstract, &doi, &openAccess, &inPress, &journalTitle, &issn, &eissn, &volume, &number); err != nil {
		return 0, Article{}, err
	}
	names, err := search.DecodeAuthorNames(string(authors))
	if err != nil {
		return 0, Article{}, err
	}
	item := Article{MetadataStatus: "available", IssueID: (*identity.ID)(issue.Value), Title: title.Value, PublicationYear: year.Value, Date: date.Value, Authors: &names, Abstract: abstract.Value, Doi: doi.Value, JournalTitle: journalTitle.Value, OpenAccess: boolValue(openAccess.Value), InPress: boolValue(inPress.Value), Volume: volume.Value, Number: number.Value, Issn: issn.Value, Eissn: eissn.Value}
	if journal.Value != nil {
		value := identity.ID(*journal.Value)
		item.JournalID = &value
	}
	return identity.ID(id), item, nil
}
