package config

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// TestDatabaseSelectionKeepsContentAndControlSeparate checks basename selection and isolated credential overrides.
func TestDatabaseSelectionKeepsContentAndControlSeparate(t *testing.T) {
	config := FromProjectRoot(t.TempDir())
	if _, err := config.ResolveIndexDbPath(nil); !errors.Is(err, ErrNoDatabases) {
		t.Fatal(err)
	}
	createContentAndControlFixture(t, config)
	expected := filepath.Join(config.IndexDir, "catalog.sqlite")
	assertDatabaseSelectionBasenames(t, config, expected)
	missing := "../missing"
	if _, err := config.ResolveIndexDbPath(&missing); !errors.Is(err, ErrNotFound) {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.IndexDir, "second.sqlite"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := config.ResolveIndexDbPath(nil); !errors.Is(err, ErrMultipleDatabases) {
		t.Fatal(err)
	}
	if changed := config.WithAuthDbPath("state/auth.sqlite"); changed.AuthDbPath != "state/auth.sqlite" || changed.IndexDir != config.IndexDir || changed.MetaDir != config.MetaDir {
		t.Fatal("auth override moved unrelated data")
	}
}

// TestProviderCatalogDiscoveryFiltersWithoutChangingIndexDiscovery preserves the two discovery admission policies.
func TestProviderCatalogDiscoveryFiltersWithoutChangingIndexDiscovery(t *testing.T) {
	config := FromProjectRoot(t.TempDir())
	createProviderDiscoveryFixture(t, config)
	catalogs, err := config.ListProviderCatalogs()
	if err != nil {
		t.Fatal(err)
	}
	stems := []string{}
	for _, catalog := range catalogs {
		stems = append(stems, catalog.Stem)
	}
	if !reflect.DeepEqual(stems, []string{"csv_only", "database_only", "paired"}) || catalogs[2].CsvFilename == nil || catalogs[2].DatabaseFilename == nil {
		t.Fatalf("%+v", catalogs)
	}
	files, err := config.ListIndexDatabases()
	if err != nil || len(files) != 3 {
		t.Fatalf("legacy discovery changed: %v %v", files, err)
	}
}

// createContentAndControlFixture keeps matching database fixtures in separate directories.
func createContentAndControlFixture(t *testing.T, config Config) {
	t.Helper()
	for _, directory := range []string{config.IndexDir, config.IndexControlDir} {
		if err := os.MkdirAll(directory, 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(directory, "catalog.sqlite"), nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
}

// assertDatabaseSelectionBasenames retains implicit and host-platform basename selections.
func assertDatabaseSelectionBasenames(t *testing.T, config Config, expected string) {
	t.Helper()
	for _, selection := range []string{"", "  ", "catalog", "catalog.sqlite", "../catalog", "catalog/.", "..", "/"} {
		if actual, err := config.ResolveIndexDbPath(&selection); err != nil || actual != expected {
			t.Fatalf("%q: %s %v", selection, actual, err)
		}
	}
}

// createProviderDiscoveryFixture supplies safe, rejected and legacy-directory entries in original order.
func createProviderDiscoveryFixture(t *testing.T, config Config) {
	t.Helper()
	for _, directory := range []string{config.IndexDir, config.MetaDir} {
		if err := os.MkdirAll(directory, 0700); err != nil {
			t.Fatal(err)
		}
	}
	for _, name := range []string{"paired.csv", "csv_only.csv", "Unsafe.csv", ".csv", "a.csv"} {
		if err := os.WriteFile(filepath.Join(config.MetaDir, name), nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
	for _, name := range []string{"paired.sqlite", "database_only.sqlite", ".sqlite"} {
		if err := os.WriteFile(filepath.Join(config.IndexDir, name), nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Mkdir(filepath.Join(config.IndexDir, "directory.sqlite"), 0700); err != nil {
		t.Fatal(err)
	}
}
