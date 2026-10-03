package maintenance

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/storage/backup"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/index"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// Paths resolves recovery locations outside the replaceable index directory.
func Paths(configuration config.Config) (RecoveryPaths, error) {
	data, err := filepath.Abs(filepath.Join(configuration.ProjectRoot, "data"))
	if err != nil {
		return RecoveryPaths{}, err
	}
	return RecoveryPaths{filepath.Join(data, markerName), filepath.Join(data, stagingName), filepath.Join(data, rollbackName)}, nil
}

// CheckInterrupted blocks startup on any recovery artifact, including malformed markers and dangling links.
func CheckInterrupted(configuration config.Config) error {
	paths, err := Paths(configuration)
	if err != nil {
		return err
	}
	for _, filename := range []string{paths.Marker, paths.Staging, paths.Rollback} {
		present, err := exists(filename)
		if err != nil {
			return err
		}
		if present {
			return interrupted(paths)
		}
	}
	return nil
}

// EnsureInactive checks service heartbeats and control leases without acquiring an indexing lease.
func EnsureInactive(ctx context.Context, configuration config.Config, now int64) error {
	active, err := backup.HasRecentHeartbeat(ctx, configuration.AuthDbPath, float64(now), backup.ActiveHeartbeatMaxAge)
	if err != nil {
		return validation("auth.sqlite", fmt.Sprintf("activity gate: %v", err))
	}
	if active {
		return Failure{Code: "active_target", Message: "index storage optimization refused because a recent service heartbeat marks the target active"}
	}
	present, err := exists(configuration.IndexControlDir)
	if err != nil || !present {
		return err
	}
	info, err := os.Lstat(configuration.IndexControlDir)
	if err != nil {
		return err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return invalid("index control target must be a regular directory")
	}
	entries, err := os.ReadDir(configuration.IndexControlDir)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		name := entry.Name()
		if !strings.HasSuffix(name, ".sqlite") || name == ".sqlite" {
			continue
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		if !info.Mode().IsRegular() {
			return invalid("index control databases must be regular files")
		}
		if !utf8.ValidString(name) {
			return invalid("index control filenames must be valid UTF-8")
		}
		if err := checkLeases(ctx, filepath.Join(configuration.IndexControlDir, name), name, now); err != nil {
			return err
		}
	}
	return nil
}
func checkLeases(ctx context.Context, filename, name string, now int64) error {
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	for _, lease := range []struct{ table, kind, query string }{
		{"index_batch_lease", "index batch lease", "SELECT expires_at FROM index_batch_lease WHERE lease_key=1 AND expires_at>?"},
		{"provider_leases", "Provider lease", "SELECT MAX(expires_at) FROM provider_leases WHERE expires_at>?"},
	} {
		var present bool
		if err := database.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?)", lease.table).Scan(&present); err != nil {
			return err
		}
		if !present {
			continue
		}
		rows, err := database.QueryContext(ctx, lease.query, now)
		if err != nil {
			return err
		}
		var expiration storage.OptionalInteger
		if rows.Next() {
			err = rows.Scan(&expiration)
		}
		rowError := rows.Err()
		rows.Close()
		if err != nil {
			return err
		}
		if rowError != nil {
			return rowError
		}
		if expiration.Value != nil {
			return Failure{Code: "active_lease", Message: fmt.Sprintf("index storage optimization refused because %s in %s remains leased until %d", lease.kind, name, *expiration.Value)}
		}
	}
	return nil
}
func acquireMarker(paths RecoveryPaths, now int64) error {
	if err := os.MkdirAll(filepath.Dir(paths.Marker), 0777); err != nil {
		return err
	}
	file, err := os.OpenFile(paths.Marker, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0666)
	if os.IsExist(err) {
		return interrupted(paths)
	}
	if err != nil {
		return err
	}
	defer file.Close()
	raw, err := json.MarshalIndent(map[string]any{"format": "litradar-index-maintenance", "version": 1, "started_at_epoch_seconds": now, "process_id": os.Getpid(), "staging": paths.Staging, "rollback": paths.Rollback}, "", "  ")
	if err != nil {
		return invalid(fmt.Sprintf("maintenance marker serialization failed: %v", err))
	}
	if _, err := file.Write(append(raw, '\n')); err != nil {
		return err
	}
	return file.Sync()
}

