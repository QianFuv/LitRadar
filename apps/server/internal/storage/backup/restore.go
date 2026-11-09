package backup

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type replacement struct {
	target, staged, rollback string
	hadOriginal, isApplied   bool
}

func (item *replacement) apply() error {
	if fileExists(item.target) {
		info, err := os.Lstat(item.target)
		if err != nil {
			return err
		}
		if info.Mode()&os.ModeSymlink != 0 {
			return failure("input", "restore targets cannot be symbolic links")
		}
		if err := os.Rename(item.target, item.rollback); err != nil {
			return err
		}
		item.hadOriginal = true
	}
	if item.staged != "" {
		if err := os.Rename(item.staged, item.target); err != nil {
			if rollbackError := item.restoreOriginalAfterFailedRename(err); rollbackError != nil {
				return rollbackError
			}
			return err
		}
	}
	item.isApplied = true
	return nil
}
func (item *replacement) compensate() error {
	if !item.isApplied {
		return nil
	}
	if item.staged != "" && fileExists(item.target) {
		if err := os.RemoveAll(item.target); err != nil {
			return err
		}
	}
	if item.hadOriginal {
		if err := os.Rename(item.rollback, item.target); err != nil {
			return err
		}
	}
	item.isApplied = false
	item.hadOriginal = false
	return nil
}
func rollback(items []replacement) error {
	failures := []string{}
	for index := len(items) - 1; index >= 0; index-- {
		if err := items[index].compensate(); err != nil {
			failures = append(failures, err.Error())
		}
	}
	if len(failures) > 0 {
		return failure("rollback", "%s", strings.Join(failures, "; "))
	}
	return nil
}
func apply(items []replacement) error {
	for index := range items {
		if err := items[index].apply(); err != nil {
			if rollbackError := rollback(items[:index]); rollbackError != nil {
				return rollbackError
			}
			return err
		}
	}
	return nil
}
func selectedGroup(backup string, manifest Manifest, kind, directory, destination string) error {
	if err := os.MkdirAll(destination, 0777); err != nil {
		return err
	}
	for _, item := range manifest.Components {
		if item.Kind != kind || !strings.HasPrefix(item.Path, directory+"/") {
			continue
		}
		relative, err := parsePath(item.Path)
		if err != nil {
			return err
		}
		target := filepath.Join(destination, filepath.FromSlash(strings.TrimPrefix(item.Path, directory+"/")))
		if err := copyFile(filepath.Join(backup, relative), target); err != nil {
			return err
		}
	}
	return nil
}
func validateOutside(options RestoreOptions, selection Selection) error {
	backup, err := canonical(options.BackupDir)
	if err != nil {
		return err
	}
	targets := []string{options.AuthDbPath}
	if selection.Metadata {
		targets = append(targets, options.Config.MetaDir)
	}
	if selection.IndexDatabases {
		targets = append(targets, options.Config.IndexDir)
	}
	if selection.PushState {
		for _, name := range []string{"push_state", "folder_push_state"} {
			targets = append(targets, filepath.Join(options.Config.ProjectRoot, "data", name))
		}
	}
	for _, target := range targets {
		resolved, err := canonical(target)
		if err == nil && within(backup, resolved) {
			return failure("input", "backup directory must be outside restored targets")
		}
	}
	return nil
}

// Restore verifies first, rejects active targets twice, and compensates every applied replacement on failure.
func Restore(ctx context.Context, options RestoreOptions) (RestoreReport, error) {
	now := time.Now()
	if now.Unix() < 0 {
		return RestoreReport{}, failure("input", "system time is before the Unix epoch")
	}
	return restore(ctx, options, float64(now.Unix())+float64(now.Nanosecond())/1e9, nil, nil)
}
func restore(ctx context.Context, options RestoreOptions, now float64, beforeReplace func() error, afterReplace func() error) (report RestoreReport, returnError error) {
	manifest, err := Verify(ctx, options.BackupDir)
	if err != nil {
		return report, err
	}

	if err := requireInactiveRestoreTarget(ctx, options.AuthDbPath, now); err != nil {
		return report, err
	}
	if err := validateOutside(options, manifest.Selection); err != nil {
		return report, err
	}
	parent := filepath.Dir(options.AuthDbPath)
	if err := os.MkdirAll(parent, 0777); err != nil {
		return report, err
	}
	data := filepath.Join(options.Config.ProjectRoot, "data")
	if err := os.MkdirAll(data, 0777); err != nil {
		return report, err
	}
	authWorkspace, err := os.MkdirTemp(parent, ".litradar-auth-restore-")
	if err != nil {
		return report, err
	}
	defer func() {
		var failed Failure
		if !errors.As(returnError, &failed) || failed.Kind != "rollback" {
			os.RemoveAll(authWorkspace)
		}
	}()
	dataWorkspace, err := os.MkdirTemp(data, ".litradar-data-restore-")
	if err != nil {
		return report, err
	}
	defer func() {
		var failed Failure
		if !errors.As(returnError, &failed) || failed.Kind != "rollback" {
			os.RemoveAll(dataWorkspace)
		}
	}()
	items, err := stageRestoreReplacements(options, manifest, data, authWorkspace, dataWorkspace)
	if err != nil {
		return report, err
	}
	if err := executeRestoreReplacements(ctx, options, manifest, now, data, items, beforeReplace, afterReplace); err != nil {
		return report, err
	}
	return restoredManifestReport(manifest), nil
}

