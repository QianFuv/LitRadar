// Package sqlite composes native connections with storage role policy.
package sqlite

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"errors"
	"net/url"
	"strings"
	"unicode"

	platform "github.com/QianFuv/LitRadar/apps/server/internal/platform/sqlite"
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

// Open opens a regular storage pool and initializes policy on every physical connection.
func Open(filename string, isReadOnly bool, capacity int) (*sql.DB, error) {
	mode := "rwc"
	if isReadOnly {
		mode = "ro"
	}
	instance := &native.SQLiteDriver{}
	instance.ConnectHook = func(connection *native.SQLiteConn) error {
		var err error
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
