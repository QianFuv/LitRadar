// Package announcements maintains public messages with transaction-local administrator checks and audits.
package announcements

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"time"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/announcementsdb"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// Announcement preserves numeric identifiers and fractional epoch timestamps.
type Announcement struct {
	Id        int64   `json:"id" nullable:"false" format:"int64"`
	Title     string  `json:"title" nullable:"false"`
	Message   string  `json:"message" nullable:"false"`
	Priority  string  `json:"priority" nullable:"false"`
	Enabled   bool    `json:"enabled" nullable:"false"`
	CreatedAt float64 `json:"created_at" nullable:"false" format:"double"`
	UpdatedAt float64 `json:"updated_at" nullable:"false" format:"double"`
}

// InvalidInput reports normalized announcement field constraints.
type InvalidInput struct{ Message string }

// Error returns the public announcement validation diagnostic.
func (failure InvalidInput) Error() string { return failure.Message }

// Update contains only explicitly supplied replacement fields.
type Update struct {
	Title, Message, Priority *string
	Enabled                  *bool
}

func normalize(input Update) (Update, error) {
	for _, field := range []struct {
		label   string
		value   **string
		maximum int
	}{{"Title", &input.Title, 200}, {"Message", &input.Message, 10000}, {"Priority", &input.Priority, 16}} {
		if *field.value == nil {
			continue
		}
		value := strings.TrimSpace(**field.value)
		if field.label == "Priority" {
			value = strings.Map(func(character rune) rune {
				if character >= 'A' && character <= 'Z' {
					return character + 32
				}
				return character
			}, value)
		}
		if utf8.RuneCountInString(value) > field.maximum {
			return Update{}, InvalidInput{fmt.Sprintf("%s must be at most %d characters", field.label, field.maximum)}
		}
		if value == "" {
			return Update{}, InvalidInput{fmt.Sprintf("%s must be 1-%d characters", field.label, field.maximum)}
		}
		if field.label == "Priority" && value != "high" && value != "normal" && value != "low" {
			return Update{}, InvalidInput{"Priority must be high, normal, or low"}
		}
		*field.value = &value
	}
	return input, nil
}

var queries announcementsdb.Queries

func decode(row announcementsdb.Announcement) Announcement {
	return Announcement{int64(row.ID), string(row.Title), string(row.Message), string(row.Priority), row.Enabled != 0, float64(row.CreatedAt), float64(row.UpdatedAt)}
}

func decodeList(rows []announcementsdb.Announcement, err error) ([]Announcement, error) {
	if err != nil {
		return nil, err
	}
	items := make([]Announcement, 0, len(rows))
	for _, row := range rows {
		items = append(items, decode(row))
	}
	return items, nil
}

func enabledValue(isEnabled bool) storage.Integer {
	if isEnabled {
		return 1
	}
	return 0
}

// ListActive returns only enabled=1 rows, ordered by priority and creation time without an added tie-breaker.
func ListActive(ctx context.Context, filename string) ([]Announcement, error) {
	database, err := storage.OpenPlain(filename)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer connection.Close()
	return decodeList(queries.Active(ctx, connection))
}

// ListAll returns administrative rows in creation-time order.
func ListAll(ctx context.Context, repository *auth.Repository) ([]Announcement, error) {
	var items []Announcement
	err := repository.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		items, err = decodeList(queries.All(ctx, connection))
		return err
	})
	return items, err
}

func get(ctx context.Context, connection *sql.Conn, id int64) (*Announcement, error) {
	row, err := queries.ByID(ctx, connection, storage.Integer(id))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	item := decode(row)
	return &item, nil
}

// Get distinguishes an absent announcement from an operational failure.
func Get(ctx context.Context, repository *auth.Repository, id int64) (*Announcement, error) {
	var result *Announcement
	err := repository.WithConnection(ctx, func(connection *sql.Conn) error { var err error; result, err = get(ctx, connection, id); return err })
	return result, err
}

func authorize(ctx context.Context, connection *sql.Conn, actor *identity.Id) error {
	if actor != nil {
		return auth.RequireAdministrator(ctx, connection, *actor)
	}
	return nil
}

func auditTarget(ctx context.Context, connection *sql.Conn, audit *domain.AuditEvent, id int64) error {
	if audit == nil {
		return nil
	}
	event := *audit
	event.TargetId = &id
	return auth.InsertAudit(ctx, connection, &event)
}

// Create atomically persists a validated message and its required completion audit after rechecking the actor.
func Create(ctx context.Context, repository *auth.Repository, actor *identity.Id, title, message, priority string, isEnabled bool, audit *domain.AuditEvent) (Announcement, error) {
	input, err := normalize(Update{Title: &title, Message: &message, Priority: &priority})
	if err != nil {
		return Announcement{}, err
	}
	var result Announcement
	err = repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		now := float64(time.Now().UnixNano()) / 1e9
		inserted, err := queries.Create(ctx, connection, announcementsdb.CreateParams{Title: storage.Text(*input.Title), Message: storage.Text(*input.Message), Priority: storage.Text(*input.Priority), Enabled: enabledValue(isEnabled), CreatedAt: storage.Number(now), UpdatedAt: storage.Number(now)})
		if err != nil {
			return err
		}
		id, err := inserted.LastInsertId()
		if err != nil {
			return err
		}
		item, err := get(ctx, connection, id)
		if err != nil {
			return err
		}
		if item == nil {
			return sql.ErrNoRows
		}
		result = *item
		return auditTarget(ctx, connection, audit, id)
	})
	if err != nil {
		return Announcement{}, err
	}
	return result, nil
}

// Modify applies supplied fields and audits only an existing row in the same write transaction.
func Modify(ctx context.Context, repository *auth.Repository, actor *identity.Id, id int64, input Update, audit *domain.AuditEvent) (*Announcement, error) {
	input, err := normalize(input)
	if err != nil {
		return nil, err
	}
	var result *Announcement
	err = repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		current, err := get(ctx, connection, id)
		if err != nil || current == nil {
			return err
		}
		if input.Title != nil {
			current.Title = *input.Title
		}
		if input.Message != nil {
			current.Message = *input.Message
		}
		if input.Priority != nil {
			current.Priority = *input.Priority
		}
		if input.Enabled != nil {
			current.Enabled = *input.Enabled
		}
		if _, err := queries.Update(ctx, connection, announcementsdb.UpdateParams{Title: storage.Text(current.Title), Message: storage.Text(current.Message), Priority: storage.Text(current.Priority), Enabled: enabledValue(current.Enabled), UpdatedAt: storage.Number(float64(time.Now().UnixNano()) / 1e9), ID: storage.Integer(id)}); err != nil {
			return err
		}
		result, err = get(ctx, connection, id)
		if err != nil {
			return err
		}
		if result != nil {
			return auditTarget(ctx, connection, audit, id)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}

// Delete rechecks the actor and rolls back deletion when its required audit cannot persist.
func Delete(ctx context.Context, repository *auth.Repository, actor *identity.Id, id int64, audit *domain.AuditEvent) (bool, error) {
	wasDeleted := false
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		result, err := queries.Delete(ctx, connection, storage.Integer(id))
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		wasDeleted = count > 0
		if wasDeleted {
			return auditTarget(ctx, connection, audit, id)
		}
		return nil
	})
	if err != nil {
		return false, err
	}
	return wasDeleted, nil
}
