package process

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"time"
)

type nativeTree struct{ group int }

func prepareTree(command *exec.Cmd) (*nativeTree, error) {
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	return &nativeTree{}, nil
}

func (tree *nativeTree) attach(command *exec.Cmd, hook startHook) error {
	tree.group = command.Process.Pid
	if hook != nil {
		return hook("after_resume", command)
	}
	return nil
}
func (tree *nativeTree) kill() error {
	if tree.group <= 0 {
		return nil
	}
	err := syscall.Kill(-tree.group, syscall.SIGKILL)
	if errors.Is(err, syscall.ESRCH) {
		return nil
	}
	return classifiedError("kill_failed", err)
}
func (tree *nativeTree) hasRunningMembers() (bool, error) {
	if tree.group <= 0 {
		return false, nil
	}
	if err := syscall.Kill(-tree.group, 0); errors.Is(err, syscall.ESRCH) {
		return false, nil
	} else if err != nil {
		return false, err
	}
	entries, err := os.ReadDir("/proc")
	if err != nil {
		return false, err
	}
	for _, entry := range entries {
		if _, err := strconv.Atoi(entry.Name()); err != nil {
			continue
		}
		data, err := os.ReadFile(filepath.Join("/proc", entry.Name(), "stat"))
		if errors.Is(err, os.ErrNotExist) || errors.Is(err, syscall.ESRCH) {
			return true, nil
		}
		if err != nil {
			return false, err
		}
		isActive, err := isProcessGroupActive(data, tree.group)
		if err != nil || isActive {
			return isActive, err
		}
	}
	finalEntries, err := os.ReadDir("/proc")
	if err != nil {
		return false, err
	}
	if !slices.EqualFunc(entries, finalEntries, func(first, second os.DirEntry) bool { return first.Name() == second.Name() }) {
		return true, nil
	}
	return false, nil
}

// isProcessGroupActive retains zombie leaders while other threads can still own shared descriptors.
func isProcessGroupActive(data []byte, expectedGroup int) (bool, error) {
	fields := strings.Fields(string(data[strings.LastIndexByte(string(data), ')')+1:]))
	if len(fields) < 18 {
		return false, errors.New("invalid process status")
	}
	group, err := strconv.Atoi(fields[2])
	if err != nil {
		return false, err
	}
	if group != expectedGroup {
		return false, nil
	}
	threads, err := strconv.Atoi(fields[17])
	if err != nil || threads < 1 {
		return false, errors.New("invalid process thread count")
	}
	return fields[0] != "Z" && fields[0] != "X" || threads > 1, nil
}

func (tree *nativeTree) terminate(grace time.Duration) (Termination, error) {
	running, err := tree.hasRunningMembers()
	if err != nil {
		return Forced, errors.Join(classifiedError("wait_failed", err), tree.kill())
	}
	if !running {
		return AlreadyExited, nil
	}
	if grace > 0 {
		if err := syscall.Kill(-tree.group, syscall.SIGTERM); err != nil && !errors.Is(err, syscall.ESRCH) {
			return Forced, errors.Join(classifiedError("terminate_failed", err), tree.kill())
		}
		ctx, cancel := context.WithTimeout(context.Background(), grace)
		err := tree.waitEmpty(ctx)
		cancel()
		if err == nil {
			return Graceful, nil
		}
	}
	return Forced, tree.kill()
}

// waitEmpty waits for executable members; orphan zombies are reaped by their actual parent.
func (tree *nativeTree) waitEmpty(ctx context.Context) error {
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		running, err := tree.hasRunningMembers()
		if err != nil || !running {
			return err
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
		}
	}
}
func (tree *nativeTree) close() error { return nil }
