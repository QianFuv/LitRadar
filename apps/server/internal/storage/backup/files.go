package backup

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
)

type snapshotFile struct {
	Path   string
	Size   uint64
	Sha256 string
}

func hashFile(filename string) (resultValue1 string, resultError error) {
	root, name, err := rootfile.Parent(filename)
	if err != nil {
		return "", err
	}
	defer func() { _ = root.Close() }()
	return hashRootFile(root, name)
}

func hashRootFile(root *os.Root, name string) (resultValue1 string, resultError error) {
	file, err := root.Open(name)
	if err != nil {
		return "", err
	}
	defer func() {
		if closeError := file.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close file: %w", closeError))
		}
	}()
	hash := sha256.New()
	if _, err := io.CopyBuffer(hash, file, make([]byte, 65536)); err != nil {
		return "", err
	}
	return hex.EncodeToString(hash.Sum(nil)), nil
}
func snapshot(root string) ([]snapshotFile, error) {
	result := []snapshotFile{}
	if _, err := os.Stat(root); err != nil {
		return result, nil
	}
	info, err := os.Lstat(root)
	if err != nil {
		return nil, err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return nil, failure("integrity", "backup source directory is not a regular directory")
	}
	opened, err := os.OpenRoot(root)
	if err != nil {
		return nil, err
	}
	defer func() { _ = opened.Close() }()
	err = fs.WalkDir(opened.FS(), ".", func(filename string, entry os.DirEntry, walkError error) error {
		if walkError != nil {
			return walkError
		}
		if filename == "." {
			return nil
		}
		if entry.Type()&os.ModeSymlink != 0 {
			return failure("integrity", "symbolic links are not allowed in backups")
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		if info.IsDir() {
			return nil
		}
		if !info.Mode().IsRegular() {
			return failure("integrity", "special files are not allowed in backups")
		}
		relative := filename
		digest, err := hashRootFile(opened, filename)
		if err != nil {
			return err
		}
		result = append(result, snapshotFile{filepath.ToSlash(relative), uint64(info.Size()), digest})
		return nil
	})
	slices.SortFunc(result, func(first, second snapshotFile) int { return strings.Compare(first.Path, second.Path) })
	return result, err
}
func copyFile(source, destination string) (resultError error) {
	sourceRoot, sourceName, err := rootfile.Parent(source)
	if err != nil {
		return err
	}
	defer func() { _ = sourceRoot.Close() }()
	if err := os.MkdirAll(filepath.Dir(destination), 0777); err != nil {
		return err
	}
	targetRoot, targetName, err := rootfile.Parent(destination)
	if err != nil {
		return err
	}
	defer func() { _ = targetRoot.Close() }()
	return copyRootFile(sourceRoot, sourceName, targetRoot, targetName)
}

func copyRootFile(sourceRoot *os.Root, source string, targetRoot *os.Root, destination string) (resultError error) {
	input, err := sourceRoot.Open(source)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := input.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close input: %w", closeError))
		}
	}()
	info, err := input.Stat()
	if err != nil {
		return err
	}
	if err := targetRoot.MkdirAll(filepath.Dir(destination), 0777); err != nil {
		return err
	}
	output, err := targetRoot.OpenFile(destination, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, info.Mode().Perm())
	if err != nil {
		return err
	}
	_, copyError := io.Copy(output, input)
	permissionError := output.Chmod(info.Mode())
	closeError := output.Close()
	if copyError != nil {
		return copyError
	}
	if permissionError != nil {
		return permissionError
	}
	return closeError
}
func copyGroup(source, stage, directory, kind, label string, afterCopy func(string) error) ([]Component, error) {
	before, err := snapshot(source)
	if err != nil {
		return nil, err
	}
	destination := filepath.Join(stage, directory)
	if err := os.MkdirAll(destination, 0777); err != nil {
		return nil, err
	}
	for _, file := range before {
		if err := copySnapshotFile(source, destination, file, label); err != nil {
			return nil, err
		}
	}
	if afterCopy != nil {
		if err := afterCopy(source); err != nil {
			return nil, err
		}
	}
	after, err := snapshot(source)
	if err != nil {
		return nil, err
	}
	if !reflect.DeepEqual(before, after) {
		return nil, failure("integrity", "%s files changed during backup", label)
	}
	components := make([]Component, 0, len(before))
	for _, file := range before {
		components = append(components, Component{Kind: kind, Path: directory + "/" + file.Path, Size: file.Size, Sha256: file.Sha256})
	}
	return components, nil
}
func canonical(filename string) (string, error) {
	resolved, err := filepath.EvalSymlinks(filename)
	if err != nil {
		return "", err
	}
	return filepath.Abs(resolved)
}
func within(filename, parent string) bool {
	relative, err := filepath.Rel(parent, filename)
	return err == nil && relative != ".." && !strings.HasPrefix(relative, ".."+string(filepath.Separator)) && !filepath.IsAbs(relative)
}
func fileExists(filename string) bool { _, err := os.Stat(filename); return err == nil }
func removeFile(filename string) error {
	if !fileExists(filename) {
		return nil
	}
	root, name, err := rootfile.Parent(filename)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	info, err := root.Lstat(name)
	if err != nil {
		return err
	}
	if info.IsDir() {
		return &os.PathError{Op: "remove", Path: filename, Err: os.ErrInvalid}
	}
	return root.Remove(name)
}

func copySnapshotFile(source, destination string, file snapshotFile, label string) error {
	if keyFile(file.Path) {
		return failure("input", "%s directory contains a forbidden key file", label)
	}
	sourceRoot, err := os.OpenRoot(source)
	if err != nil {
		return err
	}
	defer func() { _ = sourceRoot.Close() }()
	targetRoot, err := os.OpenRoot(destination)
	if err != nil {
		return err
	}
	defer func() { _ = targetRoot.Close() }()
	if err := copyRootFile(sourceRoot, filepath.FromSlash(file.Path), targetRoot, filepath.FromSlash(file.Path)); err != nil {
		return err
	}
	info, err := targetRoot.Stat(filepath.FromSlash(file.Path))
	if err != nil {
		return err
	}
	digest, err := hashRootFile(targetRoot, filepath.FromSlash(file.Path))
	if err != nil {
		return err
	}
	if uint64(info.Size()) != file.Size || digest != file.Sha256 {
		return failure("integrity", "%s copy changed while it was written", label)
	}
	return nil
}
