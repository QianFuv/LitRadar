// Package favorites maintains owner-scoped folders and immutable favorite references.
package favorites

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	sqlite3 "github.com/mattn/go-sqlite3"
)

// ErrDuplicateFolder preserves the uniqueness failure presented to folder owners.
var ErrDuplicateFolder = errors.New("Folder name already exists")

// ErrFolderNotFound hides absent folders and folders owned by another user.
var ErrFolderNotFound = errors.New("Folder not found")

// ErrSourceFolderNotFound identifies an unavailable source in a bulk move.
var ErrSourceFolderNotFound = errors.New("Source folder not found")

// ErrTargetFolderNotFound identifies an unavailable target in a bulk move.
var ErrTargetFolderNotFound = errors.New("Target folder not found")

// ErrSameFolders prevents a bulk move from deleting its own source entries.
var ErrSameFolders = errors.New("Source and target folders must be different")

// ErrCursor rejects malformed or owner-incompatible pagination anchors.
var ErrCursor = errors.New("Invalid favorite pagination cursor")

// InvalidInput retains the public business validation category.
type InvalidInput struct{ Message string }

// Error returns the public validation diagnostic.
func (failure InvalidInput) Error() string { return failure.Message }

// Folder retains numeric row identities and original fractional creation timestamps.
type Folder struct {
	Id           int64   `json:"id"`
	Name         string  `json:"name"`
	IsTracking   bool    `json:"is_tracking"`
	ArticleCount int64   `json:"article_count"`
	CreatedAt    float64 `json:"created_at"`
}

// Reference distinguishes equal article identifiers in separate source databases.
type Reference struct {
	ArticleId identity.Id `json:"article_id"`
	DbName    string      `json:"db_name"`
}

// Add supplies a reference and its initial note, which duplicate adds do not replace.
type Add struct {
	Reference
	Note string `json:"note"`
}

// Favorite is the persisted owner-scoped favorite response.
type Favorite struct {
	Id       int64 `json:"id"`
	FolderId int64 `json:"folder_id"`
	Reference
	Note      string  `json:"note"`
	CreatedAt float64 `json:"created_at"`
}

// Repository composes the existing auth pool without running migrations or owning its lifetime.
type Repository struct{ auth *auth.Repository }

// New shares the supplied auth repository without taking ownership of its pool.
func New(repository *auth.Repository) *Repository { return &Repository{repository} }

func positive(label string, value int64) error {
	if value <= 0 {
		return InvalidInput{label + " must be a positive integer"}
	}
	return nil
}
func characters(label, value string, maximum int) error {
	if utf8.RuneCountInString(value) > maximum {
		return InvalidInput{fmt.Sprintf("%s must be at most %d characters", label, maximum)}
	}
	return nil
}
func folderName(value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" || utf8.RuneCountInString(value) > 100 {
		return "", InvalidInput{"Folder name must be 1-100 characters"}
	}
	return value, nil
}
func validateReference(value Reference) error {
	if err := positive("article_id", int64(value.ArticleId)); err != nil {
		return err
	}
	return characters("db_name", value.DbName, 255)
}
func validateAdd(value Add) error {
	if err := validateReference(value.Reference); err != nil {
		return err
	}
	return characters("note", value.Note, 2000)
}
func nowSeconds() float64 { return float64(time.Now().UnixNano()) / 1e9 }
func constraint(err error) error {
	var failure sqlite3.Error
	if errors.As(err, &failure) && failure.Code == sqlite3.ErrConstraint {
		return ErrDuplicateFolder
	}
	return err
}

type scanner interface{ Scan(...any) error }

func scanFolder(row scanner) (Folder, error) {
	var id, tracking, count storage.Integer
	var name storage.Text
	var created storage.Number
	if err := row.Scan(&id, &name, &tracking, &created, &count); err != nil {
		return Folder{}, err
	}
	return Folder{int64(id), string(name), tracking != 0, int64(count), float64(created)}, nil
}
func scanFavorite(row scanner) (Favorite, error) {
	var id, folder, article storage.Integer
	var name, note storage.Text
	var created storage.Number
	if err := row.Scan(&id, &folder, &article, &name, &note, &created); err != nil {
		return Favorite{}, err
	}
	return Favorite{int64(id), int64(folder), Reference{identity.Id(article), string(name)}, string(note), float64(created)}, nil
}
func ensureFolder(ctx context.Context, connection *sql.Conn, owner identity.Id, folder int64, missing error) error {
	var id storage.Integer
	err := connection.QueryRowContext(ctx, "SELECT id FROM folders WHERE id=? AND user_id=?", folder, owner).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return missing
	}
	return err
}

