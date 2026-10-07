package runtime

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

const developmentCsp = "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"
const cspManifestLimit = 4 * 1024 * 1024

type cspManifest struct {
	Version      uint32    `json:"version"`
	Algorithm    string    `json:"algorithm"`
	Files        []cspFile `json:"files"`
	ScriptHashes []string  `json:"script_hashes"`
}
type cspFile struct {
	Path               string   `json:"path"`
	HtmlSha256         string   `json:"html_sha256"`
	InlineScriptHashes []string `json:"inline_script_hashes"`
}

func loadSecurityPolicy(root string) (string, error) {
	expected, err := buildCspManifest(root)
	if err != nil {
		return "", err
	}
	filename := filepath.Join(root, "csp-hashes.json")
	metadata, err := os.Lstat(filename)
	if err != nil {
		return "", fmt.Errorf("Unable to read CSP manifest metadata at %s: %w", filename, err)
	}
	if !metadata.Mode().IsRegular() {
		return "", fmt.Errorf("CSP manifest must be a regular file: %s", filename)
	}
	if metadata.Size() > cspManifestLimit {
		return "", fmt.Errorf("CSP manifest exceeds the %d byte limit", cspManifestLimit)
	}
	data, err := os.ReadFile(filename)
	if err != nil {
		return "", fmt.Errorf("Unable to read CSP manifest at %s: %w", filename, err)
	}
	var deployed cspManifest
	if err := json.Unmarshal(data, &deployed); err != nil {
		return "", fmt.Errorf("Invalid CSP manifest: %w", err)
	}
	if !reflect.DeepEqual(expected, deployed) {
		return "", errors.New("CSP manifest does not match the deployed static HTML")
	}
	policy := "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self'"
	for _, hash := range expected.ScriptHashes {
		policy += " '" + hash + "'"
	}
	return policy + "; style-src 'self' 'unsafe-inline'", nil
}

func buildCspManifest(root string) (cspManifest, error) {
	manifest := cspManifest{Version: 1, Algorithm: "sha256", Files: []cspFile{}, ScriptHashes: []string{}}
	paths, err := collectHtmlPaths(root, root)
	if err != nil {
		return manifest, err
	}
	slices.Sort(paths)
	if len(paths) == 0 {
		return manifest, fmt.Errorf("Static export contains no HTML files: %s", root)
	}
	for _, filename := range paths {
		if err := appendCspFile(&manifest, root, filename); err != nil {
			return manifest, err
		}
	}
	slices.SortFunc(manifest.Files, func(first, second cspFile) int { return strings.Compare(first.Path, second.Path) })
	slices.Sort(manifest.ScriptHashes)
	return manifest, nil
}

func collectHtmlPaths(root, directory string) ([]string, error) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return nil, fmt.Errorf("Unable to traverse static export at %s: %w", directory, err)
	}
	paths := []string{}
	for _, entry := range entries {
		filename := filepath.Join(directory, entry.Name())
		if entry.Type()&os.ModeSymlink != 0 {
			return nil, fmt.Errorf("Static export must not contain symbolic links: %s", filename)
		}
		if entry.IsDir() {
			nested, err := collectHtmlPaths(root, filename)
			if err != nil {
				return nil, err
			}
			paths = append(paths, nested...)
		} else if entry.Type().IsRegular() && strings.EqualFold(filepath.Ext(filename), ".html") {
			paths = append(paths, filename)
		}
	}
	return paths, nil
}

func cspDigest(value []byte) string {
	digest := sha256.Sum256(value)
	return "sha256-" + base64.StdEncoding.EncodeToString(digest[:])
}

func inlineScriptHashes(html []byte) ([]string, error) {
	hashes := []string{}
	cursor := 0
	for {
		opening := findScriptTag(html, cursor, false)
		if opening < 0 {
			return hashes, nil
		}
		end, err := scriptOpeningEnd(html, opening+len("<script"))
		if err != nil {
			return nil, err
		}
		closing := findScriptTag(html, end+1, true)
		if closing < 0 {
			return nil, errors.New("Static HTML contains an unterminated script element")
		}
		closingEnd := closing + len("</script")
		for closingEnd < len(html) && htmlWhitespace(html[closingEnd]) {
			closingEnd++
		}
		if closingEnd >= len(html) || html[closingEnd] != '>' {
			return nil, errors.New("Static HTML contains an invalid script closing tag")
		}
		if !hasScriptSource(html[opening : end+1]) {
			hashes = append(hashes, cspDigest(html[end+1:closing]))
		}
		cursor = closingEnd + 1
	}
}

