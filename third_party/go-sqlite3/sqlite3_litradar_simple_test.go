//go:build cgo

package sqlite3

import (
	"database/sql/driver"
	"errors"
	"fmt"
	"sync"
	"testing"
)

// simpleTestConnection acquires one raw physical connection without a registration hook.
func simpleTestConnection(t *testing.T) *SQLiteConn {
	t.Helper()
	instance := &SQLiteDriver{}
	value, err := instance.Open(":memory:")
	if err != nil {
		t.Fatal(err)
	}
	connection := value.(*SQLiteConn)
	t.Cleanup(func() { connection.Close() })
	return connection
}

// TestRegisterSimpleRetainsConnectionRoles checks opt-in, repeat registration, SQL admission and closed handles.
func TestRegisterSimpleRetainsConnectionRoles(t *testing.T) {
	connection := simpleTestConnection(t)
	if rows, err := connection.Query("SELECT simple_query('中文')", nil); err == nil {
		rows.Close()
		t.Fatal("plain connection acquired global Simple functions")
	}
	rows, err := connection.Query("SELECT sqlite_compileoption_used('ENABLE_FTS5')", nil)
	if err != nil {
		t.Fatal(err)
	}
	values := make([]driver.Value, 1)
	if err := rows.Next(values); err != nil {
		t.Fatal(err)
	}
	rows.Close()
	if values[0] != int64(1) {
		var failure Error
		if err := connection.RegisterSimple(); !errors.As(err, &failure) || failure.Code == 0 {
			t.Fatalf("missing FTS5 initializer status was lost: %v", err)
		}
		return
	}
	for attempt := 0; attempt < 2; attempt++ {
		if err := connection.RegisterSimple(); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := connection.Exec("CREATE VIRTUAL TABLE search USING fts5(text,tokenize='simple 0'); INSERT INTO search VALUES('中文期刊 alpha')", nil); err != nil {
		t.Fatal(err)
	}
	rows, err = connection.Query("SELECT count(*) FROM search WHERE search MATCH '中文'", nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := rows.Next(values); err != nil || values[0] != int64(1) {
		t.Fatalf("static tokenizer failed: %v %v", values, err)
	}
	rows.Close()
	if _, err := connection.Exec("SELECT load_extension('untrusted')", nil); err == nil {
		t.Fatal("registration enabled arbitrary SQL extension loading")
	}
	plain := simpleTestConnection(t)
	if rows, err := plain.Query("SELECT simple_query('中文')", nil); err == nil {
		rows.Close()
		t.Fatal("registration leaked into another physical connection")
	}
	if err := connection.Close(); err != nil {
		t.Fatal(err)
	}
	var failure Error
	if err := connection.RegisterSimple(); !errors.As(err, &failure) || failure.Code != ErrMisuse {
		t.Fatalf("closed registration did not return misuse: %v", err)
	}
}

// TestRegisterSimpleConcurrentConnections initializes independent native handles concurrently.
func TestRegisterSimpleConcurrentConnections(t *testing.T) {
	probe := simpleTestConnection(t)
	rows, err := probe.Query("SELECT sqlite_compileoption_used('ENABLE_FTS5')", nil)
	if err != nil {
		t.Fatal(err)
	}
	values := make([]driver.Value, 1)
	if err := rows.Next(values); err != nil {
		t.Fatal(err)
	}
	rows.Close()
	if values[0] != int64(1) {
		if err := probe.RegisterSimple(); err == nil {
			t.Fatal("missing FTS5 initialization succeeded")
		}
		return
	}
	if err := probe.RegisterSimple(); err != nil {
		t.Fatal(err)
	}
	const count = 16
	results := make(chan error, count)
	var workers sync.WaitGroup
	for workerIndex := 0; workerIndex < count; workerIndex++ {
		workers.Add(1)
		go func() {
			defer workers.Done()
			instance := &SQLiteDriver{}
			value, err := instance.Open(":memory:")
			if err != nil {
				results <- err
				return
			}
			connection := value.(*SQLiteConn)
			defer connection.Close()
			if err := connection.RegisterSimple(); err != nil {
				results <- err
				return
			}
			if _, err := connection.Exec("CREATE VIRTUAL TABLE search USING fts5(text,tokenize='simple 0'); INSERT INTO search VALUES('中文期刊')", nil); err != nil {
				results <- err
				return
			}
			rows, err := connection.Query("SELECT count(*) FROM search WHERE search MATCH '中文'", nil)
			if err != nil {
				results <- err
				return
			}
			defer rows.Close()
			values := make([]driver.Value, 1)
			if err := rows.Next(values); err != nil || values[0] != int64(1) {
				results <- fmt.Errorf("concurrent static search: %v %v", values, err)
			}
		}()
	}
	workers.Wait()
	close(results)
	for err := range results {
		t.Error(err)
	}
}
