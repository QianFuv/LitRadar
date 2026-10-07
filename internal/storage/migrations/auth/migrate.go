package auth

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"time"

	platformsqlite "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	sqlite3 "github.com/mattn/go-sqlite3"
)

const SchemaVersion = 20

func init() { sql.Register("litradar-auth-migration", &sqlite3.SQLiteDriver{DeferSynchronous: true}) }

var ErrProviderState = errors.New("legacy runtime Provider order state is invalid for migration")
var ErrNotificationState = errors.New("notification settings list state is invalid for migration")

// Summary records the initially observed version and the successfully reached version.
type Summary struct {
	FromVersion int `json:"from_version"`
	ToVersion   int `json:"to_version"`
}

// UnsupportedVersion rejects a newer database before enabling writable journal policy.
type UnsupportedVersion struct{ Found int }

func (failure UnsupportedVersion) Error() string {
	return fmt.Sprintf("unsupported auth database schema version %d; this binary supports up to %d", failure.Found, SchemaVersion)
}

// Migrate upgrades each historical version in its own immediate transaction.
// Current databases are not revalidated or reconfigured by this entry point.
func Migrate(ctx context.Context, filename string) (Summary, error) {
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		return Summary{}, err
	}
	database, err := openMigrationDatabase(filename)
	if err != nil {
		return Summary{}, err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return Summary{}, err
	}
	defer connection.Close()
	var version int
	if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
		return Summary{}, err
	}
	summary := Summary{FromVersion: version, ToVersion: version}
	if version > SchemaVersion {
		return summary, UnsupportedVersion{version}
	}
	if version == SchemaVersion {
		return summary, nil
	}
	if err := execute(ctx, connection, "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;"); err != nil {
		return summary, err
	}
	for version < SchemaVersion {
		locked, err := migrateStep(ctx, connection, version, summary.FromVersion)
		if err != nil {
			return summary, err
		}
		version = locked
		summary.ToVersion = version
	}
	return summary, nil
}

func openMigrationDatabase(filename string) (*sql.DB, error) {
	dsn, err := platformsqlite.FileUri(filename, "rwc")
	if err != nil {
		return nil, err
	}
	location, err := url.Parse(dsn)
	if err != nil {
		return nil, err
	}
	query := location.Query()
	query.Del("_journal_mode")
	query.Del("_foreign_keys")
	query.Del("_synchronous")
	location.RawQuery = query.Encode()
	database, err := sql.Open("litradar-auth-migration", location.String())
	if err != nil {
		return nil, err
	}
	database.SetMaxOpenConns(1)
	return database, nil
}

func migrateStep(ctx context.Context, connection *sql.Conn, version, fromVersion int) (int, error) {
	if err := execute(ctx, connection, "BEGIN IMMEDIATE"); err != nil {
		return version, err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	var locked int
	if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&locked); err != nil {
		return version, err
	}
	if locked > SchemaVersion {
		return version, UnsupportedVersion{locked}
	}
	if locked != version {
		return locked, nil
	}
	next := version + 1
	if err := applyVersion(ctx, connection, next, fromVersion); err != nil {
		return version, err
	}
	if err := execute(ctx, connection, fmt.Sprintf("PRAGMA user_version = %d", next)); err != nil {
		return version, err
	}
	if err := execute(ctx, connection, "COMMIT"); err != nil {
		return version, err
	}
	return next, nil
}

func applyVersion(ctx context.Context, connection *sql.Conn, version, fromVersion int) error {
	statements := map[int]string{2: versionTwoSql, 3: versionThreeSql, 4: versionFourSql, 5: versionFiveSql, 6: versionSixSql, 9: versionNineSql, 10: versionTenSql, 11: versionElevenSql, 16: versionSixteenSql, 17: versionSeventeenSql, 18: versionEighteenSql}
	if statement, exists := statements[version]; exists {
		return execute(ctx, connection, statement)
	}
	switch version {
	case 1:
		return versionOne(ctx, connection)
	case 7:
		return versionSeven(ctx, connection)
	case 8:
		return versionEight(ctx, connection, fromVersion)
	case 12:
		return versionTwelve(ctx, connection)
	case 13, 15:
		return addGenerationColumn(ctx, connection, version)
	case 14:
		return versionFourteen(ctx, connection)
	case 19:
		return rewriteProviders(ctx, connection, true)
	case 20:
		return versionTwenty(ctx, connection)
	default:
		return fmt.Errorf("unimplemented auth migration version %d", version)
	}
}

