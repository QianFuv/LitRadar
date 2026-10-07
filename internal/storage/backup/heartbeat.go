package backup

import (
	"context"
	"database/sql"
	"math"
	"os"
	"strings"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// RecordHeartbeat retains process start time on renewal and expires seven-day-old rows.
func RecordHeartbeat(ctx context.Context, filename string, service ServiceKind, instance string, at float64) error {
	if strings.TrimSpace(instance) == "" || math.IsNaN(at) || math.IsInf(at, 0) {
		return failure("input", "service heartbeat values are invalid")
	}
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	if _, err := database.ExecContext(ctx, "INSERT INTO service_heartbeats(service,instance_id,started_at,heartbeat_at) VALUES(?,?,?,?) ON CONFLICT(service,instance_id) DO UPDATE SET heartbeat_at=excluded.heartbeat_at", service, instance, at, at); err != nil {
		return err
	}
	_, err = database.ExecContext(ctx, "DELETE FROM service_heartbeats WHERE heartbeat_at<?", at-604800)
	return err
}

// DeleteHeartbeat removes only the requested service instance during graceful shutdown.
func DeleteHeartbeat(ctx context.Context, filename string, service ServiceKind, instance string) error {
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	_, err = database.ExecContext(ctx, "DELETE FROM service_heartbeats WHERE service=? AND instance_id=?", service, instance)
	return err
}

// HasRecentHeartbeat treats missing legacy tables as inactive and includes future timestamps.
func HasRecentHeartbeat(ctx context.Context, filename string, now, maximumAge float64) (bool, error) {
	if _, err := os.Stat(filename); err != nil {
		return false, nil
	}
	if hasInvalidHeartbeatWindow(now, maximumAge) {
		return false, failure("input", "heartbeat time window is invalid")
	}
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		return false, err
	}
	defer database.Close()
	for _, table := range []string{"service_heartbeats", "scheduler_workers"} {
		active, err := recentTableHeartbeat(ctx, database, table, now-maximumAge)
		if err != nil || active {
			return active, err
		}
	}
	return false, nil
}

func hasInvalidHeartbeatWindow(now, maximumAge float64) bool {
	return math.IsNaN(now) || math.IsInf(now, 0) || math.IsNaN(maximumAge) || math.IsInf(maximumAge, 0) || maximumAge < 0
}

func recentTableHeartbeat(ctx context.Context, database *sql.DB, table string, cutoff float64) (bool, error) {
	var exists bool
	if err := database.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?)", table).Scan(&exists); err != nil {
		return false, err
	}
	if !exists {
		return false, nil
	}
	clause := ""
	if table == "service_heartbeats" {
		clause = "service IN ('api','worker') AND "
	}
	if err := database.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM "+table+" WHERE "+clause+"heartbeat_at>=?)", cutoff).Scan(&exists); err != nil {
		return false, err
	}
	return exists, nil
}
