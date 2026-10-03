package backup

import (
	"bytes"
	"context"
	"encoding/json"
	"math"
	"os"
	"path"
	"path/filepath"
	"slices"
	"strings"

	auth "github.com/QianFuv/LitRadar/internal/domain/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func fields(raw []byte, names []string, minimum int) (map[string]json.RawMessage, error) {
	if !json.Valid(raw) {
		return nil, ErrManifestJson
	}
	result := map[string]json.RawMessage{}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	opening, err := decoder.Token()
	if err != nil {
		return nil, ErrManifestJson
	}
	switch opening {
	case json.Delim('['):
		var values []json.RawMessage
		if json.Unmarshal(raw, &values) != nil || len(values) < minimum || len(values) > len(names) {
			return nil, ErrManifestJson
		}
		for index, value := range values {
			result[names[index]] = value
		}
	case json.Delim('{'):
		for decoder.More() {
			start := decoder.InputOffset()
			key, err := decoder.Token()
			if err != nil {
				return nil, ErrManifestJson
			}
			rawKey := bytes.TrimSpace(bytes.TrimPrefix(bytes.TrimSpace(raw[start:decoder.InputOffset()]), []byte(",")))
			if !auth.ValidJson(string(rawKey)) {
				return nil, ErrManifestJson
			}
			name, ok := key.(string)
			if !ok {
				return nil, ErrManifestJson
			}
			var value json.RawMessage
			if decoder.Decode(&value) != nil {
				return nil, ErrManifestJson
			}
			if !slices.Contains(names, name) {
				continue
			}
			if _, exists := result[name]; exists {
				return nil, ErrManifestJson
			}
			result[name] = value
		}
	default:
		return nil, ErrManifestJson
	}
	return result, nil
}
func required[T any](values map[string]json.RawMessage, key string, target *T) error {
	raw := values[key]
	if len(raw) == 0 || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) || !auth.ValidJson("["+string(raw)+"]") || json.Unmarshal(raw, target) != nil {
		return ErrManifestJson
	}
	return nil
}

// ParseManifest preserves required fields, duplicate rejection and historical serde positional structs.
func ParseManifest(raw []byte) (Manifest, error) {
	result := Manifest{}
	values, err := fields(raw, []string{"format", "version", "created_at", "selection", "components"}, 5)
	if err != nil {
		return result, err
	}
	if required(values, "format", &result.Format) != nil || required(values, "version", &result.Version) != nil || required(values, "created_at", &result.CreatedAt) != nil {
		return result, ErrManifestJson
	}
	selection, err := fields(values["selection"], []string{"metadata", "index_databases", "push_state"}, 3)
	if err != nil {
		return result, err
	}
	if selection["metadata"] != nil {
		if err := required(selection, "metadata", &result.Selection.Metadata); err != nil {
			return result, err
		}
	}
	if required(selection, "index_databases", &result.Selection.IndexDatabases) != nil || required(selection, "push_state", &result.Selection.PushState) != nil {
		return result, ErrManifestJson
	}
	var components []json.RawMessage
	if len(values["components"]) == 0 || !bytes.HasPrefix(bytes.TrimSpace(values["components"]), []byte("[")) || json.Unmarshal(values["components"], &components) != nil {
		return result, ErrManifestJson
	}
	result.Components = []Component{}
	for _, raw := range components {
		values, err := fields(raw, []string{"kind", "path", "size", "sha256", "schema_version"}, 5)
		if err != nil {
			return result, err
		}
		item := Component{}
		item.Kind, err = componentKind(values["kind"])
		if err != nil || required(values, "path", &item.Path) != nil || required(values, "size", &item.Size) != nil || required(values, "sha256", &item.Sha256) != nil {
			return result, ErrManifestJson
		}
		if !slices.Contains([]string{"auth_database", "metadata", "index_database", "push_state"}, item.Kind) {
			return result, ErrManifestJson
		}
		if raw := values["schema_version"]; len(raw) > 0 && !bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			var version int64
			if bytes.Equal(bytes.TrimSpace(raw), []byte("-0")) || required(values, "schema_version", &version) != nil {
				return result, ErrManifestJson
			}
			item.SchemaVersion = &version
		}
		result.Components = append(result.Components, item)
	}
	return result, nil
}