// CreateFolder atomically changes the tracking selection only if the new folder is created.
func (repository *Repository) CreateFolder(ctx context.Context, owner identity.Id, name string, isTracking bool) (Folder, error) {
	name, err := folderName(name)
	if err != nil {
		return Folder{}, err
	}
	var folder Folder
	err = repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		now := nowSeconds()
		if isTracking {
			if _, err := connection.ExecContext(ctx, "UPDATE folders SET is_tracking=0,updated_at=? WHERE user_id=?", now, owner); err != nil {
				return err
			}
		}
		result, err := connection.ExecContext(ctx, "INSERT INTO folders(user_id,name,is_tracking,created_at,updated_at) VALUES(?,?,?,?,?)", owner, name, isTracking, now, now)
		if err != nil {
			return constraint(err)
		}
		id, err := result.LastInsertId()
		folder = Folder{id, name, isTracking, 0, now}
		return err
	})
	if err != nil {
		return Folder{}, err
	}
	return folder, nil
}

// ListFolders returns creation-ordered folders with current membership counts.
func (repository *Repository) ListFolders(ctx context.Context, owner identity.Id) ([]Folder, error) {
	result := []Folder{}
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		rows, err := connection.QueryContext(ctx, "SELECT f.id,f.name,f.is_tracking,f.created_at,COUNT(fav.id) FROM folders f LEFT JOIN favorites fav ON fav.folder_id=f.id WHERE f.user_id=? GROUP BY f.id ORDER BY f.created_at", owner)
		if err != nil {
			return err
		}
		defer rows.Close()
		for rows.Next() {
			item, err := scanFolder(rows)
			if err != nil {
				return err
			}
			result = append(result, item)
		}
		return rows.Err()
	})
	return result, err
}

// RenameFolder does not expose whether another owner's folder exists.
func (repository *Repository) RenameFolder(ctx context.Context, owner identity.Id, folder int64, name string) (bool, error) {
	name, err := folderName(name)
	if err != nil {
		return false, err
	}
	var changed bool
	err = repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		result, err := connection.ExecContext(ctx, "UPDATE folders SET name=?,updated_at=? WHERE id=? AND user_id=?", name, nowSeconds(), folder, owner)
		if err != nil {
			return constraint(err)
		}
		count, err := result.RowsAffected()
		changed = count > 0
		return err
	})
	return changed, err
}

// TrackingFolder returns the exact is_tracking=1 selection without counting its favorites.
func (repository *Repository) TrackingFolder(ctx context.Context, owner identity.Id) (*Folder, error) {
	var result *Folder
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		folder, err := scanFolder(connection.QueryRowContext(ctx, "SELECT id,name,is_tracking,created_at,0 FROM folders WHERE user_id=? AND is_tracking=1 LIMIT 1", owner))
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err == nil {
			result = &folder
		}
		return err
	})
	return result, err
}

// SetTrackingFolder retains the previous selection when the requested folder is not owned.
func (repository *Repository) SetTrackingFolder(ctx context.Context, owner identity.Id, folder int64) (bool, error) {
	if err := positive("folder_id", folder); err != nil {
		return false, err
	}
	changed := false
	ignored := errors.New("tracking update was ignored")
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := ensureFolder(ctx, connection, owner, folder, ErrFolderNotFound); err != nil {
			return err
		}
		now := nowSeconds()
		if _, err := connection.ExecContext(ctx, "UPDATE folders SET is_tracking=0,updated_at=? WHERE user_id=?", now, owner); err != nil {
			return err
		}
		result, err := connection.ExecContext(ctx, "UPDATE folders SET is_tracking=1,updated_at=? WHERE id=? AND user_id=?", now, folder, owner)
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		changed = count == 1
		if err == nil && !changed {
			return ignored
		}
		return err
	})
	if errors.Is(err, ErrFolderNotFound) || errors.Is(err, ignored) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return changed, nil
}

