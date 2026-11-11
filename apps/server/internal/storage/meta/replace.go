package meta

import (
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
)

type replacement struct {
	target, rollbackPath string
	isApplied            bool
}

// replaceFile retains stage cleanup until no-clobber publication and compensation finish.
func replaceFile(target string, data []byte) (*replacement, error) {
	parent := filepath.Dir(target)
	root, err := os.OpenRoot(parent)
	if err != nil {
		return nil, err
	}
	defer func() { _ = root.Close() }()
	stage, err := rootfile.CreateTemp(root, ".litradar-meta-stage-", 0600)
	if err != nil {
		return nil, err
	}
	stagePath := stage.Name()
	defer func() {
		if err := root.Remove(filepath.Base(stagePath)); err != nil && !errors.Is(err, os.ErrNotExist) {
			slog.Warn("cleanup.failed", "operation", "os.Remove", "error", err)
		}
	}()
	if err := writeReplacementStage(stage, data); err != nil {
		return nil, err
	}
	rollback, err := reserveReplacementRollback(parent)
	if err != nil {
		return nil, err
	}
	rollback, err = retainReplacementOriginal(target, rollback)
	if err != nil {
		return nil, err
	}
	if err := publishReplacement(stagePath, target, rollback); err != nil {
		return nil, err
	}
	return &replacement{target, rollback, true}, nil
}

// writeReplacementStage preserves write/sync/close error precedence.
func writeReplacementStage(stage *os.File, data []byte) error {
	if _, err := stage.Write(data); err != nil {
		if err := stage.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "stage.Close", "error", err)
		}
		return err
	}
	if err := stage.Sync(); err != nil {
		if err := stage.Close(); err != nil {
			slog.Warn("cleanup.failed", "operation", "stage.Close", "error", err)
		}
		return err
	}
	return stage.Close()
}

// reserveReplacementRollback closes and removes its reservation before inspecting the target.
func reserveReplacementRollback(parent string) (string, error) {
	root, err := os.OpenRoot(parent)
	if err != nil {
		return "", err
	}
	defer func() { _ = root.Close() }()
	reserved, err := rootfile.CreateTemp(root, ".litradar-meta-rollback-", 0600)
	if err != nil {
		return "", err
	}
	rollback := reserved.Name()
	if err := reserved.Close(); err != nil {
		return "", err
	}
	if err := root.Remove(filepath.Base(rollback)); err != nil {
		return "", err
	}
	return rollback, nil
}

// retainReplacementOriginal admits a regular target and moves it into the reserved recovery path.
func retainReplacementOriginal(target, rollback string) (string, error) {
	info, err := os.Lstat(target)
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	if !info.Mode().IsRegular() {
		return "", InvalidBundle{"persistent catalog " + target + " must be a regular file"}
	}
	if err := renameReplacement(target, rollback); err != nil {
		return "", err
	}
	return rollback, nil
}

// publishReplacement restores a retained original immediately when publication fails.
func publishReplacement(stagePath, target, rollback string) error {
	if err := publishFile(stagePath, target); err != nil {
		if rollback != "" {
			if restore := renameReplacement(rollback, target); restore != nil {
				return RollbackFailure{fmt.Sprintf("replacement failed (%v); %s could not be restored (%v)", err, target, restore)}
			}
		}
		return err
	}
	return nil
}

func (change *replacement) rollback() error {
	if !change.isApplied {
		return nil
	}
	if err := removeFile(change.target); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	if change.rollbackPath != "" {
		if err := renameReplacement(change.rollbackPath, change.target); err != nil {
			return err
		}
	}
	change.isApplied = false
	return nil
}

func (change *replacement) finish() error {
	if change.rollbackPath != "" {
		if err := removeFile(change.rollbackPath); err != nil {
			return err
		}
		change.rollbackPath = ""
	}
	change.isApplied = false
	return nil
}

func rollbackAfter(failure error, changes []*replacement) error {
	failures := []string{}
	for _, change := range slices.Backward(changes) {
		if err := change.rollback(); err != nil {
			failures = append(failures, err.Error())
		}
	}
	if len(failures) > 0 {
		return RollbackFailure{failure.Error() + "; " + strings.Join(failures, "; ")}
	}
	return failure
}

func renameReplacement(source, target string) error {
	root, name, err := rootfile.Parent(target)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	relative, err := filepath.Rel(root.Name(), source)
	if err != nil {
		return err
	}
	return root.Rename(relative, name)
}
