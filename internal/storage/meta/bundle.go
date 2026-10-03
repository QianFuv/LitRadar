// Package meta synchronizes immutable catalog bundles without overwriting customized metadata.
package meta

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"unicode/utf8"

	auth "github.com/QianFuv/LitRadar/internal/domain/auth"
)

const manifestFilename = "bundle-manifest.json"
const headerV2 = "catalog_id,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating"
const headerV3 = "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating"

// InvalidBundle identifies unsafe or inconsistent immutable input before persistent writes.
type InvalidBundle struct{ Message string }

// Error distinguishes immutable bundle validation from persistent-state failures.
func (failure InvalidBundle) Error() string { return "invalid metadata bundle: " + failure.Message }

// HashMismatch reports only the catalog name and public content digests.
type HashMismatch struct{ Filename, Expected, Actual string }

// Error reports the catalog and expected/actual public digests.
func (failure HashMismatch) Error() string {
	return fmt.Sprintf("metadata bundle hash mismatch for %s: expected %s, found %s", failure.Filename, failure.Expected, failure.Actual)
}

// Downgrade refuses an image older than any recorded managed catalog.
type Downgrade struct{ StoredVersion, BundleVersion int64 }

// Error reports persistent and supplied versions without modifying either.
func (failure Downgrade) Error() string {
	return fmt.Sprintf("metadata bundle downgrade refused: persistent state uses version %d, but this image provides version %d", failure.StoredVersion, failure.BundleVersion)
}

// RollbackFailure preserves a failed compensation diagnostic without claiming transactionality across files.
type RollbackFailure struct{ Message string }

// Error identifies an incomplete filesystem compensation requiring recovery.
func (failure RollbackFailure) Error() string {
	return "metadata replacement rollback failed: " + failure.Message
}

type catalog struct {
	filename, sha256 string
	legacy           []string
	data             []byte
}
type bundle struct {
	version  int64
	catalogs []catalog
}

// CanonicalSha256 normalizes line endings and adds only a missing final LF, retaining BOM and repeated final LFs.
func CanonicalSha256(data []byte) (string, error) {
	if !utf8.Valid(data) {
		return "", errors.New("catalog is not valid UTF-8")
	}
	normalized := strings.ReplaceAll(strings.ReplaceAll(string(data), "\r\n", "\n"), "\r", "\n")
	if !strings.HasSuffix(normalized, "\n") {
		normalized += "\n"
	}
	digest := sha256.Sum256([]byte(normalized))
	return hex.EncodeToString(digest[:]), nil
}

// DiscoverPackagedDirectory checks only the fixed immutable runtime bundle location.
func DiscoverPackagedDirectory() (string, error) {
	directory := "/usr/share/litradar/meta"
	_, err := os.Stat(filepath.Join(directory, manifestFilename))
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	return directory, nil
}

