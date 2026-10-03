package favorites

import (
	"context"
	"database/sql"
	"fmt"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func itemCount(label string, count int) error {
	if count > 500 {
		return InvalidInput{fmt.Sprintf("%s must contain at most 500 items", label)}
	}
	return nil
}
func uniqueReferences(items []Reference) []Reference {
	result := []Reference{}
	seen := map[Reference]bool{}
	for _, item := range items {
		if item.ArticleId > 0 && !seen[item] {
			seen[item] = true
			result = append(result, item)
		}
	}
	return result
}

// BulkAdd validates the entire submitted batch before atomically inserting new references.
func (repository *Repository) BulkAdd(ctx context.Context, owner identity.Id, folder int64, items []Add) (int64, error) {
	if err := positive("folder_id", folder); err != nil {
		return 0, err
	}
	if err := itemCount("articles", len(items)); err != nil {
		return 0, err
	}
	for _, item := range items {
		if err := validateAdd(item); err != nil {
			return 0, err
		}
	}
	var count int64
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := ensureFolder(ctx, connection, owner, folder, ErrFolderNotFound); err != nil {
			return err
		}
		statement, err := connection.PrepareContext(ctx, "INSERT OR IGNORE INTO favorites(user_id,folder_id,article_id,db_name,note,created_at) VALUES(?,?,?,?,?,?)")
		if err != nil {
			return err
		}
		defer statement.Close()
		now := nowSeconds()
		for _, item := range items {
			result, err := statement.ExecContext(ctx, owner, folder, item.ArticleId, item.DbName, item.Note, now)
			if err != nil {
				return err
			}
			added, err := result.RowsAffected()
			if err != nil {
				return err
			}
			count += added
		}
		return nil
	})
	if err != nil {
		return 0, err
	}
	return count, nil
}

// BulkRemove treats an empty validated batch as a no-op before opening the repository.
func (repository *Repository) BulkRemove(ctx context.Context, owner identity.Id, folder int64, items []Reference) (int64, error) {
	if err := positive("folder_id", folder); err != nil {
		return 0, err
	}
	if err := itemCount("articles", len(items)); err != nil {
		return 0, err
	}
	for _, item := range items {
		if err := validateReference(item); err != nil {
			return 0, err
		}
	}
	items = uniqueReferences(items)
	if len(items) == 0 {
		return 0, nil
	}
	var count int64
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := ensureFolder(ctx, connection, owner, folder, ErrFolderNotFound); err != nil {
			return err
		}
		statement, err := connection.PrepareContext(ctx, "DELETE FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?")
		if err != nil {
			return err
		}
		defer statement.Close()
		for _, item := range items {
			result, err := statement.ExecContext(ctx, owner, folder, item.ArticleId, item.DbName)
			if err != nil {
				return err
			}
			removed, err := result.RowsAffected()
			if err != nil {
				return err
			}
			count += removed
		}
		return nil
	})
	if err != nil {
		return 0, err
	}
	return count, nil
}

