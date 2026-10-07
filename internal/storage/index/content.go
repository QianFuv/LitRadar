package index

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"

	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// Connection owns one physical SQLite connection and its otherwise private pool.
type Connection struct {
	*sql.Conn
	database *sql.DB
}

// Close releases the pinned connection and closes its backing pool.
func (connection *Connection) Close() error {
	return errors.Join(connection.Conn.Close(), connection.database.Close())
}

// OpenContent opens the runtime content role without creating parent directories or upgrading legacy schemas.
func OpenContent(ctx context.Context, path string) (*Connection, error) {
	database, err := sqlite.Open(path, false, 1)
	if err != nil {
		return nil, err
	}
	physical, err := database.Conn(ctx)
	if err != nil {
		database.Close()
		return nil, err
	}
	connection := &Connection{physical, database}
	if _, err := connection.ExecContext(ctx, "PRAGMA busy_timeout=30000"); err != nil {
		connection.Close()
		return nil, err
	}
	if err := InitContent(ctx, connection.Conn); err != nil {
		connection.Close()
		return nil, err
	}
	return connection, nil
}

// RebuildRequired identifies a content database needing an explicit migration or rebuild.
type RebuildRequired struct{ FoundVersion int64 }

func (err *RebuildRequired) Error() string {
	return fmt.Sprintf("index schema version %d requires an explicit rebuild for content schema v%d", err.FoundVersion, indexschema.Version)
}

// InvalidContentSchema describes malformed runtime content structure without exposing provider content.
type InvalidContentSchema struct{ Message string }

func (err *InvalidContentSchema) Error() string {
	return fmt.Sprintf("invalid content schema v%d: %s", indexschema.Version, err.Message)
}

// InitContent accepts existing v6-v9 schemas or atomically initializes a completely empty database.
func InitContent(ctx context.Context, connection *sql.Conn) error {
	isSimple, err := search.UsesSimple(ctx, connection)
	if err != nil {
		return err
	}
	if isSimple {
		if err := sqlite.LoadSimple(connection); err != nil {
			return err
		}
	}
	if _, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;"); err != nil {
		return err
	}
	version, count, err := readContentSchemaState(ctx, connection)
	if err != nil {
		return err
	}
	if version >= 6 && version <= sqlite.Integer(indexschema.Version) {
		return validateContent(ctx, connection, int(version))
	}
	if version != 0 || count != 0 {
		return &RebuildRequired{int64(version)}
	}
	return initializeEmptyContent(ctx, connection)
}

func validateContent(ctx context.Context, connection *sql.Conn, version int) error {
	if err := indexschema.ValidateStructure(ctx, connection, version); err != nil {
		var invalid indexschema.InvalidStructure
		if errors.As(err, &invalid) {
			return &InvalidContentSchema{invalid.Message}
		}
		return err
	}
	for _, fragment := range []string{"provider", "source", "platform", "url", "permalink", "content_location", "full_text", "checkpoint", "lease", "run_id", "statistics", "stats"} {
		var count sqlite.Integer
		if err := connection.QueryRowContext(ctx, `SELECT COUNT(*) FROM pragma_table_list() AS tables JOIN pragma_table_xinfo(tables.name) AS columns WHERE tables.schema='main' AND tables.name NOT LIKE 'article_search_%' AND lower(columns.name) LIKE '%' || ?1 || '%'`, fragment).Scan(&count); err != nil {
			return err
		}
		if count != 0 {
			return &InvalidContentSchema{"forbidden column fragment " + fragment}
		}
	}
	_, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=ON")
	return err
}

// OptimizeContent refreshes SQLite planner statistics after a successful catalog traversal.
func OptimizeContent(ctx context.Context, connection *sql.Conn) error {
	_, err := connection.ExecContext(ctx, "PRAGMA optimize;")
	return err
}

func immediate(ctx context.Context, connection *sql.Conn, operation func() error) (err error) {
	if _, err = connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return err
	}
	defer func() { _, _ = connection.ExecContext(context.Background(), "ROLLBACK") }()
	if err = operation(); err != nil {
		return err
	}
	_, err = connection.ExecContext(ctx, "COMMIT")
	return err
}

func optionalString(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}
func jsonArray(value any) (string, error) {
	encoded, err := jsonBytes(value)
	return string(encoded), err
}
func isTrimmed(value string) bool { return value != "" && strings.TrimSpace(value) == value }

func initializeEmptyContent(ctx context.Context, connection *sql.Conn) error {
	if err := sqlite.LoadSimple(connection); err != nil {
		return err
	}
	if err := immediate(ctx, connection, func() error {
		_, err := connection.ExecContext(ctx, indexschema.ContentTables+fmt.Sprintf("\nPRAGMA user_version=%d;", indexschema.Version))
		return err
	}); err != nil {
		return err
	}
	return validateContent(ctx, connection, indexschema.Version)
}

func readContentSchemaState(ctx context.Context, connection *sql.Conn) (sqlite.Integer, sqlite.Integer, error) {
	var version, count sqlite.Integer
	if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
		return version, count, err
	}
	if err := connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'").Scan(&count); err != nil {
		return version, count, err
	}
	return version, count, nil
}