func object(raw []byte, fields ...string) (map[string]json.RawMessage, error) {
	invalid := errors.New("invalid metadata bundle manifest")
	if !auth.ValidJson(string(raw)) {
		return nil, invalid
	}
	if bytes.TrimSpace(raw)[0] == '[' {
		var elements []json.RawMessage
		if json.Unmarshal(raw, &elements) != nil || len(elements) != len(fields) {
			return nil, invalid
		}
		values := make(map[string]json.RawMessage, len(fields))
		for index, field := range fields {
			if bytes.Equal(bytes.TrimSpace(elements[index]), []byte("null")) {
				return nil, invalid
			}
			values[field] = elements[index]
		}
		return values, nil
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	opening, err := decoder.Token()
	if err != nil || opening != json.Delim('{') {
		return nil, invalid
	}
	values := map[string]json.RawMessage{}
	for decoder.More() {
		key, err := decoder.Token()
		if err != nil {
			return nil, invalid
		}
		name, ok := key.(string)
		if !ok || !slices.Contains(fields, name) || values[name] != nil {
			return nil, invalid
		}
		var value json.RawMessage
		if decoder.Decode(&value) != nil || bytes.Equal(bytes.TrimSpace(value), []byte("null")) {
			return nil, invalid
		}
		values[name] = value
	}
	if len(values) != len(fields) {
		return nil, invalid
	}
	return values, nil
}

func regularBundleFile(filename, name string) error {
	info, err := os.Lstat(filename)
	if errors.Is(err, os.ErrNotExist) {
		return InvalidBundle{"missing bundled file " + name}
	}
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() {
		return InvalidBundle{"bundled path " + name + " must be a regular file"}
	}
	return nil
}

func validDigest(value string) bool {
	if len(value) != 64 {
		return false
	}
	for _, character := range value {
		if !(character >= '0' && character <= '9' || character >= 'a' && character <= 'f') {
			return false
		}
	}
	return true
}

func validateBundle(directory string) (bundle, error) {
	filename := filepath.Join(directory, manifestFilename)
	if err := regularBundleFile(filename, manifestFilename); err != nil {
		return bundle{}, err
	}
	data, err := os.ReadFile(filename)
	if err != nil {
		return bundle{}, err
	}
	fields, err := object(data, "format", "version", "catalogs")
	if err != nil {
		return bundle{}, err
	}
	var format string
	var version int64
	var entries []json.RawMessage
	if json.Unmarshal(fields["format"], &format) != nil || json.Unmarshal(fields["version"], &version) != nil || json.Unmarshal(fields["catalogs"], &entries) != nil {
		return bundle{}, errors.New("invalid metadata bundle manifest")
	}
	if format != "litradar-meta-bundle" {
		return bundle{}, InvalidBundle{"unsupported format " + format}
	}
	if version <= 0 {
		return bundle{}, InvalidBundle{"version must be a positive integer"}
	}
	if len(entries) == 0 {
		return bundle{}, InvalidBundle{"catalog inventory must not be empty"}
	}
	result := bundle{version: version, catalogs: []catalog{}}
	names := map[string]bool{}
	for _, entry := range entries {
		fields, err := object(entry, "filename", "sha256", "legacy_sha256")
		if err != nil {
			return bundle{}, err
		}
		var candidate catalog
		if json.Unmarshal(fields["filename"], &candidate.filename) != nil || json.Unmarshal(fields["sha256"], &candidate.sha256) != nil || json.Unmarshal(fields["legacy_sha256"], &candidate.legacy) != nil {
			return bundle{}, errors.New("invalid metadata bundle manifest")
		}
		name := candidate.filename
		if name == "" || name == ".csv" || strings.ContainsAny(name, "/\\") || filepath.VolumeName(name) != "" || filepath.Base(name) != name || filepath.Ext(name) != ".csv" {
			return bundle{}, InvalidBundle{fmt.Sprintf("catalog filename %q must be a portable CSV basename", name)}
		}
		if names[name] {
			return bundle{}, InvalidBundle{"duplicate catalog filename " + name}
		}
		names[name] = true
		if !validDigest(candidate.sha256) {
			return bundle{}, InvalidBundle{"catalog " + name + " contains an invalid SHA-256 digest"}
		}
		digests := map[string]bool{candidate.sha256: true}
		for _, digest := range candidate.legacy {
			if !validDigest(digest) {
				return bundle{}, InvalidBundle{"catalog " + name + " contains an invalid SHA-256 digest"}
			}
			if digests[digest] {
				return bundle{}, InvalidBundle{"duplicate current or legacy hash for " + name}
			}
			digests[digest] = true
		}
		location := filepath.Join(directory, name)
		if err := regularBundleFile(location, name); err != nil {
			return bundle{}, err
		}
		candidate.data, err = os.ReadFile(location)
		if err != nil {
			return bundle{}, err
		}
		if !utf8.Valid(candidate.data) {
			return bundle{}, InvalidBundle{"catalog " + name + " is not valid UTF-8"}
		}
		if version >= 2 {
			header, _, _ := strings.Cut(string(candidate.data), "\n")
			header = strings.TrimRight(header, "\r")
			expected, headerVersion := headerV2, 2
			if version >= 3 {
				expected, headerVersion = headerV3, 3
			}
			if header != expected {
				return bundle{}, InvalidBundle{fmt.Sprintf("catalog %s must use the exact canonical v%d header", name, headerVersion)}
			}
		}
		actual, _ := CanonicalSha256(candidate.data)
		if actual != candidate.sha256 {
			return bundle{}, HashMismatch{name, candidate.sha256, actual}
		}
		result.catalogs = append(result.catalogs, candidate)
	}
	slices.SortFunc(result.catalogs, func(first, second catalog) int { return strings.Compare(first.filename, second.filename) })
	return result, nil
}
