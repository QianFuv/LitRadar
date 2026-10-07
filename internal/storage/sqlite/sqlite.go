// Package sqlite composes native connections with schema-dependent tokenizer and storage role policy.
package sqlite

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"errors"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"unicode"

	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	native "github.com/mattn/go-sqlite3"
)

type connector struct {
	instance *native.SQLiteDriver
	dsn      string
	opening  chan struct{}
}

// Connect applies the selected native setup to every newly allocated physical connection.
func (connection *connector) Connect(ctx context.Context) (driver.Conn, error) {
	select {
	case connection.opening <- struct{}{}:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	defer func() { <-connection.opening }()
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return connection.instance.Open(connection.dsn)
}

// Driver returns the same configured native driver used by Connect.
func (connection *connector) Driver() driver.Driver { return connection.instance }

// CompactSchema removes Unicode whitespace and folds ASCII case for search declaration matching.
func CompactSchema(value string) string {
	return strings.Map(func(character rune) rune {
		if unicode.IsSpace(character) {
			return -1
		}
		if character >= 'A' && character <= 'Z' {
			return character + 32
		}
		return character
	}, value)
}

func usesSimple(connection *native.SQLiteConn) (bool, error) {
	rows, err := connection.Query("SELECT sql FROM sqlite_schema WHERE type='table' AND name='article_search'", nil)
	if err != nil {
		return false, err
	}
	defer rows.Close()
	values := make([]driver.Value, 1)
	if err := rows.Next(values); errors.Is(err, io.EOF) {
		return false, nil
	} else if err != nil {
		return false, err
	}
	declaration, ok := values[0].(string)
	if !ok {
		return false, errors.New("invalid search table declaration")
	}
	return strings.Contains(CompactSchema(declaration), "tokenize='simple0'"), nil
}

// SimpleLibrary discovers only package and compiled-source locations, never a selected data directory.
func SimpleLibrary() (string, error) {
	name := simpleLibraryName()
	candidates := []string{}
	if runtime.GOOS == "linux" {
		candidates = append(candidates, "/usr/lib/litradar/libsimple.so")
	}
	if executable, err := os.Executable(); err == nil {
		candidates = append(candidates, filepath.Join(filepath.Dir(executable), name))
	}
	if _, filename, _, ok := runtime.Caller(0); ok && filepath.IsAbs(filename) {
		candidates = append(candidates, sourceSimpleLibraries(filename, name)...)
	}
	return firstRegularSimpleLibrary(candidates)
}

// simpleLibraryName preserves the host-native tokenizer filename.
func simpleLibraryName() string {
	name := "libsimple.so"
	if runtime.GOOS == "windows" {
		name = "simple.dll"
	} else if runtime.GOOS == "darwin" {
		name = "libsimple.dylib"
	}
	return name
}

// sourceSimpleLibraries derives build and bundle candidates from the original caller file anchor.
func sourceSimpleLibraries(filename, name string) []string {
	candidates := []string{}
	root := filepath.Dir(filepath.Dir(filepath.Dir(filepath.Dir(filename))))
	candidates = append(candidates, filepath.Join(root, "target", "simple-tokenizer", name))
	if runtime.GOARCH == "amd64" && (runtime.GOOS == "windows" || runtime.GOOS == "linux") {
		candidates = append(candidates, filepath.Join(root, "libs", "simple", runtime.GOOS, name))
	}
	return candidates
}

// firstRegularSimpleLibrary skips stat failures and nonregular candidates without canonicalization.
func firstRegularSimpleLibrary(candidates []string) (string, error) {
	for _, candidate := range candidates {
		if info, err := os.Stat(candidate); err == nil && info.Mode().IsRegular() {
			return candidate, nil
		}
	}
	return "", errors.New("required simple tokenizer is unavailable; build or package the native library")
}

func loadSimple(connection *native.SQLiteConn) error {
	rows, err := connection.Query("SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name='simple_highlight')", nil)
	if err != nil {
		return err
	}
	values := make([]driver.Value, 1)
	err = rows.Next(values)
	rows.Close()
	if err != nil {
		return err
	}
	if values[0] == int64(1) {
		return nil
	}
	library, err := SimpleLibrary()
	if err != nil {
		return err
	}
	return connection.LoadExtension(library, "sqlite3_simple_init")
}

// LoadSimple enables the fixed native tokenizer for an imminent schema creation on this connection.
func LoadSimple(connection *sql.Conn) error {
	return connection.Raw(func(value any) error { return loadSimple(value.(*native.SQLiteConn)) })
}

// Open opens a regular storage pool and detects native search independently on every physical connection.
func Open(filename string, isReadOnly bool, capacity int) (*sql.DB, error) {
	mode := "rwc"
	if isReadOnly {
		mode = "ro"
	}
	instance := &native.SQLiteDriver{}
	instance.ConnectHook = func(connection *native.SQLiteConn) error {
		hasSimple, err := usesSimple(connection)
		if err != nil {
			return err
		}
		if hasSimple {
			if err := loadSimple(connection); err != nil {
				return err
			}
		}
		if !isReadOnly {
			_, err = connection.Exec("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;", nil)
		}
		return err
	}
	return open(filename, mode, capacity, instance)
}

// OpenMigration defers schema-touching policy exclusively for version-first migration connections.
func OpenMigration(filename string) (*sql.DB, error) {
	return open(filename, "rwc", 1, &native.SQLiteDriver{DeferSynchronous: true})
}

// OpenPlain retains the original direct rusqlite reader's five-second timeout without loading FTS or enabling WAL.
func OpenPlain(filename string) (*sql.DB, error) {
	dsn, err := platform.FileUri(filename, "rwc")
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
	query.Set("_busy_timeout", "5000")
	query.Set("_synchronous", "FULL")
	location.RawQuery = query.Encode()
	return pool(location.String(), 1, &native.SQLiteDriver{}), nil
}

func open(filename, mode string, capacity int, instance *native.SQLiteDriver) (*sql.DB, error) {
	if capacity < 1 {
		return nil, errors.New("SQLite pool capacity must be positive")
	}
	dsn, err := platform.FileUri(filename, mode)
	if err != nil {
		return nil, err
	}
	location, err := url.Parse(dsn)
	if err != nil {
		return nil, err
	}
	query := location.Query()
	for _, key := range []string{"_journal_mode", "_foreign_keys", "_synchronous"} {
		query.Del(key)
	}
	location.RawQuery = query.Encode()
	return pool(location.String(), capacity, instance), nil
}

func pool(dsn string, capacity int, instance *native.SQLiteDriver) *sql.DB {
	database := sql.OpenDB(&connector{instance, dsn, make(chan struct{}, 1)})
	database.SetMaxOpenConns(capacity)
	database.SetMaxIdleConns(capacity)
	return database
}
