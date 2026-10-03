package maintenance

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/binary"
	"fmt"
	"math"
	"net/url"
	"os"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/index"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func readConnection(ctx context.Context, filename string, read func(*sql.Conn) error) error {
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	if _, err := connection.ExecContext(ctx, "PRAGMA temp_store=FILE"); err != nil {
		return err
	}
	return read(connection)
}
func sourceUri(filename string) (string, error) {
	value, err := platform.FileUri(filename, "ro")
	if err != nil {
		return "", invalid("SQLite source path cannot be represented as a file URI")
	}
	parsed, err := url.Parse(value)
	if err != nil {
		return "", err
	}
	parsed.RawQuery = "mode=ro&immutable=1"
	return parsed.String(), nil
}
func quote(value string) string { return `"` + strings.ReplaceAll(value, `"`, `""`) + `"` }

func validateIntegrity(ctx context.Context, connection *sql.Conn, name string) error {
	var check storage.Text
	if err := connection.QueryRowContext(ctx, "PRAGMA quick_check").Scan(&check); err != nil {
		return err
	}
	if check != "ok" {
		return validation(name, "SQLite quick_check")
	}
	var violations storage.Integer
	if err := connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM pragma_foreign_key_check").Scan(&violations); err != nil {
		return err
	}
	if violations != 0 {
		return validation(name, "SQLite foreign_key_check")
	}
	var mismatch bool
	if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT article_id FROM articles EXCEPT SELECT rowid FROM article_search) OR EXISTS(SELECT rowid FROM article_search EXCEPT SELECT article_id FROM articles)").Scan(&mismatch); err != nil {
		return err
	}
	if mismatch {
		return validation(name, "article-to-FTS rowid membership")
	}
	return nil
}

type resultDigest struct {
	count uint64
	hash  [32]byte
}

func searchDigest(ctx context.Context, connection *sql.Conn, query string) (resultDigest, error) {
	usesSimple, err := search.UsesSimple(ctx, connection)
	if err != nil {
		return resultDigest{}, err
	}
	query = search.PrepareQuery(query, usesSimple, domain.SearchAdvanced)
	rows, err := connection.QueryContext(ctx, "SELECT rowid FROM article_search WHERE article_search MATCH ? ORDER BY rowid", query)
	if err != nil {
		return resultDigest{}, err
	}
	defer rows.Close()
	hasher := sha256.New()
	count := uint64(0)
	for rows.Next() {
		var id storage.Integer
		if err := rows.Scan(&id); err != nil {
			return resultDigest{}, err
		}
		var encoded [8]byte
		binary.LittleEndian.PutUint64(encoded[:], uint64(id))
		hasher.Write(encoded[:])
		if count < math.MaxUint64 {
			count++
		}
	}
	result := resultDigest{count: count}
	copy(result.hash[:], hasher.Sum(nil))
	return result, rows.Err()
}
func validateSource(ctx context.Context, source sourceDatabase) error {
	return readConnection(ctx, source.path, func(connection *sql.Conn) error {
		if err := validateIntegrity(ctx, connection, source.name); err != nil {
			return err
		}
		for _, query := range searchCorpus {
			if _, err := searchDigest(ctx, connection, query); err != nil {
				return err
			}
		}
		return nil
	})
}