// DeleteFolder rechecks notification dependencies under the same write lock as deletion.
func (repository *Repository) DeleteFolder(ctx context.Context, owner identity.Id, folder int64) (bool, error) {
	if err := positive("folder_id", folder); err != nil {
		return false, err
	}
	changed := false
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		var tracking storage.Integer
		err := connection.QueryRowContext(ctx, "SELECT is_tracking FROM folders WHERE id=? AND user_id=?", folder, owner).Scan(&tracking)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return err
		}
		if tracking != 0 {
			var method storage.Text
			var token, sync storage.Integer
			err := connection.QueryRowContext(ctx, "SELECT delivery_method,pushplus_token<>'',sync_to_tracking_folder<>0 FROM notification_settings WHERE user_id=?", owner).Scan(&method, &token, &sync)
			if err != nil && !errors.Is(err, sql.ErrNoRows) {
				return err
			}
			if err == nil {
				switch strings.TrimSpace(string(method)) {
				case "folder":
					return InvalidInput{"A tracking folder is required when delivery_method is 'folder'"}
				case "pushplus":
					if token == 0 {
						return InvalidInput{"pushplus_token is required when delivery_method is 'pushplus'"}
					}
					if sync != 0 {
						return InvalidInput{"A tracking folder is required before enabling PushPlus sync to tracking"}
					}
				}
			}
		}
		result, err := connection.ExecContext(ctx, "DELETE FROM folders WHERE id=? AND user_id=?", folder, owner)
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		changed = count > 0
		return err
	})
	if err != nil {
		return false, err
	}
	return changed, nil
}

// AddFavorite preserves the first note and creation time on repeated insertion.
func (repository *Repository) AddFavorite(ctx context.Context, owner identity.Id, folder int64, value Add) (Favorite, error) {
	if err := positive("folder_id", folder); err != nil {
		return Favorite{}, err
	}
	if err := validateAdd(value); err != nil {
		return Favorite{}, err
	}
	var favorite Favorite
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := ensureFolder(ctx, connection, owner, folder, ErrFolderNotFound); err != nil {
			return err
		}
		var err error
		favorite, err = scanFavorite(connection.QueryRowContext(ctx, "INSERT INTO favorites(user_id,folder_id,article_id,db_name,note,created_at) VALUES(?,?,?,?,?,?) ON CONFLICT(user_id,folder_id,article_id,db_name) DO NOTHING RETURNING id,folder_id,article_id,db_name,note,created_at", owner, folder, value.ArticleId, value.DbName, value.Note, nowSeconds()))
		if errors.Is(err, sql.ErrNoRows) {
			favorite, err = scanFavorite(connection.QueryRowContext(ctx, "SELECT id,folder_id,article_id,db_name,note,created_at FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?", owner, folder, value.ArticleId, value.DbName))
		}
		return err
	})
	if err != nil {
		return Favorite{}, err
	}
	return favorite, nil
}

// RemoveFavorite deletes only the exact owner's database-qualified reference.
func (repository *Repository) RemoveFavorite(ctx context.Context, owner identity.Id, folder int64, value Reference) (bool, error) {
	if err := positive("folder_id", folder); err != nil {
		return false, err
	}
	if err := validateReference(value); err != nil {
		return false, err
	}
	changed := false
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		result, err := connection.ExecContext(ctx, "DELETE FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?", owner, folder, value.ArticleId, value.DbName)
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		changed = count > 0
		return err
	})
	return changed, err
}

func favoriteRows(ctx context.Context, connection *sql.Conn, statement string, values ...any) ([]Favorite, error) {
	rows, err := connection.QueryContext(ctx, statement, values...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []Favorite{}
	for rows.Next() {
		item, err := scanFavorite(rows)
		if err != nil {
			return nil, err
		}
		result = append(result, item)
	}
	return result, rows.Err()
}

// ListFavorites retains legacy raw SQLite limit/offset behavior and creation-time ordering.
func (repository *Repository) ListFavorites(ctx context.Context, owner identity.Id, folder *int64, limit, offset int64) ([]Favorite, error) {
	var result []Favorite
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		statement := "SELECT id,folder_id,article_id,db_name,note,created_at FROM favorites WHERE user_id=?"
		values := []any{owner}
		if folder != nil {
			statement += " AND folder_id=?"
			values = append(values, *folder)
		}
		statement += " ORDER BY created_at DESC LIMIT ? OFFSET ?"
		values = append(values, limit, offset)
		var err error
		result, err = favoriteRows(ctx, connection, statement, values...)
		return err
	})
	return result, err
}

// CountFavorites counts either all owned references or those in one folder.
func (repository *Repository) CountFavorites(ctx context.Context, owner identity.Id, folder *int64) (int64, error) {
	var count storage.Integer
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		statement := "SELECT COUNT(*) FROM favorites WHERE user_id=?"
		values := []any{owner}
		if folder != nil {
			statement += " AND folder_id=?"
			values = append(values, *folder)
		}
		return connection.QueryRowContext(ctx, statement, values...).Scan(&count)
	})
	return int64(count), err
}
