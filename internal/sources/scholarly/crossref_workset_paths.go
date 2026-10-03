package scholarly

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"unicode/utf8"
)

type worksetOwner struct {
	Owner       string  `json:"owner"`
	Version     uint32  `json:"version"`
	Token       string  `json:"token"`
	Scope       string  `json:"scope"`
	Issn        string  `json:"issn"`
	FrozenAt    int64   `json:"frozen_at"`
	UpdatedFrom *string `json:"updated_from"`
}

func makeWorksetOwner(scope string, state CrossrefCheckpoint) worksetOwner {
	return worksetOwner{"litradar-crossref-workset", 1, state.Token, scope, state.Issn, state.FrozenAt, clonePointer(state.UpdatedFrom)}
}

func validateWorksetPath(path string, isDirectory bool) error {
	metadata, err := os.Lstat(path)
	if err != nil {
		return worksetStorageError(err)
	}
	if isWorksetLink(metadata) || metadata.IsDir() != isDirectory || !isDirectory && !metadata.Mode().IsRegular() {
		return invalidWorkset("Crossref workset path is not an ordinary owned file or directory")
	}
	if !isDirectory && hasWorksetHardLinks(metadata) {
		return invalidWorkset("Crossref workset hard links are not allowed")
	}
	return nil
}

func prepareWorksetRoot(root string) (string, error) {
	if !filepath.IsAbs(root) {
		return "", invalidWorkset("Crossref workset root must be an absolute core-owned path")
	}
	for _, component := range strings.Split(filepath.ToSlash(root), "/") {
		if component == ".." {
			return "", invalidWorkset("Crossref workset root must be an absolute core-owned path")
		}
	}
	var ancestors []string
	for path := filepath.Clean(root); ; path = filepath.Dir(path) {
		ancestors = append(ancestors, path)
		if filepath.Dir(path) == path {
			break
		}
	}
	for index := len(ancestors) - 1; index >= 0; index-- {
		path := ancestors[index]
		if _, err := os.Lstat(path); errors.Is(err, os.ErrNotExist) {
			if err = os.Mkdir(path, 0777); err != nil && !errors.Is(err, os.ErrExist) {
				return "", worksetStorageError(err)
			}
		} else if err != nil {
			return "", worksetStorageError(err)
		}
		if err := validateWorksetPath(path, true); err != nil {
			return "", err
		}
	}
	resolved, err := filepath.EvalSymlinks(root)
	if err != nil {
		return "", worksetStorageError(err)
	}
	return resolved, nil
}

func worksetPaths(root, token string) ([]string, error) {
	if !validWorksetToken(token) {
		return nil, invalidWorkset("Crossref workset token is invalid")
	}
	paths := make([]string, 0, 5)
	for _, suffix := range []string{".sqlite", ".sqlite-journal", ".sqlite-wal", ".sqlite-shm", ".json"} {
		paths = append(paths, filepath.Join(root, token+suffix))
	}
	return paths, nil
}

func validateWorksetFiles(root string, owner worksetOwner) error {
	resolved, err := prepareWorksetRoot(root)
	if err != nil {
		return err
	}
	if resolved != root {
		return invalidWorkset("Crossref workset root changed")
	}
	paths, err := worksetPaths(root, owner.Token)
	if err != nil {
		return err
	}
	for _, path := range paths {
		if _, err := os.Lstat(path); errors.Is(err, os.ErrNotExist) {
			continue
		} else if err != nil {
			return worksetStorageError(err)
		}
		if err := validateWorksetPath(path, false); err != nil {
			return err
		}
	}
	manifest := paths[len(paths)-1]
	metadata, err := os.Stat(manifest)
	if err != nil {
		return worksetStorageError(err)
	}
	if metadata.Size() > 65536 {
		return invalidWorkset("Crossref ownership record is too large")
	}
	body, err := os.ReadFile(manifest)
	if err != nil {
		return worksetStorageError(err)
	}
	if !utf8.Valid(body) {
		return worksetStorageError(errors.New("stream did not contain valid UTF-8"))
	}
	var actual worksetOwner
	if decodeWorksetStruct(body, &actual) != nil {
		return invalidWorkset("invalid Crossref workset metadata")
	}
	if !reflect.DeepEqual(actual, owner) {
		return invalidWorkset("Crossref workset ownership or frozen context does not match")
	}
	return nil
}

func removeWorksetFiles(root string, owner worksetOwner) error {
	if err := validateWorksetFiles(root, owner); err != nil {
		return err
	}
	paths, err := worksetPaths(root, owner.Token)
	if err != nil {
		return err
	}
	for _, path := range paths {
		if err := unlinkWorksetFile(path); err != nil && !errors.Is(err, os.ErrNotExist) {
			return worksetStorageError(err)
		}
	}
	return nil
}
