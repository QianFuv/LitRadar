package mcp

import (
	"context"
	"database/sql"
	"errors"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestReadToolCancellationStopsActualQueryWorker(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	database, err := platform.Open(platform.Config{Filename: filepath.Join(configuration.IndexDir, "slow.sqlite"), Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec(`PRAGMA busy_timeout=1; CREATE TABLE marker(value INTEGER); INSERT INTO marker VALUES(0); CREATE VIEW journals AS WITH RECURSIVE numbers(value) AS (SELECT value FROM marker UNION ALL SELECT value+1 FROM numbers WHERE value<20000000) SELECT 'area' AS area FROM numbers`); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	pool := executor.New(1, 30*time.Second)
	defer pool.Wait()
	services := Services{Storage: configuration, Pool: pool}
	requestContext, cancel := context.WithCancel(ctx)
	defer cancel()
	done := make(chan struct{})
	go func() {
		defer close(done)
		_, _ = services.call(requestContext, nil, "list_areas", &toolInput{fields: map[string]any{"db": "slow.sqlite"}})
	}()
	waitForIndexReader(t, database)
	cancel()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("caller did not cancel")
	}
	nextContext, stop := context.WithTimeout(ctx, time.Second)
	defer stop()
	_, err = executor.Run(nextContext, pool, func() (int, error) { return 1, nil })
	if errors.Is(err, context.DeadlineExceeded) {
		t.Fatal("canceled query kept the actual worker occupied")
	}
	if err != nil {
		t.Fatal(err)
	}
}

func waitForIndexReader(t *testing.T, database *sql.DB) {
	t.Helper()
	for deadline := time.Now().Add(3 * time.Second); time.Now().Before(deadline); {
		if _, err := database.Exec("UPDATE marker SET value=1-value"); err != nil {
			t.Fatal(err)
		}
		var busy, frames, checkpointed int
		if err := database.QueryRow("PRAGMA wal_checkpoint(RESTART)").Scan(&busy, &frames, &checkpointed); err != nil {
			t.Fatal(err)
		}
		if busy != 0 {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("query never held a database read snapshot")
}
