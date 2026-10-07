package meta

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

// TestCatalogHooksAreSortedAndAdoptionPreservesBytes checks action precedence and hook ownership.
func TestCatalogHooksAreSortedAndAdoptionPreservesBytes(t *testing.T) {
	configuration := newProject(t)
	content := headerV2 + "\ncurrent\n"
	old := newBundle(t, 1, catalogFixture{"beta.csv", content, nil})
	assertAction(t, configuration, old, "created")
	target := filepath.Join(configuration.MetaDir, "beta.csv")
	original := []byte(strings.ReplaceAll(content, "\n", "\r\n"))
	writeFixture(t, target, original)
	newer := newBundle(t, 2, catalogFixture{"beta.csv", content, nil}, catalogFixture{"alpha.csv", headerV2 + "\nalpha\n", nil})
	calls := []string{}
	hook := hooks{
		beforeReplacement: func(name string) error { calls = append(calls, "replace:"+name); return nil },
		beforeStateWrite:  func(name string) error { calls = append(calls, "state:"+name); return nil },
	}
	report, err := prepare(context.Background(), configuration, newer, hook)
	if err != nil {
		t.Fatal(err)
	}
	expected := []CatalogReport{{"alpha.csv", "created"}, {"beta.csv", "adopted"}}
	if !reflect.DeepEqual(report.Catalogs, expected) {
		t.Fatal(report)
	}
	if !reflect.DeepEqual(calls, []string{"replace:alpha.csv", "state:alpha.csv", "state:beta.csv"}) {
		t.Fatal(calls)
	}
	actual, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(actual, original) {
		t.Fatal("adoption rewrote equivalent bytes")
	}
}

// TestLateBundleFailureReturnsZeroBeforeHashMismatch checks error priority after a valid catalog.
func TestLateBundleFailureReturnsZeroBeforeHashMismatch(t *testing.T) {
	directory := newBundle(t, 2, catalogFixture{"alpha.csv", headerV2 + "\nalpha\n", nil}, catalogFixture{"zeta.csv", headerV2 + "\nzeta\n", nil})
	writeFixture(t, filepath.Join(directory, "zeta.csv"), []byte("wrong header and wrong hash\n"))
	result, err := validateBundle(directory)
	if err == nil || !strings.Contains(err.Error(), "exact canonical v2 header") {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(result, bundle{}) {
		t.Fatalf("partial bundle escaped: %+v", result)
	}
}

// TestDiscoveryUsesExistingNonregularManifest preserves discovery-before-validation semantics.
func TestDiscoveryUsesExistingNonregularManifest(t *testing.T) {
	root := t.TempDir()
	system, portable := filepath.Join(root, "system"), filepath.Join(root, "portable")
	if err := os.MkdirAll(filepath.Join(system, manifestFilename), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(portable, 0700); err != nil {
		t.Fatal(err)
	}
	writeFixture(t, filepath.Join(portable, manifestFilename), []byte("{}"))
	found, err := findPackagedDirectory([]string{system, portable})
	if err != nil || found != system {
		t.Fatalf("%q %v", found, err)
	}
	_, err = validateBundle(found)
	if err == nil || !strings.Contains(err.Error(), "must be a regular file") {
		t.Fatal(err)
	}
}

// makeRollbackDirectory replaces one isolated recovery fixture with an unexpected directory.
func makeRollbackDirectory(directory string) (string, error) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return "", err
	}
	for _, entry := range entries {
		if !strings.HasPrefix(entry.Name(), ".litradar-meta-rollback-") {
			continue
		}
		filename := filepath.Join(directory, entry.Name())
		if err := os.Remove(filename); err != nil {
			return "", err
		}
		if err := os.Mkdir(filename, 0700); err != nil {
			return "", err
		}
		return filename, nil
	}
	return "", os.ErrNotExist
}

// assertCommittedCatalog checks SQL state and bytes after a post-commit cleanup failure.
func assertCommittedCatalog(t *testing.T, configuration config.Config, target string, expected []byte) {
	t.Helper()
	actual, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(actual, expected) {
		t.Fatal("committed catalog was compensated")
	}
	database := databaseForTest(t, configuration)
	var version int64
	if err := database.QueryRow("SELECT bundle_version FROM managed_meta_catalogs WHERE filename='alpha.csv'").Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 2 {
		t.Fatalf("committed version=%d", version)
	}
}

// TestPostCommitCleanupFailureRetainsCommittedState checks that cleanup cannot undo committed files.
func TestPostCommitCleanupFailureRetainsCommittedState(t *testing.T) {
	configuration := newProject(t)
	old := newBundle(t, 1, catalogFixture{"alpha.csv", "old\n", nil})
	assertAction(t, configuration, old, "created")
	content := headerV2 + "\nnew\n"
	newer := newBundle(t, 2, catalogFixture{"alpha.csv", content, nil})
	unexpected := ""
	hook := hooks{beforeStateWrite: func(string) error {
		var err error
		unexpected, err = makeRollbackDirectory(configuration.MetaDir)
		return err
	}}
	report, err := prepare(context.Background(), configuration, newer, hook)
	if err == nil {
		t.Fatal("unexpected recovery directory deleted")
	}
	if !reflect.DeepEqual(report, Report{}) {
		t.Fatal(report)
	}
	assertCommittedCatalog(t, configuration, filepath.Join(configuration.MetaDir, "alpha.csv"), []byte(content))
	info, err := os.Stat(unexpected)
	if err != nil || !info.IsDir() {
		t.Fatal("recovery evidence lost", err)
	}
}

// TestNewReplacementRollbackRemovesOnlyPublishedFile checks the missing-original flag transition.
func TestNewReplacementRollbackRemovesOnlyPublishedFile(t *testing.T) {
	target := filepath.Join(t.TempDir(), "new.csv")
	change, err := replaceFile(target, []byte("new\n"))
	if err != nil {
		t.Fatal(err)
	}
	if change.rollbackPath != "" || !change.isApplied {
		t.Fatalf("%+v", change)
	}
	if err := change.rollback(); err != nil {
		t.Fatal(err)
	}
	if change.isApplied {
		t.Fatal("rollback flag retained")
	}
	if _, err := os.Stat(target); !os.IsNotExist(err) {
		t.Fatal("created file retained", err)
	}
}
