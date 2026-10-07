package meta

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"sync"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

type catalogFixture struct {
	name, content string
	legacy        []string
}

func writeFixture(t *testing.T, filename string, data []byte) {
	t.Helper()
	if err := os.WriteFile(filename, data, 0600); err != nil {
		t.Fatal(err)
	}
}

func newProject(t *testing.T) config.Config {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	if _, err := authmigration.Migrate(context.Background(), configuration.AuthDbPath); err != nil {
		t.Fatal(err)
	}
	return configuration
}

func newBundle(t *testing.T, version int64, catalogs ...catalogFixture) string {
	t.Helper()
	directory := t.TempDir()
	entries := []map[string]any{}
	for _, catalog := range catalogs {
		digest, err := CanonicalSha256([]byte(catalog.content))
		if err != nil {
			t.Fatal(err)
		}
		legacy := []string{}
		for _, content := range catalog.legacy {
			digest, err := CanonicalSha256([]byte(content))
			if err != nil {
				t.Fatal(err)
			}
			legacy = append(legacy, digest)
		}
		entries = append(entries, map[string]any{"filename": catalog.name, "sha256": digest, "legacy_sha256": legacy})
		writeFixture(t, filepath.Join(directory, catalog.name), []byte(catalog.content))
	}
	data, err := json.Marshal(map[string]any{"format": "litradar-meta-bundle", "version": version, "catalogs": entries})
	if err != nil {
		t.Fatal(err)
	}
	writeFixture(t, filepath.Join(directory, manifestFilename), data)
	return directory
}

func databaseForTest(t *testing.T, configuration config.Config) *sql.DB {
	t.Helper()
	database, err := storage.Open(configuration.AuthDbPath, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	return database
}

func assertAction(t *testing.T, configuration config.Config, directory, action string) {
	t.Helper()
	report, err := Prepare(context.Background(), configuration, directory)
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Catalogs) != 1 || report.Catalogs[0].Action != action {
		t.Fatalf("report = %+v; expected %s", report, action)
	}
}

func TestCanonicalHashPreservesMeaningfulBytes(t *testing.T) {
	base, _ := CanonicalSha256([]byte("alpha\nbeta\n"))
	for _, input := range []string{"alpha\nbeta", "alpha\r\nbeta\r\n", "alpha\rbeta\r"} {
		actual, err := CanonicalSha256([]byte(input))
		if err != nil || actual != base {
			t.Fatalf("line ending equivalence: %q: %s %v", input, actual, err)
		}
	}
	for _, input := range []string{"alpha\nbeta\n\n", "\ufeffalpha\nbeta\n"} {
		actual, _ := CanonicalSha256([]byte(input))
		if actual == base {
			t.Fatalf("lost significant bytes: %q", input)
		}
	}
	if _, err := CanonicalSha256([]byte{0xff}); err == nil {
		t.Fatal("invalid UTF-8 accepted")
	}
}

