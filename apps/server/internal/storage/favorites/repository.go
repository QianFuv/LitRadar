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

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/favoritesdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	sqlite3 "modernc.org/sqlite/lib"
)

var queries favoritesdb.Queries

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
	Id           int64   `json:"id" nullable:"false" format:"int64"`
	Name         string  `json:"name" nullable:"false"`
	IsTracking   bool    `json:"is_tracking" nullable:"false"`
	ArticleCount int64   `json:"article_count" nullable:"false" format:"int64"`
	CreatedAt    float64 `json:"created_at" nullable:"false" format:"double"`
}

// Reference distinguishes equal article identifiers in separate source databases.
type Reference struct {
	ArticleId identity.Id `json:"article_id" nullable:"false"`
	DbName    string      `json:"db_name" nullable:"false"`
}

// Add supplies a reference and its initial note, which duplicate adds do not replace.
type Add struct {
	Reference
	Note string `json:"note"`
}

// Favorite is the persisted owner-scoped favorite response.
type Favorite struct {
	Id       int64 `json:"id" nullable:"false" format:"int64"`
	FolderId int64 `json:"folder_id" nullable:"false" format:"int64"`
	Reference
	Note      string  `json:"note" nullable:"false"`
	CreatedAt float64 `json:"created_at" nullable:"false" format:"double"`
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
	var failure interface{ Code() int }
	if errors.As(err, &failure) && failure.Code()&255 == sqlite3.SQLITE_CONSTRAINT {
		return ErrDuplicateFolder
	}
	return err
}

type scanner interface{ Scan(...any) error }

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
	_, err := queries.FolderID(ctx, connection, favoritesdb.FolderIDParams{ID: storage.Integer(folder), UserID: storage.Integer(owner)})
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
			if _, err := queries.ClearTracking(ctx, connection, favoritesdb.ClearTrackingParams{UpdatedAt: storage.Number(now), UserID: storage.Integer(owner)}); err != nil {
				return err
			}
		}
		result, err := queries.CreateFolder(ctx, connection, favoritesdb.CreateFolderParams{UserID: storage.Integer(owner), Name: storage.Text(name), IsTracking: storage.Integer(dbtypes.Boolean(isTracking)), CreatedAt: storage.Number(now), UpdatedAt: storage.Number(now)})
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
		rows, err := queries.Folders(ctx, connection, favoritesdb.FoldersParams{UserID: storage.Integer(owner)})
		if err != nil {
			return err
		}
		for _, row := range rows {
			result = append(result, Folder{int64(row.ID), string(row.Name), row.IsTracking != 0, row.Count, float64(row.CreatedAt)})
		}
		return nil
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
		result, err := queries.RenameFolder(ctx, connection, favoritesdb.RenameFolderParams{Name: storage.Text(name), UpdatedAt: storage.Number(nowSeconds()), ID: storage.Integer(folder), UserID: storage.Integer(owner)})
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
		row, err := queries.TrackingFolder(ctx, connection, favoritesdb.TrackingFolderParams{UserID: storage.Integer(owner)})
		folder := Folder{int64(row.ID), string(row.Name), row.IsTracking != 0, row.Column5, float64(row.CreatedAt)}
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
		if _, err := queries.ClearTracking(ctx, connection, favoritesdb.ClearTrackingParams{UpdatedAt: storage.Number(now), UserID: storage.Integer(owner)}); err != nil {
			return err
		}
		result, err := queries.SetTrackingFolder(ctx, connection, favoritesdb.SetTrackingFolderParams{UpdatedAt: storage.Number(now), ID: storage.Integer(folder), UserID: storage.Integer(owner)})
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
		err := func() error {
			queryResult, err := queries.FolderTracking(ctx, connection, favoritesdb.FolderTrackingParams{ID: storage.Integer(folder), UserID: storage.Integer(owner)})
			if err != nil {
				return err
			}
			tracking = storage.Integer(queryResult)
			return nil
		}()
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return err
		}
		if tracking != 0 {
			var method storage.Text
			var token, sync storage.Integer
			err := func() error {
				queryResult, err := queries.NotificationDependency(ctx, connection, favoritesdb.NotificationDependencyParams{UserID: storage.Integer(owner)})
				if err != nil {
					return err
				}
				method = storage.Text(queryResult.DeliveryMethod)
				token = storage.Integer(dbtypes.Boolean(queryResult.Column2))
				sync = storage.Integer(dbtypes.Boolean(queryResult.Column3))
				return nil
			}()
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
		result, err := queries.DeleteFolder(ctx, connection, favoritesdb.DeleteFolderParams{ID: storage.Integer(folder), UserID: storage.Integer(owner)})
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
		favorite, err = favoriteInsert(ctx, connection, owner, folder, value)
		if errors.Is(err, sql.ErrNoRows) {
			favorite, err = favoriteLookup(ctx, connection, owner, folder, value.Reference)
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
		result, err := queries.RemoveFavorite(ctx, connection, favoritesdb.RemoveFavoriteParams{UserID: storage.Integer(owner), FolderID: storage.Integer(folder), ArticleID: storage.Integer(value.ArticleId), DBName: storage.Text(value.DbName)})
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

func favoriteInsert(ctx context.Context, connection *sql.Conn, owner identity.Id, folder int64, item Add) (Favorite, error) {
	row, err := queries.AddFavorite(ctx, connection, favoritesdb.AddFavoriteParams{UserID: storage.Integer(owner), FolderID: storage.Integer(folder), ArticleID: storage.Integer(item.ArticleId), DBName: storage.Text(item.DbName), Note: storage.Text(item.Note), CreatedAt: storage.Number(nowSeconds())})
	return decodeFavorite(favoritesdb.FavoriteRow(row)), err
}

func favoriteLookup(ctx context.Context, connection *sql.Conn, owner identity.Id, folder int64, item Reference) (Favorite, error) {
	row, err := queries.Favorite(ctx, connection, favoritesdb.FavoriteParams{UserID: storage.Integer(owner), FolderID: storage.Integer(folder), ArticleID: storage.Integer(item.ArticleId), DBName: storage.Text(item.DbName)})
	return decodeFavorite(row), err
}

func decodeFavorite(row favoritesdb.FavoriteRow) Favorite {
	return Favorite{int64(row.ID), int64(row.FolderID), Reference{identity.Id(row.ArticleID), string(row.DBName)}, string(row.Note), float64(row.CreatedAt)}
}