func execute(ctx context.Context, connection *sql.Conn, statement string, arguments ...any) error {
	_, err := connection.ExecContext(ctx, statement, arguments...)
	return err
}

func columns(ctx context.Context, connection *sql.Conn, table string) ([]string, error) {
	rows, err := connection.QueryContext(ctx, "SELECT name FROM pragma_table_info(?) ORDER BY cid", table)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []string{}
	for rows.Next() {
		var name string
		if err := rows.Scan(&name); err != nil {
			return nil, err
		}
		result = append(result, name)
	}
	return result, rows.Err()
}

func addColumn(ctx context.Context, connection *sql.Conn, table, column, statement string) error {
	names, err := columns(ctx, connection, table)
	if err != nil {
		return err
	}
	if len(names) == 0 || slices.Contains(names, column) {
		return nil
	}
	return execute(ctx, connection, statement)
}

func versionOne(ctx context.Context, connection *sql.Conn) error {
	if err := execute(ctx, connection, authTablesSql); err != nil {
		return err
	}
	names, err := columns(ctx, connection, "users")
	if err != nil {
		return err
	}
	if !slices.Contains(names, "is_admin") {
		if err := execute(ctx, connection, "ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0; UPDATE users SET is_admin = 1 WHERE id = (SELECT MIN(id) FROM users);"); err != nil {
			return err
		}
	}
	for _, column := range []struct{ name, definition string }{
		{"selected_databases", "TEXT NOT NULL DEFAULT '[]'"},
		{"ai_base_url", "TEXT NOT NULL DEFAULT ''"}, {"ai_api_key", "TEXT NOT NULL DEFAULT ''"},
		{"ai_model", "TEXT NOT NULL DEFAULT ''"}, {"ai_system_prompt", "TEXT NOT NULL DEFAULT ''"},
		{"ai_backup_base_url", "TEXT NOT NULL DEFAULT ''"}, {"ai_backup_api_key", "TEXT NOT NULL DEFAULT ''"},
		{"ai_backup_model", "TEXT NOT NULL DEFAULT ''"}, {"ai_backup_system_prompt", "TEXT NOT NULL DEFAULT ''"},
		{"ai_retry_attempts", "INTEGER NOT NULL DEFAULT 3"}, {"sync_to_tracking_folder", "INTEGER NOT NULL DEFAULT 0"},
	} {
		if err := addColumn(ctx, connection, "notification_settings", column.name, "ALTER TABLE notification_settings ADD COLUMN "+column.name+" "+column.definition); err != nil {
			return err
		}
	}
	if err := addColumn(ctx, connection, "announcements", "priority", "ALTER TABLE announcements ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal'"); err != nil {
		return err
	}
	return execute(ctx, connection, authIndexesSql)
}

func nowSeconds() float64 { return float64(time.Now().UnixNano()) / 1e9 }

func checkForeignKeys(ctx context.Context, connection *sql.Conn) error {
	var count int
	if err := connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM pragma_foreign_key_check").Scan(&count); err != nil {
		return err
	}
	if count != 0 {
		return errors.New("invalid query")
	}
	return nil
}

// versionTwenty restores the consumed scheduler sequence after rebuilding manual run slots.
func versionTwenty(ctx context.Context, connection *sql.Conn) error {
	var sequence int64
	if err := connection.QueryRowContext(ctx, "SELECT COALESCE(MAX(seq), 0) FROM sqlite_sequence WHERE name = 'scheduled_task_runs'").Scan(&sequence); err != nil {
		return err
	}
	if err := execute(ctx, connection, versionTwentySql); err != nil {
		return err
	}
	if err := execute(ctx, connection, "UPDATE sqlite_sequence SET seq = MAX(seq, ?1) WHERE name = 'scheduled_task_runs'", sequence); err != nil {
		return err
	}
	return execute(ctx, connection, "INSERT INTO sqlite_sequence (name, seq) SELECT 'scheduled_task_runs', ?1 WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'scheduled_task_runs')", sequence)
}

// addGenerationColumn retains the two historical generation-column definitions.
func addGenerationColumn(ctx context.Context, connection *sql.Conn, version int) error {
	if version == 13 {
		return addColumn(ctx, connection, "cnki_sessions", "generation", "ALTER TABLE cnki_sessions ADD COLUMN generation INTEGER NOT NULL DEFAULT 1 CHECK (generation > 0)")
	}
	return addColumn(ctx, connection, "users", "token_generation", "ALTER TABLE users ADD COLUMN token_generation INTEGER NOT NULL DEFAULT 0 CHECK (token_generation >= 0)")
}
