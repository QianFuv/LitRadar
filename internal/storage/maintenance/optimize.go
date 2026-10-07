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

// optimize admits the complete source before creating the durable recovery marker.
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
	source, err := inspectOptimizationSource(options.Config)
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
	if err := identifySourceVersions(ctx, &source); err != nil {
		return empty, err
	}
	temporary := temporaryBytes(source.bytes)
	slog.InfoContext(ctx, "", "event", "storage.index_optimization.estimate", "component", "storage", "database_count", len(source.databases), "source_bytes", source.bytes, "temporary_bytes_required", temporary)
	if err := markOptimization(paths, now); err != nil {
		return empty, err
	}
	report, err := runMarked(ctx, options.Config, paths, source, now, temporary, hook)
	if err != nil {
		cleanFailedStaging(options.Config, paths, err)
		return empty, withRecovery(err, paths)
	}
	return report, nil
}

// inspectOptimizationSource admits the project data directory before its index inventory.
func inspectOptimizationSource(configuration config.Config) (directorySnapshot, error) {
	data := filepath.Join(configuration.ProjectRoot, "data")
	present, err := exists(data)
	if err != nil {
		return directorySnapshot{}, err
	}
	if present {
		info, err := os.Lstat(data)
		if err != nil {
			return directorySnapshot{}, err
		}
		if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return directorySnapshot{}, invalid("project data target must be a regular directory")
		}
	}
	return inspectDirectory(configuration)
}

// identifySourceVersions records each admitted source version in inventory order.
func identifySourceVersions(ctx context.Context, source *directorySnapshot) error {
	for index := range source.databases {
		version, err := sourceVersion(ctx, source.databases[index])
		if err != nil {
			return err
		}
		source.databases[index].version = version
	}
	return nil
}

// temporaryBytes saturates the doubled source size plus the fixed staging overhead.
func temporaryBytes(sourceBytes uint64) uint64 {
	temporary := saturatingMultiply(sourceBytes, 2)
	const overhead = 64 * 1024 * 1024
	if temporary > math.MaxUint64-overhead {
		return math.MaxUint64
	}
	return temporary + overhead
}

// markOptimization adds recovery paths only when marker acquisition leaves an artifact.
func markOptimization(paths RecoveryPaths, now int64) error {
	if err := acquireMarker(paths, now); err != nil {
		if present, _ := exists(paths.Marker); present {
			return withRecovery(err, paths)
		}
		return err
	}
	return nil
}

// cleanFailedStaging removes candidates only when the original directory remains available.
func cleanFailedStaging(configuration config.Config, paths RecoveryPaths, err error) {
	sourcePresent, _ := exists(configuration.IndexDir)
	rollbackPresent, _ := exists(paths.Rollback)
	known, isKnown := err.(Failure)
	if (!isKnown || known.Code != "rollback_failed") && sourcePresent && !rollbackPresent {
		_ = removeKnown(paths.Staging, configuration.ProjectRoot, stagingName)
	}
}

// runMarked retains completed database reports while advancing the guarded replacement.
func runMarked(ctx context.Context, configuration config.Config, paths RecoveryPaths, source directorySnapshot, now int64, temporary uint64, hook hooks) (Report, error) {
	result := Report{Databases: []DatabaseReport{}}
	if err := prepareOptimizationStaging(ctx, configuration, paths, source, now, hook); err != nil {
		return result, err
	}
	if err := validateOptimizationStaging(ctx, paths, source, hook, &result); err != nil {
		return result, err
	}
	if err := admitOptimizationSwitch(ctx, configuration, source, now); err != nil {
		return result, err
	}
	if err := switchOptimizationDirectories(ctx, configuration, paths, source, hook); err != nil {
		return result, err
	}
	if err := removeKnown(paths.Rollback, configuration.ProjectRoot, rollbackName); err != nil {
		return result, err
	}
	if err := os.Remove(paths.Marker); err != nil {
		return result, err
	}
	finishOptimizationReport(&result, source.bytes, temporary)
	return result, nil
}

// prepareOptimizationStaging validates every source before creating and filling staging.
func prepareOptimizationStaging(ctx context.Context, configuration config.Config, paths RecoveryPaths, source directorySnapshot, now int64, hook hooks) error {
	if err := EnsureInactive(ctx, configuration, now); err != nil {
		return err
	}
	for _, database := range source.databases {
		if err := validateSource(ctx, database); err != nil {
			return err
		}
	}
	if err := os.Mkdir(paths.Staging, 0777); err != nil {
		return err
	}
	if err := buildOptimizationStaging(ctx, paths, source, hook); err != nil {
		return err
	}
	info, err := os.Stat(configuration.IndexDir)
	if err != nil {
		return err
	}
	return os.Chmod(paths.Staging, info.Mode())
}

