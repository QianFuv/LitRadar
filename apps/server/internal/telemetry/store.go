package telemetry

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"log/slog"
	"time"

	migration "github.com/QianFuv/LitRadar/apps/server/internal/storage/migrations/telemetry"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

type store struct{ database *sql.DB }

// Open migrates a separate database; the runtime logs initialization failure and disables telemetry.
func Open(ctx context.Context, filename string, retentionDays int) (*Collector, error) {
	if retentionDays < 1 || retentionDays > 3650 {
		return nil, errors.New("telemetry retention must be between 1 and 3650 days")
	}
	if err := migration.Migrate(ctx, filename); err != nil {
		return nil, err
	}
	database, err := sqlite.Open(filename, false, 1)
	if err != nil {
		return nil, err
	}
	collector := New()
	collector.store, collector.retentionDays = &store{database}, retentionDays
	return collector, nil
}

// Flush drains each delta once; failed batches are logged and dropped rather than retried.
// Admin reads serialize with flushing so persisted and pending counts never overlap.
func (collector *Collector) Flush(ctx context.Context, now time.Time) error {
	if collector == nil || collector.store == nil {
		return nil
	}
	collector.storeMutex.Lock()
	defer collector.storeMutex.Unlock()
	collector.mutex.Lock()
	batch := collector.pending
	collector.pending = make(map[bucketKey]aggregate)
	collector.mutex.Unlock()
	if dropped := collector.dropped.Swap(0); dropped > 0 {
		slog.WarnContext(ctx, "telemetry.observations.dropped", "component", "telemetry", "count", dropped)
	}
	err := collector.flushBatch(ctx, batch, now)
	if err != nil {
		slog.WarnContext(ctx, "telemetry.flush.failed", "component", "telemetry", "dropped_series", len(batch))
	}
	return err
}

// flushBatch atomically merges histogram deltas and removes expired hours.
func (collector *Collector) flushBatch(ctx context.Context, batch map[bucketKey]aggregate, now time.Time) error {
	transaction, err := collector.store.database.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer transaction.Rollback()
	cutoff := now.UTC().AddDate(0, 0, -collector.retentionDays).Truncate(time.Hour).Unix()
	for key, value := range batch {
		if key.Hour < cutoff {
			continue
		}
		dimensions, err := json.Marshal(key.Dimensions)
		if err != nil {
			return err
		}
		var previous string
		err = transaction.QueryRowContext(ctx, "SELECT aggregate FROM hourly_load WHERE hour=? AND dimensions=?", key.Hour, string(dimensions)).Scan(&previous)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if err == nil {
			var stored aggregate
			if err := json.Unmarshal([]byte(previous), &stored); err != nil {
				return err
			}
			value.merge(stored)
		}
		encoded, err := json.Marshal(value)
		if err != nil {
			return err
		}
		if _, err := transaction.ExecContext(ctx, "INSERT INTO hourly_load(hour,dimensions,aggregate) VALUES(?,?,?) ON CONFLICT(hour,dimensions) DO UPDATE SET aggregate=excluded.aggregate", key.Hour, string(dimensions), string(encoded)); err != nil {
			return err
		}
	}
	if _, err := transaction.ExecContext(ctx, "DELETE FROM hourly_load WHERE hour<?", cutoff); err != nil {
		return err
	}
	return transaction.Commit()
}

// Run flushes once per minute until cancellation; Close performs the final drain.
func (collector *Collector) Run(ctx context.Context) {
	if collector == nil {
		return
	}
	ticker := time.NewTicker(time.Minute)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-ticker.C:
			flushContext, cancel := context.WithTimeout(context.WithoutCancel(ctx), 5*time.Second)
			_ = collector.Flush(flushContext, now)
			cancel()
		}
	}
}

// Close flushes after request workers drain and releases the independent pool.
func (collector *Collector) Close() {
	if collector == nil || collector.store == nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = collector.Flush(ctx, time.Now())
	if err := collector.store.database.Close(); err != nil {
		slog.Warn("telemetry.close.failed", "component", "telemetry")
	}
}
