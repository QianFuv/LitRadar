package favorites

import (
	"context"
	"database/sql"
	"encoding/base64"
	"errors"
	"fmt"
	"math"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func encodeCursor(owner identity.Id, favorite Favorite) string {
	return base64.RawURLEncoding.EncodeToString([]byte(fmt.Sprintf("1|%d|%d|%016x|%d", owner, favorite.FolderId, math.Float64bits(favorite.CreatedAt), favorite.Id)))
}
func decodeCursor(cursor string, owner identity.Id, folder int64) (float64, int64, error) {
	if len(cursor) > 128 || strings.ContainsAny(cursor, "\r\n") {
		return 0, 0, ErrCursor
	}
	raw, err := base64.RawURLEncoding.Strict().DecodeString(cursor)
	if err != nil || !utf8.Valid(raw) {
		return 0, 0, ErrCursor
	}
	fields := strings.Split(string(raw), "|")
	if len(fields) != 5 || fields[0] != "1" {
		return 0, 0, ErrCursor
	}
	user, err := strconv.ParseInt(fields[1], 10, 64)
	if err != nil || user != int64(owner) {
		return 0, 0, ErrCursor
	}
	selected, err := strconv.ParseInt(fields[2], 10, 64)
	if err != nil || selected != folder {
		return 0, 0, ErrCursor
	}
	bits, err := strconv.ParseUint(strings.TrimPrefix(fields[3], "+"), 16, 64)
	if err != nil {
		return 0, 0, ErrCursor
	}
	id, err := strconv.ParseInt(fields[4], 10, 64)
	stamp := math.Float64frombits(bits)
	if err != nil || id <= 0 || math.IsNaN(stamp) || math.IsInf(stamp, 0) || stamp < 0 {
		return 0, 0, ErrCursor
	}
	return stamp, id, nil
}

func (repository *Repository) readSnapshot(ctx context.Context, read func(*sql.Conn) error) error {
	return repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		if _, err := connection.ExecContext(ctx, "BEGIN"); err != nil {
			return err
		}
		defer connection.ExecContext(context.Background(), "ROLLBACK")
		if err := read(connection); err != nil {
			return err
		}
		_, err := connection.ExecContext(ctx, "COMMIT")
		return err
	})
}

// CitationSnapshot captures the owned folder name and one deterministic bounded reference list.
type CitationSnapshot struct {
	FolderName string
	References []Reference
	HasMore    bool
}

// LoadCitationSnapshot permits a zero limit and uses one extra row for exact oversize detection.
func (repository *Repository) LoadCitationSnapshot(ctx context.Context, owner identity.Id, folder int64, limit uint64) (CitationSnapshot, error) {
	if err := positive("folder_id", folder); err != nil {
		return CitationSnapshot{}, err
	}
	result := CitationSnapshot{References: []Reference{}}
	err := repository.readSnapshot(ctx, func(connection *sql.Conn) error {
		var name storage.Text
		err := connection.QueryRowContext(ctx, "SELECT name FROM folders WHERE id=? AND user_id=?", folder, owner).Scan(&name)
		if errors.Is(err, sql.ErrNoRows) {
			return ErrFolderNotFound
		}
		if err != nil {
			return err
		}
		result.FolderName = string(name)
		queryLimit := int64(math.MaxInt64)
		if limit < uint64(math.MaxInt64) {
			queryLimit = int64(limit + 1)
		}
		rows, err := connection.QueryContext(ctx, "SELECT article_id,db_name FROM favorites WHERE user_id=? AND folder_id=? ORDER BY created_at DESC,id DESC LIMIT ?", owner, folder, queryLimit)
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
			result.References = append(result.References, Reference{identity.Id(id), string(name)})
		}
		return rows.Err()
	})
	if err != nil {
		return CitationSnapshot{}, err
	}
	result.HasMore = uint64(len(result.References)) > limit
	if result.HasMore {
		result.References = result.References[:int(limit)]
	}
	return result, nil
}

// ArticlePage checks ownership before decoding its scope-bound cursor and enriches after committing the read snapshot.
func (repository *Repository) ArticlePage(ctx context.Context, configuration config.Config, owner identity.Id, folder, limit int64, cursor *string) (domain.Page[Article], error) {
	empty := domain.Page[Article]{}
	if err := positive("folder_id", folder); err != nil {
		return empty, err
	}
	if err := positive("limit", limit); err != nil {
		return empty, err
	}
	if limit > 500 {
		return empty, InvalidInput{"limit must contain at most 500 items"}
	}
	var favorites []Favorite
	err := repository.readSnapshot(ctx, func(connection *sql.Conn) error {
		if err := ensureFolder(ctx, connection, owner, folder, ErrFolderNotFound); err != nil {
			return err
		}
		statement := "SELECT id,folder_id,article_id,db_name,note,created_at FROM favorites WHERE user_id=? AND folder_id=?"
		values := []any{owner, folder}
		if cursor != nil {
			stamp, id, err := decodeCursor(*cursor, owner, folder)
			if err != nil {
				return err
			}
			statement += " AND (created_at,id)<(?,?)"
			values = append(values, stamp, id)
		}
		statement += " ORDER BY created_at DESC,id DESC LIMIT ?"
		values = append(values, limit+1)
		var err error
		favorites, err = favoriteRows(ctx, connection, statement, values...)
		return err
	})
	if err != nil {
		return empty, err
	}
	hasMore := int64(len(favorites)) > limit
	var next *string
	if hasMore {
		favorites = favorites[:limit]
		encoded := encodeCursor(owner, favorites[len(favorites)-1])
		next = &encoded
	}
	return domain.Page[Article]{Items: Enrich(ctx, configuration, favorites), Page: domain.PageMeta{Limit: limit, NextCursor: next, HasMore: &hasMore}}, nil
}

// ListArticles enriches the legacy offset list without imposing cursor-page validation.
func (repository *Repository) ListArticles(ctx context.Context, configuration config.Config, owner identity.Id, folder *int64, limit, offset int64) ([]Article, error) {
	favorites, err := repository.ListFavorites(ctx, owner, folder, limit, offset)
	if err != nil {
		return nil, err
	}
	return Enrich(ctx, configuration, favorites), nil
}