func (item *replacement) restoreOriginalAfterFailedRename(replaceError error) error {
	if item.hadOriginal {
		if rollbackError := os.Rename(item.rollback, item.target); rollbackError != nil {
			return failure("rollback", "replacement failed (%v); original could not be restored (%v)", replaceError, rollbackError)
		}
		item.hadOriginal = false
	}
	return nil
}

func requireInactiveRestoreTarget(ctx context.Context, filename string, now float64) error {
	active, err := HasRecentHeartbeat(ctx, filename, now, ActiveHeartbeatMaxAge)
	if err != nil {
		return err
	}
	if active {
		return ErrActiveTarget
	}
	return nil
}

func stageRestoreReplacements(options RestoreOptions, manifest Manifest, data, authWorkspace, dataWorkspace string) ([]replacement, error) {
	stagedAuth := filepath.Join(authWorkspace, "staged-auth.sqlite")
	if err := copyFile(filepath.Join(options.BackupDir, "auth.sqlite"), stagedAuth); err != nil {
		return nil, err
	}
	items := []replacement{{target: options.AuthDbPath, staged: stagedAuth, rollback: filepath.Join(authWorkspace, "rollback-auth.sqlite")}}
	for _, suffix := range []string{"-wal", "-shm", "-journal"} {
		items = append(items, replacement{target: options.AuthDbPath + suffix, rollback: filepath.Join(authWorkspace, "rollback-auth.sqlite"+suffix)})
	}
	groups := []struct {
		enabled                 bool
		kind, directory, target string
	}{
		{manifest.Selection.Metadata, "metadata", "meta", options.Config.MetaDir},
		{manifest.Selection.IndexDatabases, "index_database", "index", options.Config.IndexDir},
		{manifest.Selection.PushState, "push_state", "push_state", filepath.Join(data, "push_state")},
		{manifest.Selection.PushState, "push_state", "folder_push_state", filepath.Join(data, "folder_push_state")},
	}
	for _, group := range groups {
		if !group.enabled {
			continue
		}
		stage := filepath.Join(dataWorkspace, "staged-"+group.directory)
		if err := selectedGroup(options.BackupDir, manifest, group.kind, group.directory, stage); err != nil {
			return nil, err
		}
		items = append(items, replacement{target: group.target, staged: stage, rollback: filepath.Join(dataWorkspace, "rollback-"+group.directory)})
	}
	return items, nil
}

func executeRestoreReplacements(ctx context.Context, options RestoreOptions, manifest Manifest, now float64, data string, items []replacement, beforeReplace, afterReplace func() error) error {
	var err error
	if beforeReplace != nil {
		if err := beforeReplace(); err != nil {
			return err
		}
	}
	if err := requireInactiveRestoreTarget(ctx, options.AuthDbPath, now); err != nil {
		return err
	}
	if err := apply(items); err != nil {
		return err
	}
	if afterReplace != nil {
		err = afterReplace()
	}
	if err == nil {
		err = validateRestoredComponents(ctx, options, manifest, data)
	}
	if err != nil {
		if rollbackError := rollback(items); rollbackError != nil {
			return rollbackError
		}
		return err
	}
	return nil
}

func validateRestoredComponents(ctx context.Context, options RestoreOptions, manifest Manifest, data string) error {
	for _, component := range manifest.Components {
		target := options.AuthDbPath
		if component.Kind != "auth_database" {
			target = filepath.Join(data, filepath.FromSlash(component.Path))
		}
		if err := validateFile(ctx, component, target); err != nil {
			return err
		}
	}
	return nil
}

func restoredManifestReport(manifest Manifest) RestoreReport {
	report := RestoreReport{RestoredFiles: len(manifest.Components), RestoredMetadata: manifest.Selection.Metadata, RestoredIndexDatabases: manifest.Selection.IndexDatabases, RestoredPushState: manifest.Selection.PushState}
	for _, component := range manifest.Components {
		if component.Kind == "auth_database" || component.Kind == "index_database" {
			report.RestoredDatabases++
		}
	}
	return report
}
