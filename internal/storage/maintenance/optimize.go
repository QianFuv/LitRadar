package maintenance

import (
	"context"
	"fmt"
	"log/slog"
	"math"
	"os"
	"path/filepath"
	"time"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

type hooks struct {
	beforeCopy                                                 func(string) error
	beforeValidation, afterRename, afterSwitch, beforeRollback func() error
}

func call(hook func() error) error {
	if hook != nil {
		return hook()
	}
	return nil
}

// Optimize replaces the index directory only after every copied database passes compatibility checks.
func Optimize(ctx context.Context, options Options) (Report, error) {
	now := time.Now().Unix()
	if now < 0 {
		now = 0
	}
	return optimize(ctx, options, now, hooks{})
}
func optimize(ctx context.Context, options Options, now int64, hook hooks) (Report, error) {
	empty := Report{Databases: []DatabaseReport{}}
	if !options.Confirmed {
		return empty, Failure{Code: "confirmation_required", Message: "index storage optimization requires --confirm-index-maintenance"}
	}
	paths, err := Paths(options.Config)
	if err != nil {
		return empty, err
	}
	if err := CheckInterrupted(options.Config); err != nil {
		return empty, err
	}
	data := filepath.Join(options.Config.ProjectRoot, "data")
	present, err := exists(data)
	if err != nil {
		return empty, err
	}
	if present {
		info, err := os.Lstat(data)
		if err != nil {
			return empty, err
		}
		if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return empty, invalid("project data target must be a regular directory")
		}
	}
	source, err := inspectDirectory(options.Config)
	if err != nil {
		return empty, err
	}
	if len(source.databases) == 0 {
		empty.Outcome = "noop"
		return empty, nil
	}
	if err := EnsureInactive(ctx, options.Config, now); err != nil {
		return empty, err
	}
	for index := range source.databases {
		version, err := sourceVersion(ctx, source.databases[index])
		if err != nil {
			return empty, err
		}
		source.databases[index].version = version
	}
	temporary := saturatingMultiply(source.bytes, 2)
	const overhead = 64 * 1024 * 1024
	if temporary > math.MaxUint64-overhead {
		temporary = math.MaxUint64
	} else {
		temporary += overhead
	}
	slog.Info("", "event", "storage.index_optimization.estimate", "component", "storage", "database_count", len(source.databases), "source_bytes", source.bytes, "temporary_bytes_required", temporary)
	if err := acquireMarker(paths, now); err != nil {
		if present, _ := exists(paths.Marker); present {
			return empty, withRecovery(err, paths)
		}
		return empty, err
	}
	report, err := runMarked(ctx, options.Config, paths, source, now, temporary, hook)
	if err != nil {
		sourcePresent, _ := exists(options.Config.IndexDir)
		rollbackPresent, _ := exists(paths.Rollback)
		known, isKnown := err.(Failure)
		if (!isKnown || known.Code != "rollback_failed") && sourcePresent && !rollbackPresent {
			_ = removeKnown(paths.Staging, options.Config.ProjectRoot, stagingName)
		}
		return empty, withRecovery(err, paths)
	}
	return report, nil
}
func runMarked(ctx context.Context, configuration config.Config, paths RecoveryPaths, source directorySnapshot, now int64, temporary uint64, hook hooks) (Report, error) {
	result := Report{Databases: []DatabaseReport{}}
	if err := EnsureInactive(ctx, configuration, now); err != nil {
		return result, err
	}
	for _, database := range source.databases {
		if err := validateSource(ctx, database); err != nil {
			return result, err
		}
	}
	if err := os.Mkdir(paths.Staging, 0777); err != nil {
		return result, err
	}
	for _, database := range source.databases {
		if hook.beforeCopy != nil {
			if err := hook.beforeCopy(database.name); err != nil {
				return result, err
			}
		}
		if err := buildDatabase(ctx, database, filepath.Join(paths.Staging, database.name)); err != nil {
			return result, err
		}
	}
	info, err := os.Stat(configuration.IndexDir)
	if err != nil {
		return result, err
	}
	if err := os.Chmod(paths.Staging, info.Mode()); err != nil {
		return result, err
	}
	if err := call(hook.beforeValidation); err != nil {
		return result, err
	}
	for _, database := range source.databases {
		report, err := validateRebuilt(ctx, database, filepath.Join(paths.Staging, database.name))
		if err != nil {
			return result, err
		}
		result.Databases = append(result.Databases, report)
	}
	current, err := inspectDirectory(configuration)
	if err != nil {
		return result, err
	}
	if err := unchanged(source, current); err != nil {
		return result, err
	}
	if err := EnsureInactive(ctx, configuration, now); err != nil {
		return result, err
	}
	if err := os.Rename(configuration.IndexDir, paths.Rollback); err != nil {
		return result, err
	}
	err = call(hook.afterRename)
	if err == nil {
		err = os.Rename(paths.Staging, configuration.IndexDir)
	}
	if err != nil {
		if rollbackError := restoreRename(configuration, paths, hook, err); rollbackError != nil {
			return result, rollbackError
		}
		return result, err
	}
	err = call(hook.afterSwitch)
	if err == nil {
		for _, database := range source.databases {
			database.path = filepath.Join(paths.Rollback, database.name)
			if _, err = validateRebuilt(ctx, database, filepath.Join(configuration.IndexDir, database.name)); err != nil {
				break
			}
		}
	}
	if err != nil {
		if rollbackError := rollbackSwitch(configuration, paths, hook, err); rollbackError != nil {
			return result, rollbackError
		}
		return result, err
	}
	if err := removeKnown(paths.Rollback, configuration.ProjectRoot, rollbackName); err != nil {
		return result, err
	}
	if err := os.Remove(paths.Marker); err != nil {
		return result, err
	}
	result.Outcome = "optimized"
	result.DatabaseCount = len(result.Databases)
	result.SourceBytes = source.bytes
	result.TemporaryBytesRequired = temporary
	for _, report := range result.Databases {
		result.OptimizedBytes += report.After.FileBytes
	}
	if result.SourceBytes > result.OptimizedBytes {
		result.ReclaimedBytes = result.SourceBytes - result.OptimizedBytes
	}
	return result, nil
}
func unchanged(original, current directorySnapshot) error {
	if len(original.databases) != len(current.databases) {
		return Failure{Code: "source_changed", Message: "index source changed during maintenance: index directory inventory"}
	}
	for index, first := range original.databases {
		second := current.databases[index]
		if first.name != second.name || first.size != second.size || !first.modified.Equal(second.modified) {
			return Failure{Code: "source_changed", Message: "index source changed during maintenance: " + first.name}
		}
	}
	return nil
}
func restoreRename(configuration config.Config, paths RecoveryPaths, hook hooks, primary error) error {
	if err := call(hook.beforeRollback); err != nil {
		return rollbackFailure(fmt.Sprintf("%v; rollback hook failed: %v", primary, err), paths)
	}
	if err := os.Rename(paths.Rollback, configuration.IndexDir); err != nil {
		return rollbackFailure(fmt.Sprintf("%v; original directory restore failed: %v", primary, err), paths)
	}
	return nil
}
func rollbackSwitch(configuration config.Config, paths RecoveryPaths, hook hooks, primary error) error {
	if err := call(hook.beforeRollback); err != nil {
		return rollbackFailure(fmt.Sprintf("%v; rollback hook failed: %v", primary, err), paths)
	}
	if err := os.Rename(configuration.IndexDir, paths.Staging); err != nil {
		return rollbackFailure(fmt.Sprintf("%v; failed candidate retention failed: %v", primary, err), paths)
	}
	if err := os.Rename(paths.Rollback, configuration.IndexDir); err != nil {
		candidateError := os.Rename(paths.Staging, configuration.IndexDir)
		return rollbackFailure(fmt.Sprintf("%v; original directory restore failed: %v; candidate restore result: %v", primary, err, candidateError), paths)
	}
	return nil
}
func removeKnown(filename, projectRoot, expectedName string) error {
	present, err := exists(filename)
	if err != nil || !present {
		return err
	}
	parent, err := filepath.Abs(filepath.Join(projectRoot, "data"))
	if err != nil {
		return err
	}
	if filepath.Dir(filename) != parent || filepath.Base(filename) != expectedName {
		return invalid("maintenance cleanup path escaped the project data directory")
	}
	info, err := os.Lstat(parent)
	if err != nil {
		return err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return invalid("maintenance cleanup parent must be a regular directory")
	}
	info, err = os.Lstat(filename)
	if err != nil {
		return err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return invalid("maintenance cleanup target must be a regular directory")
	}
	return os.RemoveAll(filename)
}
