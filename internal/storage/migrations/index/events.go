package index

import (
	"context"
	"errors"
	"log/slog"
	"os"
	"time"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
)

func event(ctx context.Context, name string, err error, fields ...any) {
	level := slog.LevelInfo
	if err != nil {
		level = slog.LevelWarn
		fields = append(fields, "error_kind", errorKind(err))
	}
	attributes := []any{"event", name, "component", "storage", "database_kind", "index", "target_version", indexschema.Version}
	slog.Log(ctx, level, name, append(attributes, fields...)...)
}

func errorKind(err error) string {
	var unsupported UnsupportedVersion
	var rebuild RebuildRequired
	var file *os.PathError
	switch {
	case errors.As(err, &unsupported):
		return "unsupported_schema_version"
	case errors.As(err, &rebuild):
		return "index_rebuild_required"
	case errors.Is(err, ErrIdentityState):
		return "invalid_index_identity_state"
	case errors.Is(err, ErrIdentityConflict):
		return "index_identity_conflict"
	case errors.As(err, &file):
		return "io"
	default:
		return "sqlite"
	}
}

func errorVersion(err error) int {
	var unsupported UnsupportedVersion
	var rebuild RebuildRequired
	switch {
	case errors.As(err, &unsupported):
		return unsupported.Found
	case errors.As(err, &rebuild):
		return rebuild.Found
	case errors.Is(err, ErrIdentityState), errors.Is(err, ErrIdentityConflict):
		return 4
	default:
		return -1
	}
}

func loggedMigration(ctx context.Context, filename string) (Summary, error) {
	started := time.Now()
	event(ctx, "storage.migration.started", nil)
	summary, err := migrate(ctx, filename)
	fields := []any{"duration_ms", time.Since(started).Milliseconds()}
	if err != nil {
		event(ctx, "storage.migration.failed", err, append(fields, "database_version", errorVersion(err))...)
	} else {
		event(ctx, "storage.migration.completed", nil, append(fields, "from_version", summary.FromVersion, "to_version", summary.ToVersion, "applied_count", max(0, summary.ToVersion-summary.FromVersion))...)
	}
	return summary, err
}

func loggedPreflight(ctx context.Context, filename string) (string, error) {
	started := time.Now()
	event(ctx, "storage.index_preflight.started", nil)
	scope, err := preflight(ctx, filename)
	fields := []any{"duration_ms", time.Since(started).Milliseconds()}
	if err != nil {
		event(ctx, "storage.index_preflight.failed", err, append(fields, "database_version", errorVersion(err))...)
	} else {
		event(ctx, "storage.index_preflight.completed", nil, append(fields, "validation_scope", scope)...)
	}
	return scope, err
}

// MigrateExisting upgrades each discovered content database in deterministic path order.
func MigrateExisting(ctx context.Context, configuration config.Config) error {
	return runBatch(ctx, configuration, false)
}

// PreflightExisting preserves supported content versions during ordinary startup.
func PreflightExisting(ctx context.Context, configuration config.Config) error {
	return runBatch(ctx, configuration, true)
}

func runBatch(ctx context.Context, configuration config.Config, isPreflight bool) error {
	started := time.Now()
	prefix := "storage.migration.batch"
	if isPreflight {
		prefix = "storage.index_preflight.batch"
	}
	event(ctx, prefix+".started", nil)
	paths, err := configuration.ListIndexDatabases()
	completed := 0
	for _, filename := range paths {
		if err != nil {
			break
		}
		if isPreflight {
			_, err = Preflight(ctx, filename)
		} else {
			_, err = Migrate(ctx, filename)
		}
		if err == nil {
			completed++
		}
	}
	fields := []any{"discovered_count", len(paths), "completed_count", completed, "duration_ms", time.Since(started).Milliseconds()}
	if err != nil {
		event(ctx, prefix+".failed", err, fields...)
	} else {
		event(ctx, prefix+".completed", nil, fields...)
	}
	return err
}