func findScriptTag(html []byte, start int, isClosing bool) int {
	needle := []byte("<script")
	if isClosing {
		needle = []byte("</script")
	}
	for index := start; index+len(needle) < len(html); index++ {
		if !asciiEqualFold(html[index:index+len(needle)], needle) {
			continue
		}
		boundary := html[index+len(needle)]
		if htmlWhitespace(boundary) || boundary == '>' || !isClosing && boundary == '/' {
			return index
		}
	}
	return -1
}

func scriptOpeningEnd(html []byte, start int) (int, error) {
	var quote byte
	for index := start; index < len(html); index++ {
		character := html[index]
		if quote != 0 {
			if character == quote {
				quote = 0
			}
		} else if character == '\'' || character == '"' {
			quote = character
		} else if character == '>' {
			return index, nil
		}
	}
	return 0, errors.New("Static HTML contains an unterminated script opening tag")
}

func hasScriptSource(tag []byte) bool {
	cursor := len("<script")
	for cursor+1 < len(tag) {
		cursor = skipScriptAttributeSeparators(tag, cursor)
		start := cursor
		cursor = scriptAttributeNameEnd(tag, cursor)
		if cursor == start {
			cursor++
			continue
		}
		if asciiEqualFold(tag[start:cursor], []byte("src")) {
			return true
		}
		cursor = scriptAttributeValueEnd(tag, cursor)
	}
	return false
}

func htmlWhitespace(character byte) bool {
	return character == ' ' || character == '\t' || character == '\n' || character == '\r' || character == '\f'
}

func asciiEqualFold(first, second []byte) bool {
	if len(first) != len(second) {
		return false
	}
	for index, character := range first {
		if character >= 'A' && character <= 'Z' {
			character += 32
		}
		other := second[index]
		if other >= 'A' && other <= 'Z' {
			other += 32
		}
		if character != other {
			return false
		}
	}
	return true
}

