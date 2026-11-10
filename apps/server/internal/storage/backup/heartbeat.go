package backup

import (
	"context"
	"database/sql"
	"math"
	"os"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/backupdb"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

var heartbeatQueries backupdb.Queries

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
	if _, err := heartbeatQueries.RecordHeartbeat(ctx, database, backupdb.RecordHeartbeatParams{Service: string(service), InstanceID: instance, StartedAt: at, HeartbeatAt: at}); err != nil {
		return err
	}
	_, err = heartbeatQueries.PruneHeartbeats(ctx, database, at-604800)
	return err
}

// DeleteHeartbeat removes only the requested service instance during graceful shutdown.
func DeleteHeartbeat(ctx context.Context, filename string, service ServiceKind, instance string) error {
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	_, err = heartbeatQueries.DeleteHeartbeat(ctx, database, backupdb.DeleteHeartbeatParams{Service: string(service), InstanceID: instance})
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