func buildDatabase(ctx context.Context, source sourceDatabase, destination string) error {
	if _, err := migration.Migrate(ctx, destination); err != nil {
		return validation(source.name, fmt.Sprintf("v9 staging initialization: %v", err))
	}
	if err := copyCanonical(ctx, source, destination); err != nil {
		return err
	}
	file, err := os.OpenFile(destination, os.O_RDWR, 0)
	if err != nil {
		return err
	}
	err = file.Sync()
	closeError := file.Close()
	if err != nil {
		return err
	}
	if closeError != nil {
		return closeError
	}
	info, err := os.Stat(source.path)
	if err != nil {
		return err
	}
	return os.Chmod(destination, info.Mode())
}
func copyCanonical(ctx context.Context, source sourceDatabase, destination string) error {
	database, err := storage.Open(destination, false, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	if _, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=ON; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=FILE;"); err != nil {
		return err
	}
	rows, err := connection.QueryContext(ctx, "SELECT name,sql FROM sqlite_schema WHERE type='index' AND sql IS NOT NULL ORDER BY name")
	if err != nil {
		return err
	}
	indexes := [][2]string{}
	for rows.Next() {
		var name, definition storage.Text
		if err := rows.Scan(&name, &definition); err != nil {
			rows.Close()
			return err
		}
		indexes = append(indexes, [2]string{string(name), string(definition)})
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return err
	}
	for _, index := range indexes {
		if _, err := connection.ExecContext(ctx, "DROP INDEX "+quote(index[0])); err != nil {
			return err
		}
	}
	uri, err := sourceUri(source.path)
	if err != nil {
		return err
	}
	if _, err := connection.ExecContext(ctx, "ATTACH DATABASE ? AS source", uri); err != nil {
		return err
	}
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	for _, table := range tables {
		statement := "INSERT INTO main." + table.name + " (" + table.columns + ") SELECT " + table.columns + " FROM source." + table.name
		if _, err := connection.ExecContext(ctx, statement); err != nil {
			return err
		}
	}
	if err := search.Rebuild(ctx, connection); err != nil {
		return err
	}
	if _, err := connection.ExecContext(ctx, "COMMIT; DETACH DATABASE source;"); err != nil {
		return err
	}
	for _, index := range indexes {
		if _, err := connection.ExecContext(ctx, index[1]); err != nil {
			return err
		}
	}
	_, err = connection.ExecContext(ctx, "INSERT INTO article_search(article_search) VALUES('optimize'); ANALYZE; VACUUM; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
	return err
}

func countTable(ctx context.Context, connection *sql.Conn, database, table string) (uint64, error) {
	var count storage.Integer
	err := connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM "+database+"."+table).Scan(&count)
	if err != nil {
		return 0, err
	}
	if count < 0 {
		return 0, fmt.Errorf("negative table count")
	}
	return uint64(count), nil
}
func differentKeys(ctx context.Context, connection *sql.Conn, table, keys string) (bool, error) {
	statement := "SELECT EXISTS(SELECT " + keys + " FROM main." + table + " EXCEPT SELECT " + keys + " FROM source." + table + ") OR EXISTS(SELECT " + keys + " FROM source." + table + " EXCEPT SELECT " + keys + " FROM main." + table + ")"
	var mismatch bool
	err := connection.QueryRowContext(ctx, statement).Scan(&mismatch)
	return mismatch, err
}
func validateRebuilt(ctx context.Context, source sourceDatabase, candidate string) (DatabaseReport, error) {
	result := DatabaseReport{Database: source.name, SourceSchemaVersion: source.version, TargetSchemaVersion: 9, RowCounts: map[string]uint64{}}
	if _, err := migration.Preflight(ctx, candidate); err != nil {
		return result, validation(source.name, fmt.Sprintf("v9 exact schema preflight: %v", err))
	}
	err := readConnection(ctx, candidate, func(connection *sql.Conn) error {
		if err := validateIntegrity(ctx, connection, source.name); err != nil {
			return err
		}
		uri, err := sourceUri(source.path)
		if err != nil {
			return err
		}
		if _, err := connection.ExecContext(ctx, "ATTACH DATABASE ? AS source", uri); err != nil {
			return err
		}
		if _, err := connection.ExecContext(ctx, "PRAGMA query_only=ON"); err != nil {
			return err
		}
		for _, table := range tables {
			count, err := countTable(ctx, connection, "main", table.name)
			if err != nil {
				return err
			}
			original, err := countTable(ctx, connection, "source", table.name)
			if err != nil {
				return err
			}
			if count != original {
				return validation(source.name, table.name+" row-count equivalence")
			}
			mismatch, err := differentKeys(ctx, connection, table.name, table.keys)
			if err != nil {
				return err
			}
			if mismatch {
				return validation(source.name, table.name+" key equivalence")
			}
			result.RowCounts[table.name] = count
		}
		count, err := countTable(ctx, connection, "main", "article_search")
		if err != nil {
			return err
		}
		original, err := countTable(ctx, connection, "source", "article_search")
		if err != nil {
			return err
		}
		mismatch, err := differentKeys(ctx, connection, "article_search", "rowid")
		if err != nil {
			return err
		}
		if mismatch || count != original {
			return validation(source.name, "FTS rowid equivalence")
		}
		result.RowCounts["article_search"] = count
		return nil
	})
	if err != nil {
		return result, err
	}
	err = readConnection(ctx, source.path, func(before *sql.Conn) error {
		return readConnection(ctx, candidate, func(after *sql.Conn) error {
			for _, query := range searchCorpus {
				original, err := searchDigest(ctx, before, query)
				if err != nil {
					return err
				}
				current, err := searchDigest(ctx, after, query)
				if err != nil {
					return err
				}
				if source.version == 9 && original != current {
					return validation(source.name, "deterministic FTS result equivalence")
				}
			}
			return nil
		})
	})
	if err != nil {
		return result, err
	}
	result.Before, err = measure(ctx, source.path)
	if err != nil {
		return result, err
	}
	result.After, err = measure(ctx, candidate)
	if err != nil {
		return result, err
	}
	if result.After.HasContentShadow {
		return result, validation(source.name, "contentless FTS shadow-table absence")
	}
	if result.After.PageCount > 0 && saturatingMultiply(result.After.FreelistCount, 100) > result.After.PageCount {
		return result, validation(source.name, "freelist ratio at or below one percent")
	}
	return result, nil
}
func saturatingMultiply(first, second uint64) uint64 {
	if second != 0 && first > math.MaxUint64/second {
		return math.MaxUint64
	}
	return first * second
}
func measure(ctx context.Context, filename string) (Measurement, error) {
	result := Measurement{}
	err := readConnection(ctx, filename, func(connection *sql.Conn) error {
		for _, field := range []struct {
			query string
			value *uint64
		}{{"PRAGMA page_size", &result.PageSize}, {"PRAGMA page_count", &result.PageCount}, {"PRAGMA freelist_count", &result.FreelistCount}, {"SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name GLOB 'article_search*'", &result.FtsBytes}} {
			var value storage.Integer
			if err := connection.QueryRowContext(ctx, field.query).Scan(&value); err != nil {
				return err
			}
			if value < 0 {
				return fmt.Errorf("negative storage measurement")
			}
			*field.value = uint64(value)
		}
		result.FreelistBytes = saturatingMultiply(result.PageSize, result.FreelistCount)
		return connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='article_search_content')").Scan(&result.HasContentShadow)
	})
	if err != nil {
		return result, err
	}
	info, err := os.Stat(filename)
	if err != nil {
		return result, err
	}
	result.FileBytes = uint64(info.Size())
	return result, nil
}