func cspFields(data []byte, names []string) ([]json.RawMessage, error) {
	if !jsonvalue.ValidJson(string(data)) {
		return nil, errors.New("invalid manifest JSON")
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	token, err := decoder.Token()
	if err != nil {
		return nil, err
	}
	result := make([]json.RawMessage, len(names))
	switch token {
	case json.Delim('{'):
		if err := decodeCspObject(decoder, names, result); err != nil {
			return nil, err
		}
	case json.Delim('['):
		if err := decodeCspSequence(decoder, names, result); err != nil {
			return nil, err
		}
	default:
		return nil, errors.New("manifest must be a struct")
	}
	if _, err := decoder.Token(); err != nil {
		return nil, err
	}
	if _, err := decoder.Token(); err != io.EOF {
		return nil, errors.New("trailing manifest data")
	}
	if err := requireCspFields(result, names); err != nil {
		return nil, err
	}
	return result, nil
}

func (manifest *cspManifest) UnmarshalJSON(data []byte) error {
	fields, err := cspFields(data, []string{"version", "algorithm", "files", "script_hashes"})
	if err != nil {
		return err
	}
	for index, target := range []any{&manifest.Version, &manifest.Algorithm, &manifest.Files, &manifest.ScriptHashes} {
		if err := json.Unmarshal(fields[index], target); err != nil {
			return err
		}
	}
	return nil
}
func (file *cspFile) UnmarshalJSON(data []byte) error {
	fields, err := cspFields(data, []string{"path", "html_sha256", "inline_script_hashes"})
	if err != nil {
		return err
	}
	for index, target := range []any{&file.Path, &file.HtmlSha256, &file.InlineScriptHashes} {
		if err := json.Unmarshal(fields[index], target); err != nil {
			return err
		}
	}
	return nil
}

// appendCspFile retains aggregate hash mutation before relative-path validation and file append.
func appendCspFile(manifest *cspManifest, root, filename string) error {
	data, err := os.ReadFile(filename)
	if err != nil {
		return fmt.Errorf("Unable to read static HTML at %s: %w", filename, err)
	}
	if !utf8.Valid(data) {
		return fmt.Errorf("Static HTML must be valid UTF-8: %s", filename)
	}
	hashes, err := inlineScriptHashes(data)
	if err != nil {
		return err
	}
	for _, hash := range hashes {
		if !slices.Contains(manifest.ScriptHashes, hash) {
			manifest.ScriptHashes = append(manifest.ScriptHashes, hash)
		}
	}
	relative, err := filepath.Rel(root, filename)
	if err != nil {
		return errors.New("Static HTML escaped the export root")
	}
	if !utf8.ValidString(relative) {
		return errors.New("Static HTML path must be valid UTF-8")
	}
	manifest.Files = append(manifest.Files, cspFile{strings.ReplaceAll(relative, "\\", "/"), cspDigest(data), hashes})
	return nil
}

// skipScriptAttributeSeparators advances only over original HTML whitespace and slash bytes.
func skipScriptAttributeSeparators(tag []byte, cursor int) int {
	for cursor < len(tag) && (htmlWhitespace(tag[cursor]) || tag[cursor] == '/') {
		cursor++
	}
	return cursor
}

// scriptAttributeNameEnd consumes the exact original attribute-name byte grammar.
func scriptAttributeNameEnd(tag []byte, cursor int) int {
	for cursor < len(tag) && !htmlWhitespace(tag[cursor]) && !strings.ContainsRune("=/>", rune(tag[cursor])) {
		cursor++
	}
	return cursor
}

// scriptAttributeValueEnd skips a supplied quoted or unquoted value after equals admission.
func scriptAttributeValueEnd(tag []byte, cursor int) int {
	cursor = skipScriptAttributeWhitespace(tag, cursor)
	if cursor >= len(tag) || tag[cursor] != '=' {
		return cursor
	}
	cursor++
	cursor = skipScriptAttributeWhitespace(tag, cursor)
	if cursor < len(tag) && (tag[cursor] == '\'' || tag[cursor] == '"') {
		cursor = scriptQuotedValueEnd(tag, cursor)
	} else {
		for cursor < len(tag) && !htmlWhitespace(tag[cursor]) && tag[cursor] != '>' {
			cursor++
		}
	}
	return cursor
}

// scriptQuotedValueEnd consumes through an optional matching quote without interpreting bytes.
func scriptQuotedValueEnd(tag []byte, cursor int) int {
	quote := tag[cursor]
	cursor++
	for cursor < len(tag) && tag[cursor] != quote {
		cursor++
	}
	if cursor < len(tag) {
		cursor++
	}
	return cursor
}

// decodeCspObject rejects unknown and duplicate names before decoding their values.
func decodeCspObject(decoder *json.Decoder, names []string, result []json.RawMessage) error {
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return err
		}
		name, ok := token.(string)
		if !ok {
			return errors.New("invalid manifest field")
		}
		index := slices.Index(names, name)
		if index < 0 {
			return fmt.Errorf("unknown field %s", name)
		}
		if result[index] != nil {
			return fmt.Errorf("duplicate field %s", name)
		}
		if err := decoder.Decode(&result[index]); err != nil {
			return err
		}
	}
	return nil
}

// decodeCspSequence rejects excess positional values before reading their contents.
func decodeCspSequence(decoder *json.Decoder, names []string, result []json.RawMessage) error {
	index := 0
	for decoder.More() {
		if index >= len(names) {
			return errors.New("invalid manifest sequence length")
		}
		if err := decoder.Decode(&result[index]); err != nil {
			return err
		}
		index++
	}
	return nil
}

// requireCspFields checks missing and null fields in the supplied declaration order.
func requireCspFields(result []json.RawMessage, names []string) error {
	for index, value := range result {
		if value == nil || bytes.Equal(bytes.TrimSpace(value), []byte("null")) {
			return fmt.Errorf("missing or null field %s", names[index])
		}
	}
	return nil
}

// skipScriptAttributeWhitespace consumes the original five HTML whitespace bytes.
func skipScriptAttributeWhitespace(tag []byte, cursor int) int {
	for cursor < len(tag) && htmlWhitespace(tag[cursor]) {
		cursor++
	}
	return cursor
}