// buildOptimizationStaging invokes each copy hook before building its corresponding database.
func buildOptimizationStaging(ctx context.Context, paths RecoveryPaths, source directorySnapshot, hook hooks) error {
	for _, database := range source.databases {
		if hook.beforeCopy != nil {
			if err := hook.beforeCopy(database.name); err != nil {
				return err
			}
		}
		if err := buildDatabase(ctx, database, filepath.Join(paths.Staging, database.name)); err != nil {
			return err
		}
	}
	return nil
}

// validateOptimizationStaging appends reports only after each candidate passes all checks.
func validateOptimizationStaging(ctx context.Context, paths RecoveryPaths, source directorySnapshot, hook hooks, result *Report) error {
	if err := call(hook.beforeValidation); err != nil {
		return err
	}
	for _, database := range source.databases {
		report, err := validateRebuilt(ctx, database, filepath.Join(paths.Staging, database.name))
		if err != nil {
			return err
		}
		result.Databases = append(result.Databases, report)
	}
	return nil
}

// admitOptimizationSwitch rechecks inventory and activity immediately before the first rename.
func admitOptimizationSwitch(ctx context.Context, configuration config.Config, source directorySnapshot, now int64) error {
	current, err := inspectDirectory(configuration)
	if err != nil {
		return err
	}
	if err := unchanged(source, current); err != nil {
		return err
	}
	return EnsureInactive(ctx, configuration, now)
}

// switchOptimizationDirectories preserves separate compensation for each replacement phase.
func switchOptimizationDirectories(ctx context.Context, configuration config.Config, paths RecoveryPaths, source directorySnapshot, hook hooks) error {
	if err := os.Rename(configuration.IndexDir, paths.Rollback); err != nil {
		return err
	}
	err := call(hook.afterRename)
	if err == nil {
		err = os.Rename(paths.Staging, configuration.IndexDir)
	}
	if err != nil {
		if rollbackError := restoreRename(configuration, paths, hook, err); rollbackError != nil {
			return rollbackError
		}
		return err
	}
	err = call(hook.afterSwitch)
	if err == nil {
		err = validateOptimizationSwitch(ctx, configuration, paths, source)
	}
	if err != nil {
		if rollbackError := rollbackSwitch(configuration, paths, hook, err); rollbackError != nil {
			return rollbackError
		}
		return err
	}
	return nil
}

// validateOptimizationSwitch compares the published candidates against retained originals.
func validateOptimizationSwitch(ctx context.Context, configuration config.Config, paths RecoveryPaths, source directorySnapshot) error {
	for _, database := range source.databases {
		database.path = filepath.Join(paths.Rollback, database.name)
		if _, err := validateRebuilt(ctx, database, filepath.Join(configuration.IndexDir, database.name)); err != nil {
			return err
		}
	}
	return nil
}

// finishOptimizationReport computes the existing successful allocation summary.
func finishOptimizationReport(result *Report, sourceBytes, temporary uint64) {
	result.Outcome = "optimized"
	result.DatabaseCount = len(result.Databases)
	result.SourceBytes = sourceBytes
	result.TemporaryBytesRequired = temporary
	for _, report := range result.Databases {
		result.OptimizedBytes += report.After.FileBytes
	}
	if result.SourceBytes > result.OptimizedBytes {
		result.ReclaimedBytes = result.SourceBytes - result.OptimizedBytes
	}
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

// removeKnown deletes only an existing, admitted maintenance directory.
func removeKnown(filename, projectRoot, expectedName string) error {
	present, err := exists(filename)
	if err != nil || !present {
		return err
	}
	parent, err := knownCleanupParent(filename, projectRoot, expectedName)
	if err != nil {
		return err
	}
	if err := admitCleanupDirectory(parent, "parent"); err != nil {
		return err
	}
	if err := admitCleanupDirectory(filename, "target"); err != nil {
		return err
	}
	return os.RemoveAll(filename)
}

// knownCleanupParent requires the exact maintenance basename and absolute project data parent.
func knownCleanupParent(filename, projectRoot, expectedName string) (string, error) {
	parent, err := filepath.Abs(filepath.Join(projectRoot, "data"))
	if err != nil {
		return "", err
	}
	if filepath.Dir(filename) != parent || filepath.Base(filename) != expectedName {
		return "", invalid("maintenance cleanup path escaped the project data directory")
	}
	return parent, nil
}

// admitCleanupDirectory rejects links and non-directory cleanup parents or targets.
func admitCleanupDirectory(filename, role string) error {
	info, err := os.Lstat(filename)
	if err != nil {
		return err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return invalid("maintenance cleanup " + role + " must be a regular directory")
	}
	return nil
}
