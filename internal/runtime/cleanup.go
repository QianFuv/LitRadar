package runtime

import (
	"context"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// ProcessStorage resolves only the first raw path options, before command parsing.
func ProcessStorage(args []string) (config.Config, error) {
	root, err := os.Getwd()
	if err != nil {
		return config.Config{}, err
	}
	if value, err := processOption(args, "--project-root"); err != nil {
		return config.Config{}, err
	} else if value != nil {
		root = *value
	}
	storage := config.FromProjectRoot(root)
	if value, err := processOption(args, "--auth-db"); err != nil {
		return config.Config{}, err
	} else if value != nil {
		storage.AuthDbPath = *value
	}
	return storage, nil
}

func processOption(args []string, name string) (*string, error) {
	position := slices.Index(args, name)
	if position < 0 {
		return nil, nil
	}
	if position+1 == len(args) {
		return nil, errors.New("missing option value")
	}
	return &args[position+1], nil
}

func cleanupPaths(args []string) ([]string, error) {
	storage, err := ProcessStorage(args)
	if err != nil {
		return nil, err
	}
	paths := []string{storage.AuthDbPath}
	for _, directory := range []string{storage.IndexDir, storage.IndexControlDir} {
		entries, err := os.ReadDir(directory)
		if os.IsNotExist(err) {
			continue
		}
		if err != nil {
			return nil, err
		}
		for _, entry := range entries {
			if entry.Type().IsRegular() && entry.Name() != ".sqlite" && filepath.Ext(entry.Name()) == ".sqlite" {
				paths = append(paths, filepath.Join(directory, entry.Name()))
			}
		}
	}
	slices.Sort(paths)
	return slices.Compact(paths), nil
}

// CleanupAfterProcess lets SQLite converge idle WAL sidecars without unlinking active state.
// Internal workers defer cleanup to their parent, even when their marker is malformed.
func CleanupAfterProcess(args []string) {
	if slices.Contains(args, "--litradar-parent-run-id") || slices.Contains(args, "--live-worker-request") {
		return
	}
	paths, err := cleanupPaths(args)
	if err != nil {
		slog.Warn("storage.sqlite_sidecars.cleanup_failed", "event", "storage.sqlite_sidecars.cleanup_failed", "component", "storage", "error_kind", "path_discovery")
		return
	}
	for _, path := range paths {
		label := filepath.Base(path)
		if label == "." || label == string(filepath.Separator) || !utf8.ValidString(label) {
			label = "sqlite"
		}
		result, err := sqlite.CleanupSidecars(context.Background(), path)
		if err != nil {
			slog.Warn("storage.sqlite_sidecars.cleanup_failed", "event", "storage.sqlite_sidecars.cleanup_failed", "component", "storage", "database", label, "error_kind", "sqlite_error")
			continue
		}
		switch result {
		case sqlite.SidecarCleaned:
			slog.Debug("storage.sqlite_sidecars.cleaned", "event", "storage.sqlite_sidecars.cleaned", "component", "storage", "database", label, "outcome", "success")
		case sqlite.SidecarBusy, sqlite.SidecarRetained:
			reason := "sqlite_retained"
			if result == sqlite.SidecarBusy {
				reason = "active_connection"
			}
			slog.Debug("storage.sqlite_sidecars.retained", "event", "storage.sqlite_sidecars.retained", "component", "storage", "database", label, "outcome", "skipped", "reason", reason)
		}
	}
}
