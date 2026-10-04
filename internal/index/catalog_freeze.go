package index

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"unicode"
	"unicode/utf8"

	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// FreezeCatalog hashes and parses one exact file read before batch admission can mutate durable state.
func FreezeCatalog(path, provider string) (storage.CatalogInput, error) {
	filename := frozenCatalogBasename(path)
	if path == "" || filename == "." || filename == ".." || filename == string(filepath.Separator) || !utf8.ValidString(filename) {
		return storage.CatalogInput{}, &storage.BatchError{Kind: "input", Reason: "catalog path must have a non-empty UTF-8 basename"}
	}
	validate := func(value, reason string) error {
		if value == "" || len(value) > 512 || !utf8.ValidString(value) {
			return &storage.BatchError{Kind: "input", Reason: reason}
		}
		for _, character := range value {
			if unicode.IsControl(character) {
				return &storage.BatchError{Kind: "input", Reason: reason}
			}
		}
		return nil
	}
	if err := validate(filename, "catalog filename must be non-empty and bounded"); err != nil {
		return storage.CatalogInput{}, err
	}
	name := filename
	if extension := filepath.Ext(name); extension != "" && extension != name {
		name = strings.TrimSuffix(name, extension)
	}
	if name == "" {
		return storage.CatalogInput{}, &storage.BatchError{Kind: "input", Reason: "catalog filename must have a non-empty UTF-8 stem"}
	}
	if err := validate(provider, "provider route must be non-empty and bounded"); err != nil {
		return storage.CatalogInput{}, err
	}
	body, err := os.ReadFile(path)
	if err != nil {
		return storage.CatalogInput{}, err
	}
	digest := sha256.Sum256(body)
	if !utf8.Valid(body) {
		return storage.CatalogInput{}, fmt.Errorf("catalog %s is not valid UTF-8", filename)
	}
	entries, err := ParseCatalogCsv(string(body))
	if err != nil {
		return storage.CatalogInput{}, err
	}
	return storage.CatalogInput{Path: path, Filename: filename, CatalogName: name, CsvSha256: hex.EncodeToString(digest[:]), ProviderName: provider, Entries: entries}, nil
}

func frozenCatalogBasename(path string) string {
	value := path
	for {
		value = strings.TrimRightFunc(value, func(character rune) bool {
			return character == '/' || os.IsPathSeparator(uint8(character)) && character < 128
		})
		name := filepath.Base(value)
		if name != "." || value == "." || value == "" {
			return name
		}
		value = strings.TrimSuffix(value, ".")
	}
}
