package backup

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
	platform "github.com/QianFuv/LitRadar/apps/server/internal/platform/sqlite"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	native "modernc.org/sqlite"
	sqlite3 "modernc.org/sqlite/lib"
)

func backupDatabase(ctx context.Context, source, destination string) error {
	info, err := os.Lstat(source)
	if err != nil {
		return failure("input", "SQLite backup source does not exist")
	}
	if !info.Mode().IsRegular() {
		return failure("input", "SQLite backup source is not a regular file")
	}
	if err := os.MkdirAll(filepath.Dir(destination), 0777); err != nil {
		return err
	}
	if err := copyDatabase(ctx, source, destination); err != nil {
		return err
	}
	for _, suffix := range []string{"-wal", "-shm", "-journal"} {
		if err := removeFile(destination + suffix); err != nil {
			return err
		}
	}
	_, err = databaseVersion(ctx, destination)
	return err
}

// copyDatabase creates a standalone online snapshot and finalizes its rollback journal.
func copyDatabase(ctx context.Context, source, destination string) (resultError error) {
	input, err := storage.Open(source, true, 1)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := input.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close input: %w", closeError))
		}
	}()
	sourceConnection, err := input.Conn(ctx)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := sourceConnection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close sourceConnection: %w", closeError))
		}
	}()
	destinationUri, err := backupDestinationUri(destination)
	if err != nil {
		return err
	}
	err = sourceConnection.Raw(func(source any) error {
		backup, err := source.(interface {
			NewBackup(string) (*native.Backup, error)
		}).NewBackup(destinationUri)
		if err != nil {
			return err
		}
		return completeDatabaseBackup(ctx, backup)
	})
	if err != nil {
		return err
	}
	output, err := storage.OpenPlain(destination)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := output.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close output: %w", closeError))
		}
	}()
	_, err = output.ExecContext(ctx, "PRAGMA journal_mode=DELETE")
	return err
}

// backupDestinationUri retains the plain destination policy on modernc's internally owned connection.
func backupDestinationUri(filename string) (string, error) {
	dsn, err := platform.FileUri(filename, "rwc")
	if err != nil {
		return "", err
	}
	location, err := url.Parse(dsn)
	if err != nil {
		return "", err
	}
	query := location.Query()
	query.Del("_journal_mode")
	query.Del("_foreign_keys")
	query.Set("_busy_timeout", "5000")
	query.Set("_synchronous", "FULL")
	location.RawQuery = query.Encode()
	return location.String(), nil
}

// completeDatabaseBackup waits on stalled steps, finishes every snapshot, and preserves the primary failure.
func completeDatabaseBackup(ctx context.Context, backup *native.Backup) error {
	copiedPages := 0
	var stepError error
	for {
		if stepError = ctx.Err(); stepError != nil {
			break
		}
		var more bool
		more, stepError = backup.Step(2048)
		var failure *native.Error
		if errors.As(stepError, &failure) && (failure.Code()&255 == sqlite3.SQLITE_BUSY || failure.Code()&255 == sqlite3.SQLITE_LOCKED) {
			stepError = nil
			more = true
		}
		if stepError != nil || !more {
			break
		}
		previousCopiedPages := copiedPages
		copiedPages = backup.PageCount() - backup.Remaining()
		if copiedPages > previousCopiedPages {
			continue
		}
		timer := time.NewTimer(25 * time.Millisecond)
		select {
		case <-timer.C:
		case <-ctx.Done():
			timer.Stop()
			stepError = ctx.Err()
		}
		if stepError != nil {
			break
		}
	}
	finishError := backup.Finish()
	if stepError != nil {
		return stepError
	}
	return finishError
}

func databaseComponent(ctx context.Context, kind, relative, filename string) (Component, error) {
	info, err := os.Stat(filename)
	if err != nil {
		return Component{}, err
	}
	digest, err := hashFile(filename)
	if err != nil {
		return Component{}, err
	}
	version, err := databaseVersion(ctx, filename)
	if err != nil {
		return Component{}, err
	}
	return Component{kind, relative, uint64(info.Size()), digest, &version}, nil
}
func writeManifest(directory string, manifest Manifest) (resultError error) {
	raw, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	root, err := os.OpenRoot(directory)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	file, err := root.OpenFile("manifest.json", os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0666)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := file.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close file: %w", closeError))
		}
	}()
	if _, err := file.Write(append(raw, '\n')); err != nil {
		return err
	}
	return file.Sync()
}
func validateOutput(options CreateOptions) error {
	parent, err := canonical(filepath.Dir(options.OutputDir))
	if err != nil {
		return err
	}
	output := filepath.Join(parent, filepath.Base(options.OutputDir))
	sources := []string{options.Config.MetaDir}
	if options.IncludeIndexDatabases {
		sources = append(sources, options.Config.IndexDir)
	}
	if options.IncludePushState {
		for _, name := range []string{"push_state", "folder_push_state"} {
			sources = append(sources, filepath.Join(options.Config.ProjectRoot, "data", name))
		}
	}
	for _, source := range sources {
		if !fileExists(source) {
			continue
		}
		resolved, err := canonical(source)
		if err != nil {
			return err
		}
		if within(output, resolved) {
			return failure("input", "output directory must be outside included source directories")
		}
	}
	return nil
}

