package config

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

// writeConfigEntry creates an isolated discovery fixture without database contents.
func writeConfigEntry(t *testing.T, directory, name string) {
	t.Helper()
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, name), nil, 0600); err != nil {
		t.Fatal(err)
	}
}

// TestProviderDiscoveryKeepsEmptySliceAndFirstReadFailure checks missing-source and failure representation.
func TestProviderDiscoveryKeepsEmptySliceAndFirstReadFailure(t *testing.T) {
	configuration := FromProjectRoot(t.TempDir())
	catalogs, err := configuration.ListProviderCatalogs()
	if err != nil || catalogs == nil || len(catalogs) != 0 {
		t.Fatalf("%+v %v", catalogs, err)
	}
	writeConfigEntry(t, configuration.IndexDir, "alpha.sqlite")
	catalogs, err = configuration.ListProviderCatalogs()
	if err != nil || len(catalogs) != 1 {
		t.Fatalf("%+v %v", catalogs, err)
	}
	if err := os.WriteFile(configuration.MetaDir, nil, 0600); err != nil {
		t.Fatal(err)
	}
	catalogs, err = configuration.ListProviderCatalogs()
	var pathError *os.PathError
	if !errors.As(err, &pathError) || pathError.Path != configuration.MetaDir || catalogs != nil {
		t.Fatalf("%+v %v", catalogs, err)
	}
}

// assertProviderFilename checks basename presence and exact value.
func assertProviderFilename(t *testing.T, filename *string, expected string) {
	t.Helper()
	if filename == nil || *filename != expected {
		t.Fatalf("filename=%v, expected %q", filename, expected)
	}
}

// assertPairedProviderCatalog checks both independent basename fields.
func assertPairedProviderCatalog(t *testing.T, catalog domain.ProviderCatalog, stem string) {
	t.Helper()
	if catalog.Stem != stem {
		t.Fatalf("%+v", catalog)
	}
	assertProviderFilename(t, catalog.CsvFilename, stem+".csv")
	assertProviderFilename(t, catalog.DatabaseFilename, stem+".sqlite")
	if catalog.CsvFilename == catalog.DatabaseFilename {
		t.Fatal("paired filenames share storage")
	}
}

// TestProviderCatalogFilenamesRemainIndependent checks field merging and per-entry pointer ownership.
func TestProviderCatalogFilenamesRemainIndependent(t *testing.T) {
	configuration := FromProjectRoot(t.TempDir())
	for _, name := range []string{"alpha", "beta"} {
		writeConfigEntry(t, configuration.MetaDir, name+".csv")
		writeConfigEntry(t, configuration.IndexDir, name+".sqlite")
	}
	catalogs, err := configuration.ListProviderCatalogs()
	if err != nil || len(catalogs) != 2 {
		t.Fatalf("%+v %v", catalogs, err)
	}
	assertPairedProviderCatalog(t, catalogs[0], "alpha")
	assertPairedProviderCatalog(t, catalogs[1], "beta")
	if catalogs[0].CsvFilename == catalogs[1].CsvFilename || catalogs[0].DatabaseFilename == catalogs[1].DatabaseFilename {
		t.Fatal("entries share filename storage")
	}
}

// TestRuntimeNameKeepsAsciiByteBoundaries checks maintained grammar without platform-reserved-name rules.
func TestRuntimeNameKeepsAsciiByteBoundaries(t *testing.T) {
	cases := []struct {
		name    string
		isValid bool
	}{
		{"", false}, {"a", false}, {"ab", true}, {"1a", true}, {".a", false}, {"_a", false}, {"-a", false},
		{"a.", true}, {"a_", true}, {"a-", true}, {"a..__", true}, {"aA", false}, {"A1", false},
		{"a ", false}, {"a\n", false}, {"a\x00", false}, {"aé", false}, {"a中", false}, {"a\xff", false},
		{"nul", true}, {"con", true}, {strings.Repeat("a", 128), true}, {strings.Repeat("a", 129), false},
	}
	for _, test := range cases {
		if actual := IsRuntimeName(test.name); actual != test.isValid {
			t.Fatalf("%q: %t, expected %t", test.name, actual, test.isValid)
		}
	}
}

// TestCanonicalStemAdmissionFollowsOrdinarySelection checks explicit unsafe-stem selection precedence.
func TestCanonicalStemAdmissionFollowsOrdinarySelection(t *testing.T) {
	configuration := FromProjectRoot(t.TempDir())
	writeConfigEntry(t, configuration.IndexDir, "Unsafe.sqlite")
	selection := "Unsafe"
	actual, err := configuration.ResolveIndexDbPath(&selection)
	if err != nil || actual != filepath.Join(configuration.IndexDir, "Unsafe.sqlite") {
		t.Fatalf("%q %v", actual, err)
	}
	_, err = configuration.ResolveIndexCatalogStem(&selection)
	if !errors.Is(err, ErrInvalidName) {
		t.Fatal(err)
	}
	missing := "missing"
	_, err = configuration.ResolveIndexCatalogStem(&missing)
	if !errors.Is(err, ErrNotFound) {
		t.Fatal(err)
	}
}