func parsePath(value string) (string, error) {
	if value == "" || strings.Contains(value, "\\") {
		return "", failure("manifest", "component path is not portable")
	}
	for _, part := range strings.Split(value, "/") {
		if part == "" || part == "." || part == ".." || strings.Contains(part, ":") {
			return "", failure("manifest", "component path is unsafe")
		}
	}
	return filepath.FromSlash(value), nil
}
func componentKind(raw []byte) (string, error) {
	if !auth.ValidJson(string(raw)) {
		return "", ErrManifestJson
	}
	var name string
	if json.Unmarshal(raw, &name) == nil {
		return name, nil
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	opening, err := decoder.Token()
	if err != nil || opening != json.Delim('{') || !decoder.More() {
		return "", ErrManifestJson
	}
	key, err := decoder.Token()
	if err != nil {
		return "", ErrManifestJson
	}
	var payload json.RawMessage
	if decoder.Decode(&payload) != nil || !bytes.Equal(bytes.TrimSpace(payload), []byte("null")) || decoder.More() {
		return "", ErrManifestJson
	}
	name, ok := key.(string)
	if !ok {
		return "", ErrManifestJson
	}
	return name, nil
}
func extension(value string) string {
	name := path.Base(filepath.ToSlash(value))
	position := strings.LastIndexByte(name, '.')
	if position <= 0 {
		return ""
	}
	return name[position:]
}
func keyFile(value string) bool {
	suffix := []byte(extension(value))
	for index, character := range suffix {
		if character >= 'A' && character <= 'Z' {
			suffix[index] = character + 32
		}
	}
	return string(suffix) == ".key" || string(suffix) == ".pem"
}
func validateHeader(manifest Manifest) error {
	if manifest.Format != "litradar-backup" {
		return failure("unsupported", "manifest format identifier is unknown")
	}
	if manifest.Version < 1 || manifest.Version > FormatVersion {
		return failure("unsupported", "manifest version %d is not supported", manifest.Version)
	}
	if manifest.Version == 1 && manifest.Selection.Metadata || manifest.Version == 2 && !manifest.Selection.Metadata {
		return failure("manifest", "metadata selection does not match the manifest version")
	}
	if math.IsNaN(manifest.CreatedAt) || math.IsInf(manifest.CreatedAt, 0) || manifest.CreatedAt < 0 {
		return failure("manifest", "creation timestamp is invalid")
	}
	return nil
}
func validateLayout(item Component, selection Selection) error {
	if keyFile(item.Path) {
		return failure("manifest", "key files are forbidden in backups")
	}
	valid := false
	label := ""
	switch item.Kind {
	case "auth_database":
		label = "auth database"
		valid = item.Path == "auth.sqlite" && item.SchemaVersion != nil
	case "metadata":
		label = "metadata"
		valid = selection.Metadata && strings.HasPrefix(item.Path, "meta/") && item.SchemaVersion == nil
	case "index_database":
		label = "index database"
		valid = selection.IndexDatabases && path.Dir(item.Path) == "index" && extension(item.Path) == ".sqlite" && item.SchemaVersion != nil
	case "push_state":
		label = "push-state"
		valid = selection.PushState && (strings.HasPrefix(item.Path, "push_state/") || strings.HasPrefix(item.Path, "folder_push_state/")) && item.SchemaVersion == nil
	}
	if !valid {
		return failure("manifest", "%s component layout is invalid", label)
	}
	return nil
}

func databaseVersion(ctx context.Context, filename string) (int64, error) {
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		return 0, err
	}
	defer database.Close()
	var check storage.Text
	if err := database.QueryRowContext(ctx, "PRAGMA quick_check").Scan(&check); err != nil {
		return 0, err
	}
	if check != "ok" {
		return 0, failure("integrity", "SQLite quick_check failed")
	}
	var version storage.Integer
	err = database.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version)
	return int64(version), err
}
func validateFile(ctx context.Context, item Component, filename string) error {
	info, err := os.Lstat(filename)
	if err != nil {
		return failure("integrity", "component %s is missing", item.Path)
	}
	if !info.Mode().IsRegular() {
		return failure("integrity", "component %s is not a regular file", item.Path)
	}
	digest, err := hashFile(filename)
	if err != nil {
		return err
	}
	if uint64(info.Size()) != item.Size || digest != item.Sha256 {
		return failure("integrity", "component %s size or hash does not match", item.Path)
	}
	if item.Kind == "auth_database" || item.Kind == "index_database" {
		version, err := databaseVersion(ctx, filename)
		if err != nil {
			return err
		}
		if item.SchemaVersion == nil {
			return failure("manifest", "database schema version is missing")
		}
		maximum := int64(9)
		if item.Kind == "auth_database" {
			maximum = 20
		}
		if *item.SchemaVersion < 0 || *item.SchemaVersion > maximum {
			return failure("unsupported", "database schema version %d exceeds supported version %d", *item.SchemaVersion, maximum)
		}
		if version != *item.SchemaVersion {
			return failure("integrity", "component %s schema version does not match", item.Path)
		}
	}
	return nil
}

// Verify checks complete inventory, hashes, layout and SQLite integrity without migrating historical schemas.
func Verify(ctx context.Context, directory string) (Manifest, error) {
	info, err := os.Stat(directory)
	if err != nil || !info.IsDir() {
		return Manifest{}, failure("input", "backup directory does not exist")
	}
	raw, err := os.ReadFile(filepath.Join(directory, "manifest.json"))
	if err != nil {
		return Manifest{}, err
	}
	manifest, err := ParseManifest(raw)
	if err != nil {
		return Manifest{}, err
	}
	if err := validateHeader(manifest); err != nil {
		return Manifest{}, err
	}
	expected := map[string]bool{"manifest.json": true}
	seen := map[string]bool{}
	authCount := 0
	for _, item := range manifest.Components {
		relative, err := parsePath(item.Path)
		if err != nil {
			return Manifest{}, err
		}
		if err := validateLayout(item, manifest.Selection); err != nil {
			return Manifest{}, err
		}
		if seen[item.Path] {
			return Manifest{}, failure("manifest", "component paths must be unique")
		}
		seen[item.Path] = true
		expected[item.Path] = true
		if item.Kind == "auth_database" {
			authCount++
		}
		if err := validateFile(ctx, item, filepath.Join(directory, relative)); err != nil {
			return Manifest{}, err
		}
	}
	if authCount != 1 {
		return Manifest{}, failure("manifest", "exactly one auth database component is required")
	}
	actual, err := snapshot(directory)
	if err != nil {
		return Manifest{}, err
	}
	if len(actual) != len(expected) {
		return Manifest{}, failure("integrity", "backup contains missing or unlisted files")
	}
	for _, file := range actual {
		if !expected[file.Path] {
			return Manifest{}, failure("integrity", "backup contains missing or unlisted files")
		}
	}
	return manifest, nil
}