// BulkMove copies notes then counts all source deletions, including references already in the target.
func (repository *Repository) BulkMove(ctx context.Context, owner identity.Id, source, target int64, items []Reference) (int64, error) {
	if err := positive("source_folder_id", source); err != nil {
		return 0, err
	}
	if err := positive("target_folder_id", target); err != nil {
		return 0, err
	}
	if source == target {
		return 0, ErrSameFolders
	}
	if err := itemCount("articles", len(items)); err != nil {
		return 0, err
	}
	for _, item := range items {
		if err := validateReference(item); err != nil {
			return 0, err
		}
	}
	items = uniqueReferences(items)
	if len(items) == 0 {
		return 0, nil
	}
	var count storage.Integer
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := ensureFolder(ctx, connection, owner, source, ErrSourceFolderNotFound); err != nil {
			return err
		}
		if err := ensureFolder(ctx, connection, owner, target, ErrTargetFolderNotFound); err != nil {
			return err
		}
		insert, err := connection.PrepareContext(ctx, "INSERT OR IGNORE INTO favorites(user_id,folder_id,article_id,db_name,note,created_at) SELECT user_id,?,article_id,db_name,note,? FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?")
		if err != nil {
			return err
		}
		defer insert.Close()
		now := nowSeconds()
		for _, item := range items {
			if _, err := insert.ExecContext(ctx, target, now, owner, source, item.ArticleId, item.DbName); err != nil {
				return err
			}
		}
		var before storage.Integer
		if err := connection.QueryRowContext(ctx, "SELECT total_changes()").Scan(&before); err != nil {
			return err
		}
		remove, err := connection.PrepareContext(ctx, "DELETE FROM favorites WHERE user_id=? AND folder_id=? AND article_id=? AND db_name=?")
		if err != nil {
			return err
		}
		defer remove.Close()
		for _, item := range items {
			if _, err := remove.ExecContext(ctx, owner, source, item.ArticleId, item.DbName); err != nil {
				return err
			}
		}
		return connection.QueryRowContext(ctx, "SELECT total_changes()-?", before).Scan(&count)
	})
	if err != nil {
		return 0, err
	}
	return int64(count), nil
}

// Membership identifies an owned folder containing a database-qualified article.
type Membership struct {
	FolderId   int64  `json:"folder_id"`
	FolderName string `json:"folder_name"`
}

// BatchMembership retains first-occurrence request ordering after filtering nonpositive identifiers.
type BatchMembership struct {
	ArticleId identity.Id  `json:"article_id"`
	Folders   []Membership `json:"folders"`
}

// IsFavorited returns all owned memberships for one exact reference.
func (repository *Repository) IsFavorited(ctx context.Context, owner identity.Id, item Reference) ([]Membership, error) {
	if err := validateReference(item); err != nil {
		return nil, err
	}
	result := []Membership{}
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		rows, err := connection.QueryContext(ctx, "SELECT fav.folder_id,f.name FROM favorites fav JOIN folders f ON fav.folder_id=f.id WHERE fav.user_id=? AND fav.article_id=? AND fav.db_name=?", owner, item.ArticleId, item.DbName)
		if err != nil {
			return err
		}
		defer rows.Close()
		for rows.Next() {
			var id storage.Integer
			var name storage.Text
			if err := rows.Scan(&id, &name); err != nil {
				return err
			}
			result = append(result, Membership{int64(id), string(name)})
		}
		return rows.Err()
	})
	return result, err
}

// BatchIsFavorited checks at most 500 raw identifiers and ignores repeated or nonpositive entries.
func (repository *Repository) BatchIsFavorited(ctx context.Context, owner identity.Id, ids []int64, name string) ([]BatchMembership, error) {
	if err := itemCount("article_ids", len(ids)); err != nil {
		return nil, err
	}
	if err := characters("db_name", name, 255); err != nil {
		return nil, err
	}
	result := []BatchMembership{}
	positions := map[int64]int{}
	values := []any{owner, name}
	for _, id := range ids {
		if _, exists := positions[id]; id > 0 && !exists {
			positions[id] = len(result)
			result = append(result, BatchMembership{identity.Id(id), []Membership{}})
			values = append(values, id)
		}
	}
	if len(result) == 0 {
		return result, nil
	}
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		marks := strings.TrimSuffix(strings.Repeat("?,", len(result)), ",")
		rows, err := connection.QueryContext(ctx, "SELECT fav.article_id,fav.folder_id,f.name FROM favorites fav JOIN folders f ON fav.folder_id=f.id WHERE fav.user_id=? AND fav.db_name=? AND fav.article_id IN ("+marks+") ORDER BY fav.article_id,fav.created_at", values...)
		if err != nil {
			return err
		}
		defer rows.Close()
		for rows.Next() {
			var article, folder storage.Integer
			var name storage.Text
			if err := rows.Scan(&article, &folder, &name); err != nil {
				return err
			}
			position := positions[int64(article)]
			result[position].Folders = append(result[position].Folders, Membership{int64(folder), string(name)})
		}
		return rows.Err()
	})
	return result, err
}
