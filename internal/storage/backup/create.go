package backup

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	native "github.com/mattn/go-sqlite3"
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
func copyDatabase(ctx context.Context, source, destination string) error {
	input, err := storage.Open(source, true, 1)
	if err != nil {
		return err
	}
	defer input.Close()
	output, err := storage.OpenPlain(destination)
	if err != nil {
		return err
	}
	defer output.Close()
	sourceConnection, err := input.Conn(ctx)
	if err != nil {
		return err
	}
	defer sourceConnection.Close()
	destinationConnection, err := output.Conn(ctx)
	if err != nil {
		return err
	}
	defer destinationConnection.Close()
	err = sourceConnection.Raw(func(source any) error {
		return destinationConnection.Raw(func(destination any) error {
			backup, err := destination.(*native.SQLiteConn).Backup("main", source.(*native.SQLiteConn), "main")
			if err != nil {
				return err
			}
			var stepError error
			for {
				if stepError = ctx.Err(); stepError != nil {
					break
				}
				var done bool
				done, stepError = backup.Step(128)
				if stepError != nil || done {
					break
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
		})
	})
	if err != nil {
		return err
	}
	hasSimple, err := search.UsesSimple(ctx, destinationConnection)
	if err != nil {
		return err
	}
	if hasSimple {
		if err := storage.LoadSimple(destinationConnection); err != nil {
			return err
		}
	}
	_, err = destinationConnection.ExecContext(ctx, "PRAGMA journal_mode=DELETE")
	return err
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
func writeManifest(directory string, manifest Manifest) error {
	raw, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	file, err := os.OpenFile(filepath.Join(directory, "manifest.json"), os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0666)
	if err != nil {
		return err
	}
	defer file.Close()
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
	defer os.RemoveAll(stage)
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
func withAuthGate(ctx context.Context, filename string, read func(*sql.Conn) error) error {
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	if err := read(connection); err != nil {
		return err
	}
	_, err = connection.ExecContext(ctx, "ROLLBACK")
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
	stage, err := os.MkdirTemp(parent, ".litradar-backup-")
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
	if err := os.MkdirAll(filepath.Join(stage, "index"), 0777); err != nil {
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
	if err := os.Rename(stage, output); err != nil {
		return Manifest{}, err
	}
	return manifest, nil
}
