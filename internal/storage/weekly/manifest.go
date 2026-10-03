package weekly

import (
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

func manifestPaths(directory string) ([]string, error) {
	entries, err := os.ReadDir(directory)
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
	history := filepath.Join(directory, "history")
	if _, err := os.Stat(history); err != nil {
		return paths, nil
	}
	catalogs, err := os.ReadDir(history)
	if err != nil {
		return nil, err
	}
	for _, catalog := range catalogs {
		if !catalog.IsDir() {
			continue
		}
		entries, err := os.ReadDir(filepath.Join(history, catalog.Name()))
		if err != nil {
			return nil, err
		}
		for _, entry := range entries {
			info, err := entry.Info()
			if err != nil {
				return nil, err
			}
			if info.Mode().IsRegular() && managedHistoryName(entry.Name()) {
				paths = append(paths, filepath.Join(history, catalog.Name(), entry.Name()))
			}
		}
	}
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

func readManifest(filename string) (*Manifest, error) {
	data, err := os.ReadFile(filename)
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
	result := []Manifest{}
	seen := map[string]bool{}
	for _, filename := range paths {
		var manifest *Manifest
		if cache == nil {
			manifest, err = readManifest(filename)
		} else {
			manifest, err = cache.read(filename)
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

func manifestKey(manifest Manifest) string {
	var result strings.Builder
	result.WriteString(strconv.Quote(manifest.DbName))
	result.WriteByte('|')
	if manifest.RunId == nil {
		result.WriteString("null")
	} else {
		result.WriteString(strconv.Quote(*manifest.RunId))
	}
	result.WriteByte('|')
	result.WriteString(strconv.FormatInt(manifest.GeneratedAt.Seconds, 10))
	result.WriteByte(':')
	result.WriteString(strconv.FormatUint(uint64(manifest.GeneratedAt.Nanoseconds), 10))
	for _, id := range manifest.ArticleIds {
		result.WriteByte('|')
		result.WriteString(strconv.FormatInt(id, 10))
	}
	return result.String()
}

func canonicalPath(filename string) (string, error) {
	resolved, err := filepath.EvalSymlinks(filename)
	if err != nil {
		return "", err
	}
	return filepath.Abs(resolved)
}

func fingerprint(filename string) (fileFingerprint, error) {
	info, err := os.Stat(filename)
	if err != nil {
		return fileFingerprint{}, err
	}
	created, hasCreated := creationTime(filename, info)
	return fileFingerprint{info.Size(), info.ModTime().Unix(), info.ModTime().Nanosecond(), created, hasCreated}, nil
}