// TestPreparationPreservesCustomizationAndAdoptsEquivalentBytes checks managed ownership without rewriting customization.
func TestPreparationPreservesCustomizationAndAdoptsEquivalentBytes(t *testing.T) {
	configuration := newProject(t)
	current, legacy := "name,value\nalpha,current\n", "name,value\nalpha,legacy\n"
	directory := newBundle(t, 1, catalogFixture{"alpha.csv", current, []string{legacy}})
	assertAction(t, configuration, directory, "created")
	assertAction(t, configuration, directory, "unchanged")
	database := databaseForTest(t, configuration)
	if _, err := database.Exec("DELETE FROM managed_meta_catalogs"); err != nil {
		t.Fatal(err)
	}
	target := filepath.Join(configuration.MetaDir, "alpha.csv")
	crlf := []byte(strings.ReplaceAll(current, "\n", "\r\n"))
	writeFixture(t, target, crlf)
	assertAction(t, configuration, directory, "adopted")
	actual, _ := os.ReadFile(target)
	if !bytes.Equal(actual, crlf) {
		t.Fatal("adoption rewrote equivalent operator bytes")
	}
	if _, err := database.Exec("DELETE FROM managed_meta_catalogs"); err != nil {
		t.Fatal(err)
	}
	writeFixture(t, target, []byte(legacy))
	assertAction(t, configuration, directory, "updated")
	updated := headerV2 + "\nalpha,updated\n"
	newer := newBundle(t, 2, catalogFixture{"alpha.csv", updated, nil})
	assertAction(t, configuration, newer, "updated")
	assertCustomizedCatalogBytes(t, configuration, newer, target)
	var version int64
	if err := database.QueryRow("SELECT bundle_version FROM managed_meta_catalogs WHERE filename='alpha.csv'").Scan(&version); err != nil || version != 2 {
		t.Fatalf("state = %d, %v", version, err)
	}
	if err := os.Remove(target); err != nil {
		t.Fatal(err)
	}
	assertDowngradeDoesNotRecreate(t, configuration, directory, target)
}

func TestWholeBundleValidatedBeforePersistentWrites(t *testing.T) {
	for _, mutation := range []string{"late_hash", "unknown_field", "duplicate_field", "missing_field", "null_field", "bad_header"} {
		t.Run(mutation, func(t *testing.T) {
			configuration := config.FromProjectRoot(filepath.Join(t.TempDir(), "absent-project"))
			directory := newBundle(t, 1, catalogFixture{"alpha.csv", "a\n", nil}, catalogFixture{"zeta.csv", "z\n", nil})
			manifest := filepath.Join(directory, manifestFilename)
			data, err := os.ReadFile(manifest)
			if err != nil {
				t.Fatal(err)
			}
			switch mutation {
			case "late_hash":
				writeFixture(t, filepath.Join(directory, "zeta.csv"), []byte("tampered\n"))
			case "unknown_field":
				data = bytes.Replace(data, []byte(`"version":1`), []byte(`"extra":0,"version":1`), 1)
			case "duplicate_field":
				data = bytes.Replace(data, []byte(`"version":1`), []byte(`"version":1,"version":1`), 1)
			case "missing_field":
				data = bytes.Replace(data, []byte(`,"version":1`), nil, 1)
			case "null_field":
				data = bytes.Replace(data, []byte(`"version":1`), []byte(`"version":null`), 1)
			case "bad_header":
				data = bytes.Replace(data, []byte(`"version":1`), []byte(`"version":2`), 1)
			}
			writeFixture(t, manifest, data)
			if _, err := Prepare(context.Background(), configuration, directory); err == nil {
				t.Fatal("invalid bundle accepted")
			}
			if _, err := os.Stat(configuration.ProjectRoot); !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("persistent path created: %v", err)
			}
		})
	}
}

