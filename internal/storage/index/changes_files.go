package index

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

func manifestTempPath(path string) string {
	return filepath.Join(filepath.Dir(path), fmt.Sprintf(".%s.%d.tmp", filepath.Base(path), os.Getpid()))
}

func stageManifest(path string, payload []byte) (string, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0777); err != nil {
		return "", err
	}
	temporary := manifestTempPath(path)
	file, err := os.OpenFile(temporary, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0666)
	if err != nil {
		return temporary, err
	}
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
	temporary, err := stageManifest(path, payload)
	hasPublished := false
	if temporary != "" {
		defer func() {
			if !hasPublished {
				_ = removeManifestFile(temporary)
			}
		}()
	}
	if err != nil {
		return err
	}
	if err := os.Rename(temporary, path); err != nil {
		return err
	}
	hasPublished = true
	return syncManifestDirectory(path)
}

func verifyHistory(path string, payload []byte) (bool, error) {
	existing, err := os.ReadFile(path)
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
	if exists, err := verifyHistory(path, payload); err != nil || exists {
		return err
	}
	temporary, err := stageManifest(path, payload)
	hasPublished := false
	if temporary != "" {
		defer func() {
			if !hasPublished {
				_ = removeManifestFile(temporary)
			}
		}()
	}
	if err != nil {
		return err
	}
	if err := os.Link(temporary, path); err != nil {
		if !errors.Is(err, os.ErrExist) {
			return err
		}
		exists, err := verifyHistory(path, payload)
		if err != nil {
			return err
		}
		if !exists {
			return &manifestFileError{"concurrent change history publication disappeared", os.ErrNotExist}
		}
		return nil
	}
	if err := removeManifestFile(temporary); err != nil {
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

// PruneContentChangeHistory removes only managed hash-named files with canonical expired epoch timestamps.
func PruneContentChangeHistory(directory string, cutoff int64) (uint64, error) {
	entries, err := os.ReadDir(directory)
	if errors.Is(err, os.ErrNotExist) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	var removed uint64
	for _, entry := range entries {
		name := entry.Name()
		digest, ok := strings.CutSuffix(name, ".changes.json")
		if !ok || len(digest) != 64 {
			continue
		}
		isManaged := true
		for _, character := range []byte(digest) {
			if !(character >= '0' && character <= '9' || character >= 'a' && character <= 'f') {
				isManaged = false
				break
			}
		}
		if !isManaged {
			continue
		}
		path := filepath.Join(directory, name)
		info, err := os.Stat(path)
		if err != nil || !info.Mode().IsRegular() {
			continue
		}
		body, err := os.ReadFile(path)
		if err != nil {
			return removed, err
		}
		if !jsonvalue.ValidJson(string(body)) {
			return removed, errors.New("invalid change history JSON")
		}
		var document map[string]json.RawMessage
		if json.Unmarshal(body, &document) != nil {
			continue
		}
		var generated string
		if json.Unmarshal(document["generated_at"], &generated) != nil {
			continue
		}
		generated = strings.TrimSpace(generated)
		epoch, err := strconv.ParseInt(generated, 10, 64)
		if err != nil || strconv.FormatInt(epoch, 10) != generated {
			continue
		}
		if epoch < cutoff {
			if err := removeManifestFile(path); err != nil {
				return removed, err
			}
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
