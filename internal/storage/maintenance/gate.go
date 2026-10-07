package maintenance

import (
	"context"
	"database/sql"
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
	entries, err := readControlEntries(configuration.IndexControlDir)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if err := checkControlDatabase(ctx, configuration.IndexControlDir, entry, now); err != nil {
			return err
		}
	}
	return nil
}

// readControlEntries rejects links before reading the control directory.
func readControlEntries(directory string) ([]os.DirEntry, error) {
	info, err := os.Lstat(directory)
	if err != nil {
		return nil, err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return nil, invalid("index control target must be a regular directory")
	}
	return os.ReadDir(directory)
}

// checkControlDatabase skips unrelated names before admitting and checking one control file.
func checkControlDatabase(ctx context.Context, directory string, entry os.DirEntry, now int64) error {
	name := entry.Name()
	if !strings.HasSuffix(name, ".sqlite") || name == ".sqlite" {
		return nil
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
	return checkLeases(ctx, filepath.Join(directory, name), name, now)
}

// checkLeases admits batch and provider leases in their original priority order.
func checkLeases(ctx context.Context, filename, name string, now int64) error {
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	for _, lease := range []leaseCheck{
		{"index_batch_lease", "index batch lease", "SELECT expires_at FROM index_batch_lease WHERE lease_key=1 AND expires_at>?"},
		{"provider_leases", "Provider lease", "SELECT MAX(expires_at) FROM provider_leases WHERE expires_at>?"},
	} {
		if err := checkLease(ctx, database, lease, name, now); err != nil {
			return err
		}
	}
	return nil
}

type leaseCheck struct{ table, kind, query string }

// checkLease closes queried rows before classifying scan, iteration or active-lease results.
func checkLease(ctx context.Context, database *sql.DB, lease leaseCheck, name string, now int64) error {
	var present bool
	if err := database.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?)", lease.table).Scan(&present); err != nil {
		return err
	}
	if !present {
		return nil
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

// inspectDirectory retains admitted databases on entry or deferred sidecar validation failure.
func inspectDirectory(configuration config.Config) (directorySnapshot, error) {
	result := directorySnapshot{databases: []sourceDatabase{}}
	present, err := exists(configuration.IndexDir)
	if err != nil || !present {
		return result, err
	}
	entries, err := readIndexEntries(configuration.IndexDir)
	if err != nil {
		return result, err
	}
	sidecars, err := inspectIndexEntries(configuration.IndexDir, entries, &result)
	if err != nil {
		return result, err
	}
	if err := validateIndexSidecars(sidecars, result.databases); err != nil {
		return result, err
	}
	return result, nil
}

type indexSidecar struct {
	name, database, kind string
	size                 int64
}

// readIndexEntries preserves platform-specific directory permission admission before enumeration.
func readIndexEntries(directory string) ([]os.DirEntry, error) {
	info, err := os.Lstat(directory)
	if err != nil {
		return nil, err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return nil, invalid("index target must be a regular directory")
	}
	if runtime.GOOS != "windows" && info.Mode().Perm()&0222 == 0 {
		return nil, invalid("index target must be writable for directory replacement")
	}
	return os.ReadDir(directory)
}

// inspectIndexEntries admits every entry before validating collected sidecar relationships.
func inspectIndexEntries(directory string, entries []os.DirEntry, result *directorySnapshot) ([]indexSidecar, error) {
	sidecars := []indexSidecar{}
	for _, entry := range entries {
		info, err := admitIndexEntry(entry)
		if err != nil {
			return sidecars, err
		}
		name := entry.Name()
		if strings.HasSuffix(name, ".sqlite") {
			result.databases = append(result.databases, sourceDatabase{name, filepath.Join(directory, name), uint64(info.Size()), info.ModTime(), 0})
			result.bytes += uint64(info.Size())
			continue
		}
		sidecar, isSidecar := identifyIndexSidecar(name, info.Size())
		if !isSidecar {
			return sidecars, invalid("unexpected file exists beside index databases")
		}
		sidecars = append(sidecars, sidecar)
	}
	return sidecars, nil
}

// admitIndexEntry checks metadata, file type, permission bits and filename in that order.
func admitIndexEntry(entry os.DirEntry) (os.FileInfo, error) {
	info, err := entry.Info()
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() {
		return nil, invalid("index directory entries must be regular files")
	}
	if info.Mode().Perm()&0222 == 0 {
		return nil, invalid("index database and sidecar files must be writable for replacement")
	}
	if !utf8.ValidString(entry.Name()) {
		return nil, invalid("index filenames must be valid UTF-8")
	}
	return info, nil
}

// identifyIndexSidecar recognizes the ordered SQLite sidecar suffixes.
func identifyIndexSidecar(name string, size int64) (indexSidecar, bool) {
	for _, kind := range []string{"wal", "shm", "journal"} {
		suffix := "-" + kind
		if strings.HasSuffix(name, suffix) && strings.HasSuffix(strings.TrimSuffix(name, suffix), ".sqlite") {
			return indexSidecar{name, strings.TrimSuffix(name, suffix), kind, size}, true
		}
	}
	return indexSidecar{}, false
}

// validateIndexSidecars requires a parent before checking sidecar content.
func validateIndexSidecars(sidecars []indexSidecar, databases []sourceDatabase) error {
	for _, sidecar := range sidecars {
		if !hasSidecarDatabase(sidecar.database, databases) {
			return invalid("orphaned SQLite sidecar " + sidecar.name)
		}
		if sidecar.kind != "shm" && sidecar.size != 0 {
			return invalid(fmt.Sprintf("non-empty SQLite %s sidecar %s", sidecar.kind, sidecar.name))
		}
	}
	return nil
}

// hasSidecarDatabase searches the admitted database inventory in encounter order.
func hasSidecarDatabase(name string, databases []sourceDatabase) bool {
	for _, database := range databases {
		if database.name == name {
			return true
		}
	}
	return false
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
