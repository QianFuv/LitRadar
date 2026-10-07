// Package config resolves existing storage layouts and public catalog selections.
package config

import (
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

var (
	ErrNoDatabases       = errors.New("No SQLite databases found")
	ErrNotFound          = errors.New("Database not found")
	ErrMultipleDatabases = errors.New("Multiple databases found, specify ?db=<name>")
	ErrInvalidName       = errors.New("Database name is not a safe catalog stem")
)

// Config separates canonical content from disposable control and credential databases.
type Config struct {
	ProjectRoot     string
	IndexDir        string
	IndexControlDir string
	MetaDir         string
	AuthDbPath      string
}

// FromProjectRoot derives the unchanged deployment data paths.
func FromProjectRoot(root string) Config {
	return Config{root, filepath.Join(root, "data", "index"), filepath.Join(root, "data", "index-control"), filepath.Join(root, "data", "meta"), filepath.Join(root, "data", "auth.sqlite")}
}

// WithAuthDbPath overrides only the explicitly selected credential database.
func (config Config) WithAuthDbPath(filename string) Config {
	config.AuthDbPath = filename
	return config
}

// ResolveIndexDbPath preserves basename selection and the implicit single-database rule.
func (config Config) ResolveIndexDbPath(name *string) (string, error) {
	if name != nil {
		if filename := databaseName(*name); filename != "" {
			selected := filepath.Join(config.IndexDir, filename)
			if _, err := os.Stat(selected); err == nil {
				return selected, nil
			}
			return "", ErrNotFound
		}
	}
	files, err := config.ListIndexDatabases()
	if err != nil {
		return "", err
	}
	switch len(files) {
	case 0:
		return "", ErrNoDatabases
	case 1:
		return files[0], nil
	default:
		return "", ErrMultipleDatabases
	}
}

// ResolveIndexCatalogStem requires a canonical safe name after ordinary database selection.
func (config Config) ResolveIndexCatalogStem(name *string) (string, error) {
	filename, err := config.ResolveIndexDbPath(name)
	if err != nil {
		return "", err
	}
	stem := strings.TrimSuffix(filepath.Base(filename), filepath.Ext(filename))
	if !IsRuntimeName(stem) {
		return "", ErrInvalidName
	}
	return stem, nil
}

// ListIndexDatabases returns sorted matching entries, retaining legacy directory/symlink discovery behavior.
func (config Config) ListIndexDatabases() ([]string, error) {
	if _, err := os.Stat(config.IndexDir); err != nil {
		return []string{}, nil
	}
	entries, err := os.ReadDir(config.IndexDir)
	if err != nil {
		return nil, err
	}
	result := []string{}
	for _, entry := range entries {
		if hasExtension(entry.Name(), "sqlite") {
			result = append(result, filepath.Join(config.IndexDir, entry.Name()))
		}
	}
	sort.Strings(result)
	return result, nil
}

// ListProviderCatalogs merges only regular files with safe lowercase stems across metadata and content.
func (config Config) ListProviderCatalogs() ([]domain.ProviderCatalog, error) {
	catalogs := map[string]domain.ProviderCatalog{}
	for _, source := range []providerCatalogSource{{config.MetaDir, "csv", true}, {config.IndexDir, "sqlite", false}} {
		if err := mergeProviderCatalogSource(source, catalogs); err != nil {
			return nil, err
		}
	}
	result := make([]domain.ProviderCatalog, 0, len(catalogs))
	for _, catalog := range catalogs {
		result = append(result, catalog)
	}
	sort.Slice(result, func(first, second int) bool { return result[first].Stem < result[second].Stem })
	return result, nil
}

type providerCatalogSource struct {
	directory, extension string
	isCsv                bool
}

// mergeProviderCatalogSource ignores stat failures but retains enumeration and entry errors.
func mergeProviderCatalogSource(source providerCatalogSource, catalogs map[string]domain.ProviderCatalog) error {
	if _, err := os.Stat(source.directory); err != nil {
		return nil
	}
	entries, err := os.ReadDir(source.directory)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if err := mergeProviderCatalogEntry(source, entry, catalogs); err != nil {
			return err
		}
	}
	return nil
}

// mergeProviderCatalogEntry reads metadata before filtering and merges distinct basename pointers.
func mergeProviderCatalogEntry(source providerCatalogSource, entry os.DirEntry, catalogs map[string]domain.ProviderCatalog) error {
	metadata, err := entry.Info()
	if err != nil {
		return err
	}
	if !metadata.Mode().IsRegular() || !hasExtension(entry.Name(), source.extension) {
		return nil
	}
	stem := strings.TrimSuffix(entry.Name(), "."+source.extension)
	if !IsRuntimeName(stem) {
		return nil
	}
	catalog := catalogs[stem]
	catalog.Stem = stem
	filename := entry.Name()
	if source.isCsv {
		catalog.CsvFilename = &filename
	} else {
		catalog.DatabaseFilename = &filename
	}
	catalogs[stem] = catalog
	return nil
}

// IsRuntimeName preserves the maintained catalog stem grammar and byte length limit.
func IsRuntimeName(value string) bool {
	if len(value) < 2 || len(value) > 128 {
		return false
	}
	for index, character := range value {
		if isRuntimeLetterOrDigit(character) {
			continue
		}
		if index > 0 && (character == '.' || character == '_' || character == '-') {
			continue
		}
		return false
	}
	return true
}

// isRuntimeLetterOrDigit admits only lowercase ASCII letters and digits.
func isRuntimeLetterOrDigit(character rune) bool {
	return character >= 'a' && character <= 'z' || character >= '0' && character <= '9'
}

func hasExtension(filename, extension string) bool {
	return utf8.ValidString(filename) && filename != "."+extension && filepath.Ext(filename) == "."+extension
}

func databaseName(value string) string {
	value = strings.TrimSpace(value)
	value = strings.TrimPrefix(value, filepath.VolumeName(value))
	for value != "" {
		value = strings.TrimRightFunc(value, func(character rune) bool { return character < 128 && os.IsPathSeparator(uint8(character)) })
		filename := filepath.Base(value)
		if filename == ".." {
			return ""
		}
		if filename == "." {
			value = strings.TrimSuffix(value, ".")
			continue
		}
		if !utf8.ValidString(filename) {
			return ""
		}
		if strings.HasSuffix(filename, ".sqlite") {
			return filename
		}
		return filename + ".sqlite"
	}
	return ""
}

// NormalizeDatabaseName retains host-platform basename selection without requiring the database to exist.
func NormalizeDatabaseName(value string) string { return databaseName(value) }