// Create captures SQLite through its online backup API and verifies all files before publishing the directory.
func Create(ctx context.Context, options CreateOptions) (Manifest, error) {
	return create(ctx, options, nil)
}
func create(ctx context.Context, options CreateOptions, afterMetaCopy func(string) error) (Manifest, error) {
	empty := Manifest{}
	stage, err := prepareBackupStage(options)
	if err != nil {
		return empty, err
	}
	stageParent, err := os.OpenRoot(filepath.Dir(stage))
	if err != nil {
		return empty, err
	}
	defer func() { _ = stageParent.Close() }()
	defer func() {
		if err := stageParent.RemoveAll(filepath.Base(stage)); err != nil {
			slog.Warn("cleanup.failed", "operation", "os.RemoveAll", "error", err)
		}
	}()
	components, err := copyInitialBackupComponents(ctx, options, stage, afterMetaCopy)
	if err != nil {
		return empty, err
	}
	if options.IncludeIndexDatabases {
		items, err := copyBackupIndexes(ctx, options, stage)
		if err != nil {
			return empty, err
		}
		components = append(components, items...)
	}
	if options.IncludePushState {
		items, err := copyBackupPushState(options, stage)
		if err != nil {
			return empty, err
		}
		components = append(components, items...)
	}
	slices.SortFunc(components, func(first, second Component) int { return strings.Compare(first.Path, second.Path) })
	now := time.Now()
	if now.Unix() < 0 {
		return empty, failure("input", "system time is before the Unix epoch")
	}
	manifest := Manifest{"litradar-backup", FormatVersion, float64(now.Unix()) + float64(now.Nanosecond())/1e9, Selection{true, options.IncludeIndexDatabases, options.IncludePushState}, components}
	return publishBackupStage(ctx, options.OutputDir, stage, manifest)
}
func withAuthGate(ctx context.Context, filename string, read func(*sql.Conn) error) (resultError error) {
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	connection, err := database.Conn(ctx)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := connection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return err
	}
	hasTransaction := true
	defer func() {
		if hasTransaction {
			if _, rollbackError := connection.ExecContext(context.Background(), "ROLLBACK"); rollbackError != nil {
				slog.Error("storage.rollback.failed", "error", rollbackError)
			}
		}
	}()
	if err := read(connection); err != nil {
		return err
	}
	_, err = connection.ExecContext(ctx, "ROLLBACK")
	hasTransaction = err != nil
	return err
}

func prepareBackupStage(options CreateOptions) (string, error) {
	if fileExists(options.OutputDir) {
		return "", failure("input", "output directory already exists")
	}
	info, err := os.Stat(options.AuthDbPath)
	if err != nil || !info.Mode().IsRegular() {
		return "", failure("input", "auth database does not exist")
	}
	parent := filepath.Dir(options.OutputDir)
	if err := os.MkdirAll(parent, 0777); err != nil {
		return "", err
	}
	if err := validateOutput(options); err != nil {
		return "", err
	}
	root, err := os.OpenRoot(parent)
	if err != nil {
		return "", err
	}
	defer func() { _ = root.Close() }()
	stage, err := rootfile.MkdirTemp(root, ".litradar-backup-")
	if err != nil {
		return "", err
	}
	return stage, nil
}

func copyInitialBackupComponents(ctx context.Context, options CreateOptions, stage string, afterMetaCopy func(string) error) ([]Component, error) {
	components := []Component{}
	err := withAuthGate(ctx, options.AuthDbPath, func(_ *sql.Conn) error {
		destination := filepath.Join(stage, "auth.sqlite")
		if err := backupDatabase(ctx, options.AuthDbPath, destination); err != nil {
			return err
		}
		component, err := databaseComponent(ctx, "auth_database", "auth.sqlite", destination)
		if err != nil {
			return err
		}
		components = append(components, component)
		metadata, err := copyGroup(options.Config.MetaDir, stage, "meta", "metadata", "metadata", afterMetaCopy)
		if err != nil {
			return err
		}
		components = append(components, metadata...)
		return nil
	})

	return components, err
}

func copyBackupIndexes(ctx context.Context, options CreateOptions, stage string) ([]Component, error) {
	components := []Component{}
	root, err := os.OpenRoot(stage)
	if err != nil {
		return nil, err
	}
	defer func() { _ = root.Close() }()
	if err := root.MkdirAll("index", 0777); err != nil {
		return nil, err
	}
	files, err := options.Config.ListIndexDatabases()
	if err != nil {
		return nil, err
	}
	for _, source := range files {
		name := filepath.Base(source)
		destination := filepath.Join(stage, "index", name)
		if err := backupDatabase(ctx, source, destination); err != nil {
			return nil, err
		}
		component, err := databaseComponent(ctx, "index_database", "index/"+name, destination)
		if err != nil {
			return nil, err
		}
		components = append(components, component)
	}
	return components, nil
}

func copyBackupPushState(options CreateOptions, stage string) ([]Component, error) {
	components := []Component{}
	for _, name := range []string{"push_state", "folder_push_state"} {
		items, err := copyGroup(filepath.Join(options.Config.ProjectRoot, "data", name), stage, name, "push_state", "push-state", nil)
		if err != nil {
			return nil, err
		}
		components = append(components, items...)
	}
	return components, nil
}

func publishBackupStage(ctx context.Context, output, stage string, manifest Manifest) (Manifest, error) {
	if err := writeManifest(stage, manifest); err != nil {
		return Manifest{}, err
	}
	manifest, err := Verify(ctx, stage)
	if err != nil {
		return Manifest{}, err
	}
	root, name, err := rootfile.Parent(output)
	if err != nil {
		return Manifest{}, err
	}
	defer func() { _ = root.Close() }()
	stageName, err := filepath.Rel(root.Name(), stage)
	if err != nil {
		return Manifest{}, err
	}
	if err := root.Rename(stageName, name); err != nil {
		return Manifest{}, err
	}
	return manifest, nil
}
