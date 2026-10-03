// Package sqlite initializes every physical native SQLite connection with explicit policy.
package sqlite

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	sqlite3 "github.com/mattn/go-sqlite3"
)

// Config describes a database role and a trusted composition-supplied extension, never a data-derived library.
type Config struct {
	Filename       string
	Mode           string
	NoFollow       bool
	SimpleLibrary  string
	MaxConnections int
}

type connector struct {
	driver      *sqlite3.SQLiteDriver
	dsn         string
	filename    string
	rejectLinks bool
	opening     chan struct{}
}

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
	if connection.rejectLinks {
		metadata, err := os.Lstat(connection.filename)
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return nil, err
		}
		if err == nil && !metadata.Mode().IsRegular() {
			return nil, fmt.Errorf("database path is not a regular file")
		}
	}
	return connection.driver.Open(connection.dsn)
}

func (connection *connector) Driver() driver.Driver { return connection.driver }

// FileUri encodes filesystem punctuation as path data while keeping explicit SQLite options separate.
func FileUri(filename, mode string) (string, error) {
	if mode != "ro" && mode != "rw" && mode != "rwc" {
		return "", fmt.Errorf("unsupported SQLite open mode")
	}
	if strings.IndexByte(filename, 0) >= 0 || filename == "" {
		return "", fmt.Errorf("invalid SQLite filename")
	}
	absolute, err := filepath.Abs(filename)
	if err != nil {
		return "", err
	}
	converted := filepath.ToSlash(absolute)
	if runtime.GOOS == "windows" {
		if !strings.HasPrefix(converted, "//") {
			converted = "/" + converted
		}
	}
	location := url.URL{Scheme: "file", Path: converted}
	query := url.Values{"mode": {mode}, "_busy_timeout": {"30000"}, "_foreign_keys": {"1"}, "_synchronous": {"NORMAL"}}
	if mode != "ro" {
		query.Set("_journal_mode", "WAL")
	}
	location.RawQuery = query.Encode()
	return location.String(), nil
}

// Open constructs a lazy pool whose hooks run on all physical opens and reconnects.
func Open(config Config) (*sql.DB, error) {
	dsn, err := FileUri(config.Filename, config.Mode)
	if err != nil {
		return nil, err
	}
	if config.SimpleLibrary != "" && !filepath.IsAbs(config.SimpleLibrary) {
		return nil, fmt.Errorf("Simple library must be an absolute trusted path")
	}
	if config.MaxConnections < 1 {
		return nil, fmt.Errorf("SQLite pool capacity must be positive")
	}
	instance := &sqlite3.SQLiteDriver{NoFollow: config.NoFollow}
	instance.ConnectHook = func(connection *sqlite3.SQLiteConn) error {
		if config.SimpleLibrary != "" {
			if err := connection.LoadExtension(config.SimpleLibrary, "sqlite3_simple_init"); err != nil {
				return fmt.Errorf("load trusted Simple tokenizer: %w", err)
			}
		}
		return nil
	}
	database := sql.OpenDB(&connector{driver: instance, dsn: dsn, filename: config.Filename, rejectLinks: config.NoFollow, opening: make(chan struct{}, 1)})
	database.SetMaxOpenConns(config.MaxConnections)
	database.SetMaxIdleConns(config.MaxConnections)
	return database, nil
}
