package meta

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type replacement struct {
	target, rollbackPath string
	isApplied            bool
}

// replaceFile retains stage cleanup until no-clobber publication and compensation finish.
func replaceFile(target string, data []byte) (*replacement, error) {
	parent := filepath.Dir(target)
	stage, err := os.CreateTemp(parent, ".litradar-meta-stage-")
	if err != nil {
		return nil, err
	}
	stagePath := stage.Name()
	defer os.Remove(stagePath)
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
		stage.Close()
		return err
	}
	if err := stage.Sync(); err != nil {
		stage.Close()
		return err
	}
	return stage.Close()
}

// reserveReplacementRollback closes and removes its reservation before inspecting the target.
func reserveReplacementRollback(parent string) (string, error) {
	reserved, err := os.CreateTemp(parent, ".litradar-meta-rollback-")
	if err != nil {
		return "", err
	}
	rollback := reserved.Name()
	if err := reserved.Close(); err != nil {
		return "", err
	}
	if err := os.Remove(rollback); err != nil {
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
	if err := os.Rename(target, rollback); err != nil {
		return "", err
	}
	return rollback, nil
}

// publishReplacement restores a retained original immediately when publication fails.
func publishReplacement(stagePath, target, rollback string) error {
	if err := publishFile(stagePath, target); err != nil {
		if rollback != "" {
			if restore := os.Rename(rollback, target); restore != nil {
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
		if err := os.Rename(change.rollbackPath, change.target); err != nil {
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
	for index := len(changes) - 1; index >= 0; index-- {
		if err := changes[index].rollback(); err != nil {
			failures = append(failures, err.Error())
		}
	}
	if len(failures) > 0 {
		return RollbackFailure{failure.Error() + "; " + strings.Join(failures, "; ")}
	}
	return failure
}
