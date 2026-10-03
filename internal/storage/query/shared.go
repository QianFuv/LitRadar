// Package query reads provider-neutral canonical content using the original filter and pagination contracts.
package query

import (
	"context"
	"database/sql"
	"fmt"
	"slices"
	"strings"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// InvalidInput distinguishes public filter validation failures from operational storage errors.
type InvalidInput struct{ Message string }

// Error returns the public query validation diagnostic.
func (failure InvalidInput) Error() string { return failure.Message }

// UnsupportedSortField identifies a case-sensitive field outside the operation's allowlist.
type UnsupportedSortField struct{ Field string }

// Error identifies the unsupported public sort field.
func (failure UnsupportedSortField) Error() string { return "Unsupported sort field: " + failure.Field }

// NotFound identifies absent canonical records after the database was successfully opened.
type NotFound struct{ Message string }

// Error returns the missing-record diagnostic after successful database selection.
func (failure NotFound) Error() string { return failure.Message }

func open(configuration config.Config, name *string) (*sql.DB, error) {
	filename, err := configuration.ResolveIndexDbPath(name)
	if err != nil {
		return nil, err
	}
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return nil, err
	}
	return database, nil
}

func validatePagination(limit, offset int64) error {
	if limit < 1 || limit > 200 {
		return InvalidInput{"limit must be between 1 and 200"}
	}
	if offset < 0 {
		return InvalidInput{"offset must be greater than or equal to 0"}
	}
	return nil
}

func orderBy(sort *string, fallback, alias string, allowed ...string) (string, error) {
	input := fallback
	if sort != nil {
		input = *sort
	}
	specs := []string{}
	for _, part := range strings.Split(input, ",") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		field, direction := part, "ASC"
		if strings.HasPrefix(part, "-") {
			field, direction = strings.TrimSpace(part[1:]), "DESC"
		} else if before, after, ok := strings.Cut(part, ":"); ok {
			field = strings.TrimSpace(before)
			if asciiLower(strings.TrimSpace(after)) == "desc" {
				direction = "DESC"
			}
		}
		if !slices.Contains(allowed, field) {
			return "", UnsupportedSortField{field}
		}
		specs = append(specs, alias+"."+field+" "+direction)
	}
	if len(specs) == 0 {
		return "", nil
	}
	return "ORDER BY " + strings.Join(specs, ", "), nil
}

func asciiLower(value string) string {
	return strings.Map(func(character rune) rune {
		if character >= 'A' && character <= 'Z' {
			return character + ('a' - 'A')
		}
		return character
	}, value)
}

type filter struct {
	clauses []string
	values  []any
}

func (filter *filter) add(clause string, values ...any) {
	filter.clauses = append(filter.clauses, clause)
	filter.values = append(filter.values, values...)
}
func (filter *filter) where() string {
	if len(filter.clauses) == 0 {
		return ""
	}
	return "WHERE " + strings.Join(filter.clauses, " AND ")
}

func ratingGroups(ratings domain.JournalRatings) []struct {
	field  string
	values []string
} {
	return []struct {
		field  string
		values []string
	}{{"utd_rating", ratings.UtdRating}, {"abs_rating", ratings.AbsRating}, {"fms_rating", ratings.FmsRating}, {"fmscn_rating", ratings.FmscnRating}}
}

func (filter *filter) ratings(ratings domain.JournalRatings) error {
	for _, group := range ratingGroups(ratings) {
		values := make([]string, 0, len(group.values))
		for _, value := range group.values {
			if utf8.RuneCountInString(value) > 2048 {
				return InvalidInput{fmt.Sprintf("%s must be at most 2048 characters", group.field)}
			}
			value = strings.TrimSpace(value)
			if value == "" {
				return InvalidInput{fmt.Sprintf("%s must be 1-2048 characters", group.field)}
			}
			values = append(values, value)
		}
		slices.Sort(values)
		values = slices.Compact(values)
		if len(values) == 0 {
			continue
		}
		arguments := make([]any, len(values))
		for index, value := range values {
			arguments[index] = value
		}
		filter.add("j."+group.field+" IN ("+strings.TrimSuffix(strings.Repeat("?,", len(values)), ",")+")", arguments...)
	}
	return nil
}

type scanner interface{ Scan(...any) error }

type rowQuerier interface {
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
}

func collect[Record any](ctx context.Context, database rowQuerier, statement string, values []any, decode func(scanner) (Record, error)) ([]Record, error) {
	rows, err := database.QueryContext(ctx, statement, values...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []Record{}
	for rows.Next() {
		record, err := decode(rows)
		if err != nil {
			return nil, err
		}
		items = append(items, record)
	}
	return items, rows.Err()
}
