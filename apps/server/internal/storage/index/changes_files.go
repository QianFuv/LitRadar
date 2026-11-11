package index

import (
	"bytes"
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
)

func stageManifest(root *os.Root, path string, payload []byte) (string, error) {
	file, err := rootfile.CreateTemp(root, "."+filepath.Base(path)+"-", 0666)
	if err != nil {
		return "", err
	}
	temporary := filepath.Base(file.Name())
	_, err = file.Write(payload)
	if err == nil {
		err = file.Sync()
	}
	closeError := file.Close()
	if err == nil {
		err = closeError
	}
	return temporary, err
}

// PublishContentChangeManifest replaces the current publication after flushing its exact bytes.
func PublishContentChangeManifest(path string, payload []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0777); err != nil {
		return err
	}
	root, name, err := rootfile.Parent(path)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()

	temporary, err := stageManifest(root, name, payload)
	hasPublished := false
	if temporary != "" {
		defer func() {
			if !hasPublished {
				_ = root.Remove(temporary)
			}
		}()
	}
	if err != nil {
		return err
	}
	if err := root.Rename(temporary, name); err != nil {
		return err
	}
	hasPublished = true
	return syncManifestDirectory(path)
}

func verifyHistory(root *os.Root, path string, payload []byte) (bool, error) {
	existing, err := root.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	if bytes.Equal(existing, payload) {
		return true, nil
	}
	return false, &manifestFileError{"existing change history payload does not match its digest path", os.ErrExist}
}

// PublishContentChangeHistory never replaces different bytes at an immutable history path.
func PublishContentChangeHistory(path string, payload []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0777); err != nil {
		return err
	}
	root, name, err := rootfile.Parent(path)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()

	if exists, err := verifyHistory(root, name, payload); err != nil || exists {
		return err
	}
	temporary, err := stageManifest(root, name, payload)
	hasPublished := false
	if temporary != "" {
		defer func() {
			if !hasPublished {
				_ = root.Remove(temporary)
			}
		}()
	}
	if err != nil {
		return err
	}
	if err := root.Link(temporary, name); err != nil {
		if !errors.Is(err, os.ErrExist) {
			return err
		}
		exists, err := verifyHistory(root, name, payload)
		if err != nil {
			return err
		}
		if !exists {
			return &manifestFileError{"concurrent change history publication disappeared", os.ErrNotExist}
		}
		return nil
	}
	if err := root.Remove(temporary); err != nil {
		return err
	}
	hasPublished = true
	return syncManifestDirectory(path)
}

type manifestFileError struct {
	message string
	cause   error
}

func (err *manifestFileError) Error() string { return err.message }
func (err *manifestFileError) Unwrap() error { return err.cause }

// PruneContentChangeHistory removes only managed hash-named files with expired RFC3339Nano timestamps.
func PruneContentChangeHistory(directory string, cutoff int64) (uint64, error) {
	root, err := os.OpenRoot(directory)
	if errors.Is(err, os.ErrNotExist) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	defer func() { _ = root.Close() }()
	entries, err := fs.ReadDir(root.FS(), ".")
	if errors.Is(err, os.ErrNotExist) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	var removed uint64
	for _, entry := range entries {
		wasRemoved, err := pruneHistoryEntry(root, entry, cutoff)
		if err != nil {
			return removed, err
		}
		if wasRemoved {
			removed++
		}
	}
	if removed > 0 {
		if err := syncManifestDirectory(filepath.Join(directory, "history.changes.json")); err != nil {
			return removed, err
		}
	}
	return removed, nil
}

func isManagedHistoryName(name string) bool {
	digest, ok := strings.CutSuffix(name, ".changes.json")
	if !ok || len(digest) != 64 {
		return false
	}
	isManaged := true
	for _, character := range []byte(digest) {
		if !(character >= '0' && character <= '9' || character >= 'a' && character <= 'f') {
			isManaged = false
			break
		}
	}
	if !isManaged {
		return false
	}
	return true
}

func pruneHistoryEntry(root *os.Root, entry os.DirEntry, cutoff int64) (bool, error) {
	if !isManagedHistoryName(entry.Name()) {
		return false, nil
	}
	path := entry.Name()
	info, err := root.Stat(path)
	if err != nil || !info.Mode().IsRegular() {
		return false, nil
	}
	body, err := root.ReadFile(path)
	if err != nil {
		return false, err
	}
	if !jsonvalue.ValidJson(string(body)) {
		return false, errors.New("invalid change history JSON")
	}
	epoch, isCanonical := historyEpoch(body)
	if !isCanonical {
		return false, nil
	}
	if epoch < cutoff {
		if err := root.Remove(path); err != nil {
			return false, err
		}
		return true, nil
	}
	return false, nil
}

func historyEpoch(body []byte) (int64, bool) {
	var document map[string]json.RawMessage
	if json.Unmarshal(body, &document) != nil {
		return 0, false
	}
	var generated string
	if json.Unmarshal(document["generated_at"], &generated) != nil {
		return 0, false
	}
	stamp, err := time.Parse(time.RFC3339Nano, generated)
	if err != nil {
		return 0, false
	}
	return stamp.Unix(), true
}