func TestReplacementAndStateFailuresRestoreWholePreparation(t *testing.T) {
	for _, failurePoint := range []string{"replace", "state", "commit"} {
		for _, hasExisting := range []bool{false, true} {
			name := failurePoint + "/created"
			if hasExisting {
				name = failurePoint + "/updated"
			}
			t.Run(name, func(t *testing.T) {
				configuration := newProject(t)
				database := databaseForTest(t, configuration)
				old := newBundle(t, 1, catalogFixture{"alpha.csv", "old-a\n", nil}, catalogFixture{"zeta.csv", "old-z\n", nil})
				if hasExisting {
					if _, err := Prepare(context.Background(), configuration, old); err != nil {
						t.Fatal(err)
					}
				}
				newer := newBundle(t, 2, catalogFixture{"alpha.csv", headerV2 + "\nnew-a\n", nil}, catalogFixture{"zeta.csv", headerV2 + "\nnew-z\n", nil})
				injected := errors.New("injected failure")
				hook := hooks{}
				if failurePoint == "replace" {
					hook.beforeReplacement = func(name string) error {
						if name == "zeta.csv" {
							return injected
						}
						return nil
					}
				}
				if failurePoint == "state" {
					hook.beforeStateWrite = func(name string) error {
						if name == "zeta.csv" {
							return injected
						}
						return nil
					}
				}
				if failurePoint == "commit" {
					_, err := database.Exec(`CREATE TABLE test_parent(id INTEGER PRIMARY KEY); CREATE TABLE test_child(id INTEGER REFERENCES test_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER test_meta_commit_failure AFTER INSERT ON managed_meta_catalogs BEGIN INSERT INTO test_child VALUES(1); END; CREATE TRIGGER test_meta_update_failure AFTER UPDATE ON managed_meta_catalogs BEGIN INSERT INTO test_child VALUES(1); END;`)
					if err != nil {
						t.Fatal(err)
					}
				}
				if _, err := prepare(context.Background(), configuration, newer, hook); err == nil {
					t.Fatal("injected failure succeeded")
				} else if failurePoint != "commit" && !errors.Is(err, injected) {
					t.Fatal(err)
				}
				for _, name := range []string{"alpha.csv", "zeta.csv"} {
					actual, err := os.ReadFile(filepath.Join(configuration.MetaDir, name))
					if hasExisting {
						expected, readErr := os.ReadFile(filepath.Join(old, name))
						if readErr != nil || err != nil || !bytes.Equal(actual, expected) {
							t.Fatalf("%s not restored: %q %v", name, actual, err)
						}
					} else if !errors.Is(err, os.ErrNotExist) {
						t.Fatalf("new file retained: %s %v", name, err)
					}
				}
				var count, version int
				if err := database.QueryRow("SELECT COUNT(*),COALESCE(MAX(bundle_version),0) FROM managed_meta_catalogs").Scan(&count, &version); err != nil {
					t.Fatal(err)
				}
				if hasExisting && (count != 2 || version != 1) || !hasExisting && count != 0 {
					t.Fatalf("partial state retained: count=%d version=%d", count, version)
				}
				entries, err := os.ReadDir(configuration.MetaDir)
				if err != nil {
					t.Fatal(err)
				}
				for _, entry := range entries {
					if strings.HasPrefix(entry.Name(), ".litradar-meta-") {
						t.Fatalf("temporary artifact retained: %s", entry.Name())
					}
				}
			})
		}
	}
}

func TestConcurrentPreparationConvergesWithoutLosingState(t *testing.T) {
	configuration := newProject(t)
	directory := newBundle(t, 1, catalogFixture{"alpha.csv", "alpha\n", nil})
	start := make(chan struct{})
	var workers sync.WaitGroup
	actions := make(chan string, 2)
	for range 2 {
		workers.Go(func() {
			<-start
			report, err := Prepare(context.Background(), configuration, directory)
			if err != nil {
				t.Error(err)
				return
			}
			actions <- report.Catalogs[0].Action
		})
	}
	close(start)
	workers.Wait()
	close(actions)
	actual := []string{}
	for action := range actions {
		actual = append(actual, action)
	}
	slices.Sort(actual)
	if !reflect.DeepEqual(actual, []string{"created", "unchanged"}) {
		t.Fatalf("actions = %v", actual)
	}
}

func TestManifestAcceptsOriginalSerdeSequences(t *testing.T) {
	for _, sequence := range []string{"manifest", "catalog", "both"} {
		t.Run(sequence, func(t *testing.T) {
			directory := newBundle(t, 1, catalogFixture{"alpha.csv", "alpha\n", nil})
			digest, _ := CanonicalSha256([]byte("alpha\n"))
			var entry any = map[string]any{"filename": "alpha.csv", "sha256": digest, "legacy_sha256": []string{}}
			if sequence != "manifest" {
				entry = []any{"alpha.csv", digest, []string{}}
			}
			var manifest any = map[string]any{"format": "litradar-meta-bundle", "version": 1, "catalogs": []any{entry}}
			if sequence != "catalog" {
				manifest = []any{"litradar-meta-bundle", 1, []any{entry}}
			}
			data, err := json.Marshal(manifest)
			if err != nil {
				t.Fatal(err)
			}
			writeFixture(t, filepath.Join(directory, manifestFilename), data)
			assertAction(t, newProject(t), directory, "created")
		})
	}
}

