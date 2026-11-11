package weekly

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
)

func manifestPaths(directory string) ([]string, error) {
	root, err := os.OpenRoot(directory)
	if err != nil {
		return nil, err
	}
	defer func() { _ = root.Close() }()
	entries, err := fs.ReadDir(root.FS(), ".")
	if err != nil {
		return nil, err
	}
	paths := []string{}
	for _, entry := range entries {
		info, err := entry.Info()
		if err != nil {
			return nil, err
		}
		if info.Mode().IsRegular() && strings.HasSuffix(entry.Name(), ".changes.json") {
			paths = append(paths, filepath.Join(directory, entry.Name()))
		}
	}
	if _, err := root.Stat("history"); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return paths, nil
		}
		return nil, err
	}
	catalogs, err := fs.ReadDir(root.FS(), "history")
	if err != nil {
		return nil, err
	}
	historyPaths, err := historyManifestPaths(root, catalogs)
	if err != nil {
		return nil, err
	}
	paths = append(paths, historyPaths...)
	slices.Sort(paths)
	return paths, nil
}

func managedHistoryName(name string) bool {
	const suffix = ".changes.json"
	if !strings.HasSuffix(name, suffix) {
		return false
	}
	digest := strings.TrimSuffix(name, suffix)
	if len(digest) != 64 {
		return false
	}
	for _, character := range digest {
		if !(character >= '0' && character <= '9' || character >= 'a' && character <= 'f') {
			return false
		}
	}
	return true
}

func readRootManifest(root *os.Root, name string) (*Manifest, error) {
	data, err := root.ReadFile(name)
	if err != nil {
		return nil, err
	}
	return ParseManifest(data)
}

func compareRun(first, second *string) int {
	if first == nil {
		if second == nil {
			return 0
		}
		return -1
	}
	if second == nil {
		return 1
	}
	return strings.Compare(*first, *second)
}

// LoadManifests reads current and immutable history publications within inclusive fixed UTC bounds.
func LoadManifests(configuration config.Config, start, end Timestamp, cache *Cache) ([]Manifest, error) {
	directory := filepath.Join(configuration.ProjectRoot, "data", "push_state")
	if _, err := os.Stat(directory); err != nil {
		return []Manifest{}, nil
	}
	paths, err := manifestPaths(directory)
	if err != nil {
		return nil, err
	}
	root, err := os.OpenRoot(directory)
	if err != nil {
		return nil, err
	}
	defer func() { _ = root.Close() }()
	result := []Manifest{}
	seen := map[string]bool{}
	for _, filename := range paths {
		name, err := filepath.Rel(directory, filename)
		if err != nil {
			return nil, err
		}
		var manifest *Manifest
		if cache == nil {
			manifest, err = readRootManifest(root, name)
		} else {
			manifest, err = cache.readRootAt(root, name, time.Now())
		}
		if err != nil {
			return nil, err
		}
		if manifest == nil || manifest.GeneratedAt.Compare(start) < 0 || manifest.GeneratedAt.Compare(end) > 0 {
			continue
		}
		key := manifestKey(*manifest)
		if seen[key] {
			continue
		}
		seen[key] = true
		result = append(result, *manifest)
	}
	slices.SortFunc(result, func(first, second Manifest) int {
		if order := second.GeneratedAt.Compare(first.GeneratedAt); order != 0 {
			return order
		}
		if order := strings.Compare(first.DbName, second.DbName); order != 0 {
			return order
		}
		if order := compareRun(first.RunId, second.RunId); order != 0 {
			return order
		}
		return slices.Compare(first.ArticleIds, second.ArticleIds)
	})
	return result, nil
}

// manifestKey serializes exact publication identity while reusing a bounded integer formatting buffer.
func manifestKey(manifest Manifest) string {
	var result strings.Builder
	var numberBuffer [20]byte
	result.WriteString(strconv.Quote(manifest.DbName))
	result.WriteByte('|')
	if manifest.RunId == nil {
		result.WriteString("null")
	} else {
		result.WriteString(strconv.Quote(*manifest.RunId))
	}
	result.WriteByte('|')
	result.Write(strconv.AppendInt(numberBuffer[:0], manifest.GeneratedAt.Seconds, 10))
	result.WriteByte(':')
	result.Write(strconv.AppendUint(numberBuffer[:0], uint64(manifest.GeneratedAt.Nanoseconds), 10))
	for _, id := range manifest.ArticleIds {
		result.WriteByte('|')
		result.Write(strconv.AppendInt(numberBuffer[:0], id, 10))
	}
	return result.String()
}

func rootFingerprint(root *os.Root, name string) (fileFingerprint, error) {
	info, err := root.Stat(name)
	if err != nil {
		return fileFingerprint{}, err
	}
	created, hasCreated := creationTime(filepath.Join(root.Name(), name), info)
	return fileFingerprint{info.Size(), info.ModTime().Unix(), info.ModTime().Nanosecond(), created, hasCreated}, nil
}

// historyManifestPaths admits regular managed publications beneath actual catalog directories.
func historyManifestPaths(root *os.Root, catalogs []os.DirEntry) ([]string, error) {
	paths := []string{}
	for _, catalog := range catalogs {
		if !catalog.IsDir() {
			continue
		}
		entries, err := fs.ReadDir(root.FS(), "history/"+catalog.Name())
		if err != nil {
			return nil, err
		}
		for _, entry := range entries {
			info, err := entry.Info()
			if err != nil {
				return nil, err
			}
			if info.Mode().IsRegular() && managedHistoryName(entry.Name()) {
				paths = append(paths, filepath.Join(root.Name(), "history", catalog.Name(), entry.Name()))
			}
		}
	}
	return paths, nil
}