type sourceDatabase struct {
	name, path string
	size       uint64
	modified   time.Time
	version    int64
}
type directorySnapshot struct {
	databases []sourceDatabase
	bytes     uint64
}

func inspectDirectory(configuration config.Config) (directorySnapshot, error) {
	result := directorySnapshot{databases: []sourceDatabase{}}
	present, err := exists(configuration.IndexDir)
	if err != nil || !present {
		return result, err
	}
	info, err := os.Lstat(configuration.IndexDir)
	if err != nil {
		return result, err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return result, invalid("index target must be a regular directory")
	}
	if runtime.GOOS != "windows" && info.Mode().Perm()&0222 == 0 {
		return result, invalid("index target must be writable for directory replacement")
	}
	entries, err := os.ReadDir(configuration.IndexDir)
	if err != nil {
		return result, err
	}
	type sidecar struct {
		name, database, kind string
		size                 int64
	}
	sidecars := []sidecar{}
	for _, entry := range entries {
		info, err := entry.Info()
		if err != nil {
			return result, err
		}
		if !info.Mode().IsRegular() {
			return result, invalid("index directory entries must be regular files")
		}
		if info.Mode().Perm()&0222 == 0 {
			return result, invalid("index database and sidecar files must be writable for replacement")
		}
		name := entry.Name()
		if !utf8.ValidString(name) {
			return result, invalid("index filenames must be valid UTF-8")
		}
		if strings.HasSuffix(name, ".sqlite") {
			result.databases = append(result.databases, sourceDatabase{name, filepath.Join(configuration.IndexDir, name), uint64(info.Size()), info.ModTime(), 0})
			result.bytes += uint64(info.Size())
			continue
		}
		isSidecar := false
		for _, kind := range []string{"wal", "shm", "journal"} {
			suffix := "-" + kind
			if strings.HasSuffix(name, suffix) && strings.HasSuffix(strings.TrimSuffix(name, suffix), ".sqlite") {
				sidecars = append(sidecars, sidecar{name, strings.TrimSuffix(name, suffix), kind, info.Size()})
				isSidecar = true
				break
			}
		}
		if !isSidecar {
			return result, invalid("unexpected file exists beside index databases")
		}
	}
	for _, sidecar := range sidecars {
		found := false
		for _, database := range result.databases {
			if database.name == sidecar.database {
				found = true
				break
			}
		}
		if !found {
			return result, invalid("orphaned SQLite sidecar " + sidecar.name)
		}
		if sidecar.kind != "shm" && sidecar.size != 0 {
			return result, invalid(fmt.Sprintf("non-empty SQLite %s sidecar %s", sidecar.kind, sidecar.name))
		}
	}
	return result, nil
}
func sourceVersion(ctx context.Context, source sourceDatabase) (int64, error) {
	version := int64(0)
	if source.size > 0 {
		database, err := storage.Open(source.path, true, 1)
		if err != nil {
			return 0, err
		}
		var stored storage.Integer
		err = database.QueryRowContext(ctx, "PRAGMA user_version").Scan(&stored)
		database.Close()
		if err != nil {
			return 0, err
		}
		version = int64(stored)
	}
	if version < 6 || version > 9 {
		return 0, Failure{Code: "unsupported_schema", Message: fmt.Sprintf("index database %s uses unsupported schema version %d; supported versions are 6 through 9", source.name, version)}
	}
	if _, err := migration.Preflight(ctx, source.path); err != nil {
		return 0, validation(source.name, fmt.Sprintf("exact schema preflight: %v", err))
	}
	return version, nil
}
