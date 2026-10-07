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

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
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

// DiscoverPackagedDirectory checks system and executable-relative immutable bundles.
func DiscoverPackagedDirectory() (string, error) {
	executable, err := os.Executable()
	if err != nil {
		return "", err
	}
	return findPackagedDirectory([]string{"/usr/share/litradar/meta", filepath.Join(filepath.Dir(executable), "assets", "meta")})
}

func findPackagedDirectory(directories []string) (string, error) {
	for _, directory := range directories {
		_, err := os.Stat(filepath.Join(directory, manifestFilename))
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return "", err
		}
		return directory, nil
	}
	return "", nil
}

// object requires every declared field exactly once and rejects null or unknown values.
func object(raw []byte, fields ...string) (map[string]json.RawMessage, error) {
	invalid := errors.New("invalid metadata bundle manifest")
	if !jsonvalue.ValidJson(string(raw)) {
		return nil, invalid
	}
	if bytes.TrimSpace(raw)[0] == '[' {
		return sequenceObject(raw, fields, invalid)
	}
	return namedObject(raw, fields, invalid)
}

// sequenceObject maps the original positional form only after exact arity and value admission.
func sequenceObject(raw []byte, fields []string, invalid error) (map[string]json.RawMessage, error) {
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

// namedObject decodes admitted fields in encounter order and requires a complete object.
func namedObject(raw []byte, fields []string, invalid error) (map[string]json.RawMessage, error) {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	opening, err := decoder.Token()
	if err != nil || opening != json.Delim('{') {
		return nil, invalid
	}
	values := map[string]json.RawMessage{}
	for decoder.More() {
		if err := decodeObjectField(decoder, fields, values, invalid); err != nil {
			return nil, err
		}
	}
	if len(values) != len(fields) {
		return nil, invalid
	}
	return values, nil
}

// decodeObjectField validates the key before consuming its nonnull raw value.
func decodeObjectField(decoder *json.Decoder, fields []string, values map[string]json.RawMessage, invalid error) error {
	key, err := decoder.Token()
	if err != nil {
		return invalid
	}
	name, ok := key.(string)
	if !ok || !slices.Contains(fields, name) || values[name] != nil {
		return invalid
	}
	var value json.RawMessage
	if decoder.Decode(&value) != nil || bytes.Equal(bytes.TrimSpace(value), []byte("null")) {
		return invalid
	}
	values[name] = value
	return nil
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

// validateBundle admits catalogs in manifest order and sorts only the fully valid bundle.
func validateBundle(directory string) (bundle, error) {
	result, entries, err := readBundleManifest(directory)
	if err != nil {
		return bundle{}, err
	}
	names := map[string]bool{}
	for _, entry := range entries {
		candidate, err := validateCatalog(directory, result.version, entry, names)
		if err != nil {
			return bundle{}, err
		}
		result.catalogs = append(result.catalogs, candidate)
	}
	slices.SortFunc(result.catalogs, func(first, second catalog) int { return strings.Compare(first.filename, second.filename) })
	return result, nil
}

// readBundleManifest decodes all header types before admitting format, version and inventory.
func readBundleManifest(directory string) (bundle, []json.RawMessage, error) {
	filename := filepath.Join(directory, manifestFilename)
	if err := regularBundleFile(filename, manifestFilename); err != nil {
		return bundle{}, nil, err
	}
	data, err := os.ReadFile(filename)
	if err != nil {
		return bundle{}, nil, err
	}
	fields, err := object(data, "format", "version", "catalogs")
	if err != nil {
		return bundle{}, nil, err
	}
	var format string
	var version int64
	var entries []json.RawMessage
	if json.Unmarshal(fields["format"], &format) != nil || json.Unmarshal(fields["version"], &version) != nil || json.Unmarshal(fields["catalogs"], &entries) != nil {
		return bundle{}, nil, errors.New("invalid metadata bundle manifest")
	}
	if format != "litradar-meta-bundle" {
		return bundle{}, nil, InvalidBundle{"unsupported format " + format}
	}
	if version <= 0 {
		return bundle{}, nil, InvalidBundle{"version must be a positive integer"}
	}
	if len(entries) == 0 {
		return bundle{}, nil, InvalidBundle{"catalog inventory must not be empty"}
	}
	return bundle{version: version, catalogs: []catalog{}}, entries, nil
}

// validateCatalog admits one complete descriptor before reading and checking its file.
func validateCatalog(directory string, version int64, entry json.RawMessage, names map[string]bool) (catalog, error) {
	candidate, err := decodeCatalog(entry)
	if err != nil {
		return catalog{}, err
	}
	if err := admitCatalogName(candidate.filename, names); err != nil {
		return catalog{}, err
	}
	if err := admitCatalogDigests(candidate); err != nil {
		return catalog{}, err
	}
	location := filepath.Join(directory, candidate.filename)
	if err := regularBundleFile(location, candidate.filename); err != nil {
		return catalog{}, err
	}
	candidate.data, err = os.ReadFile(location)
	if err != nil {
		return catalog{}, err
	}
	if err := validateCatalogData(candidate, version); err != nil {
		return catalog{}, err
	}
	return candidate, nil
}

// decodeCatalog retains strict object/sequence fields and sequential typed decoding.
func decodeCatalog(entry json.RawMessage) (catalog, error) {
	fields, err := object(entry, "filename", "sha256", "legacy_sha256")
	if err != nil {
		return catalog{}, err
	}
	var candidate catalog
	if json.Unmarshal(fields["filename"], &candidate.filename) != nil || json.Unmarshal(fields["sha256"], &candidate.sha256) != nil || json.Unmarshal(fields["legacy_sha256"], &candidate.legacy) != nil {
		return catalog{}, errors.New("invalid metadata bundle manifest")
	}
	return candidate, nil
}

// admitCatalogName checks portable CSV names before recording duplicate admission.
func admitCatalogName(name string, names map[string]bool) error {
	if name == "" || name == ".csv" || strings.ContainsAny(name, "/\\") || filepath.VolumeName(name) != "" || filepath.Base(name) != name || filepath.Ext(name) != ".csv" {
		return InvalidBundle{fmt.Sprintf("catalog filename %q must be a portable CSV basename", name)}
	}
	if names[name] {
		return InvalidBundle{"duplicate catalog filename " + name}
	}
	names[name] = true
	return nil
}

// admitCatalogDigests checks the current digest before ordered legacy syntax and duplicates.
func admitCatalogDigests(candidate catalog) error {
	name := candidate.filename
	if !validDigest(candidate.sha256) {
		return InvalidBundle{"catalog " + name + " contains an invalid SHA-256 digest"}
	}
	digests := map[string]bool{candidate.sha256: true}
	for _, digest := range candidate.legacy {
		if !validDigest(digest) {
			return InvalidBundle{"catalog " + name + " contains an invalid SHA-256 digest"}
		}
		if digests[digest] {
			return InvalidBundle{"duplicate current or legacy hash for " + name}
		}
		digests[digest] = true
	}
	return nil
}

// validateCatalogData checks original UTF-8/header bytes before their canonical content digest.
func validateCatalogData(candidate catalog, version int64) error {
	name := candidate.filename
	if !utf8.Valid(candidate.data) {
		return InvalidBundle{"catalog " + name + " is not valid UTF-8"}
	}
	if version >= 2 {
		if err := validateCatalogHeader(candidate, version); err != nil {
			return err
		}
	}
	actual, _ := CanonicalSha256(candidate.data)
	if actual != candidate.sha256 {
		return HashMismatch{name, candidate.sha256, actual}
	}
	return nil
}

// validateCatalogHeader requires the exact v2 or v3 first line without trimming a BOM.
func validateCatalogHeader(candidate catalog, version int64) error {
	header, _, _ := strings.Cut(string(candidate.data), "\n")
	header = strings.TrimRight(header, "\r")
	expected, headerVersion := headerV2, 2
	if version >= 3 {
		expected, headerVersion = headerV3, 3
	}
	if header != expected {
		return InvalidBundle{fmt.Sprintf("catalog %s must use the exact canonical v%d header", candidate.filename, headerVersion)}
	}
	return nil
}