func TestCorruptStoredVersionFailsBeforeRecreatingCatalog(t *testing.T) {
	for _, value := range []string{"CAST('1' AS BLOB)", "1.5"} {
		t.Run(value, func(t *testing.T) {
			configuration := newProject(t)
			directory := newBundle(t, 1, catalogFixture{"alpha.csv", "alpha\n", nil})
			assertAction(t, configuration, directory, "created")
			database := databaseForTest(t, configuration)
			if _, err := database.Exec("UPDATE managed_meta_catalogs SET bundle_version=" + value); err != nil {
				t.Fatal(err)
			}
			target := filepath.Join(configuration.MetaDir, "alpha.csv")
			if err := os.Remove(target); err != nil {
				t.Fatal(err)
			}
			if _, err := Prepare(context.Background(), configuration, directory); err == nil {
				t.Fatal("corrupt stored version accepted")
			}
			if _, err := os.Stat(target); !errors.Is(err, os.ErrNotExist) {
				t.Fatal("catalog recreated before rejecting corrupt version")
			}
		})
	}
}

func TestRollbackAndCleanupPreserveUnexpectedDirectories(t *testing.T) {
	for _, operation := range []string{"rollback", "finish"} {
		t.Run(operation, func(t *testing.T) {
			target := filepath.Join(t.TempDir(), "alpha.csv")
			writeFixture(t, target, []byte("original\n"))
			change, err := replaceFile(target, []byte("new\n"))
			if err != nil {
				t.Fatal(err)
			}
			unexpected := target
			if operation == "finish" {
				unexpected = change.rollbackPath
			}
			if err := os.Remove(unexpected); err != nil {
				t.Fatal(err)
			}
			if err := os.Mkdir(unexpected, 0700); err != nil {
				t.Fatal(err)
			}
			if operation == "rollback" {
				err = change.rollback()
			} else {
				err = change.finish()
			}
			if err == nil {
				t.Fatal("unexpected directory removed")
			}
			info, err := os.Stat(unexpected)
			if err != nil || !info.IsDir() {
				t.Fatal("recovery evidence lost")
			}
			if operation == "rollback" {
				actual, err := os.ReadFile(change.rollbackPath)
				if err != nil || string(actual) != "original\n" {
					t.Fatal("original rollback evidence lost")
				}
			}
		})
	}
}

// assertCustomizedCatalogBytes keeps valid and invalid UTF-8 operator bytes intact.
func assertCustomizedCatalogBytes(t *testing.T, configuration config.Config, newer, target string) {
	t.Helper()
	for _, custom := range [][]byte{[]byte("operator,customization\n"), {0xff, 0xfe}} {
		writeFixture(t, target, custom)
		assertAction(t, configuration, newer, "customized")
		actual, _ := os.ReadFile(target)
		if !bytes.Equal(actual, custom) {
			t.Fatal("custom bytes replaced")
		}
	}
}

// assertDowngradeDoesNotRecreate checks typed refusal before recreating a missing catalog.
func assertDowngradeDoesNotRecreate(t *testing.T, configuration config.Config, directory, target string) {
	t.Helper()
	if _, err := Prepare(context.Background(), configuration, directory); err == nil {
		t.Fatal("downgrade accepted")
	} else {
		var downgrade Downgrade
		if !errors.As(err, &downgrade) {
			t.Fatal(err)
		}
	}
	if _, err := os.Stat(target); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("downgrade recreated missing catalog")
	}
}
