package backup

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func write(t *testing.T, filename string, data []byte) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, data, 0600); err != nil {
		t.Fatal(err)
	}
}
func fixtureConfig(t *testing.T) config.Config {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	root := filepath.Join("..", "..", "..", "tests", "migration", "storage", "fixtures")
	for _, file := range []struct{ source, destination string }{{"favorites-auth.sqlite.fixture", configuration.AuthDbPath}, {"metadata.sqlite.fixture", filepath.Join(configuration.IndexDir, "metadata.sqlite")}} {
		raw, err := os.ReadFile(filepath.Join(root, file.source))
		if err != nil {
			t.Fatal(err)
		}
		write(t, file.destination, raw)
	}
	write(t, filepath.Join(configuration.MetaDir, "nested", "custom.csv"), []byte("custom metadata\n"))
	write(t, filepath.Join(configuration.ProjectRoot, "data", "push_state", "run.json"), []byte("{}"))
	write(t, filepath.Join(configuration.ProjectRoot, "data", "index-control", "must-not-copy.sqlite"), []byte("control"))
	write(t, filepath.Join(configuration.ProjectRoot, "data", "deployment.key"), []byte("not-a-real-key"))
	return configuration
}
func optionsFor(t *testing.T, configuration config.Config) CreateOptions {
	t.Helper()
	return CreateOptions{Config: configuration, AuthDbPath: configuration.AuthDbPath, OutputDir: filepath.Join(t.TempDir(), "backup"), IncludeIndexDatabases: true, IncludePushState: true}
}
func requireFailure(t *testing.T, err error, kind string) {
	t.Helper()
	var actual Failure
	if !errors.As(err, &actual) || actual.Kind != kind {
		t.Fatalf("want %s failure, got %v", kind, err)
	}
}

func TestCreateCapturesCommittedWalAndExactSelectedGroups(t *testing.T) {
	ctx := context.Background()
	configuration := fixtureConfig(t)
	options := optionsFor(t, configuration)
	live, err := storage.Open(configuration.AuthDbPath, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	if _, err := live.ExecContext(ctx, "PRAGMA wal_autocheckpoint=0; UPDATE users SET username='committed-wal' WHERE id=1"); err != nil {
		t.Fatal(err)
	}
	manifest, err := Create(ctx, options)
	if err != nil {
		t.Fatal(err)
	}
	if manifest.Version != 2 || manifest.Selection != (Selection{true, true, true}) || len(manifest.Components) != 4 {
		t.Fatalf("%+v", manifest)
	}
	database, err := storage.Open(filepath.Join(options.OutputDir, "auth.sqlite"), true, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	var name string
	if err := database.QueryRowContext(ctx, "SELECT username FROM users WHERE id=1").Scan(&name); err != nil || name != "committed-wal" {
		t.Fatalf("%q %v", name, err)
	}
	for _, file := range manifest.Components {
		if strings.Contains(file.Path, "control") || keyFile(file.Path) {
			t.Fatal(file.Path)
		}
	}
	verified, err := Verify(ctx, options.OutputDir)
	if err != nil || !reflect.DeepEqual(manifest, verified) {
		t.Fatalf("%+v %v", verified, err)
	}
}

func TestCreateRejectsChangingMetadataAndForbiddenKeysBeforePublication(t *testing.T) {
	ctx := context.Background()
	configuration := fixtureConfig(t)
	options := optionsFor(t, configuration)
	_, err := create(ctx, options, func(source string) error {
		return os.WriteFile(filepath.Join(source, "late.csv"), []byte("late"), 0600)
	})
	requireFailure(t, err, "integrity")
	if fileExists(options.OutputDir) {
		t.Fatal("partial snapshot published")
	}
	write(t, filepath.Join(configuration.MetaDir, "operator.PEM"), []byte("secret"))
	_, err = Create(ctx, options)
	requireFailure(t, err, "input")
	if fileExists(options.OutputDir) {
		t.Fatal("key snapshot published")
	}
}

func TestVerifyRejectsInventoryTamperingAndRetainsHistoricalSchemaSupport(t *testing.T) {
	ctx := context.Background()
	configuration := fixtureConfig(t)
	options := optionsFor(t, configuration)
	manifest, err := Create(ctx, options)
	if err != nil {
		t.Fatal(err)
	}
	write(t, filepath.Join(options.OutputDir, "unexpected.txt"), []byte("extra"))
	_, err = Verify(ctx, options.OutputDir)
	requireFailure(t, err, "integrity")
	if err := os.Remove(filepath.Join(options.OutputDir, "unexpected.txt")); err != nil {
		t.Fatal(err)
	}
	write(t, filepath.Join(options.OutputDir, "meta", "nested", "custom.csv"), []byte("changed"))
	_, err = Verify(ctx, options.OutputDir)
	requireFailure(t, err, "integrity")
	write(t, filepath.Join(options.OutputDir, "meta", "nested", "custom.csv"), []byte("custom metadata\n"))
	for index := range manifest.Components {
		if manifest.Components[index].Kind == "index_database" {
			future := int64(10)
			manifest.Components[index].SchemaVersion = &future
		}
	}
	if err := writeManifest(options.OutputDir, manifest); err != nil {
		t.Fatal(err)
	}
	_, err = Verify(ctx, options.OutputDir)
	requireFailure(t, err, "unsupported")
}

func TestRestoreReplacesSelectedGroupsAndPreservesV1Metadata(t *testing.T) {
	ctx := context.Background()
	source := fixtureConfig(t)
	options := optionsFor(t, source)
	manifest, err := Create(ctx, options)
	if err != nil {
		t.Fatal(err)
	}
	for _, version := range []uint32{2, 1} {
		t.Run(string(rune('0'+version)), func(t *testing.T) {
			target := fixtureConfig(t)
			write(t, filepath.Join(target.MetaDir, "target-only.txt"), []byte("original"))
			write(t, filepath.Join(target.IndexDir, "obsolete.sqlite"), []byte("obsolete"))
			write(t, filepath.Join(target.ProjectRoot, "data", "folder_push_state", "old.json"), []byte("old"))
			backup := options.OutputDir
			if version == 1 {
				backup = filepath.Join(t.TempDir(), "legacy")
				if err := os.MkdirAll(backup, 0700); err != nil {
					t.Fatal(err)
				}
				legacy := manifest
				legacy.Version = 1
				legacy.Selection.Metadata = false
				legacy.Components = []Component{}
				for _, item := range manifest.Components {
					if item.Kind != "metadata" {
						legacy.Components = append(legacy.Components, item)
						if err := copyFile(filepath.Join(options.OutputDir, filepath.FromSlash(item.Path)), filepath.Join(backup, filepath.FromSlash(item.Path))); err != nil {
							t.Fatal(err)
						}
					}
				}
				if err := writeManifest(backup, legacy); err != nil {
					t.Fatal(err)
				}
			}
			report, err := restore(ctx, RestoreOptions{target, target.AuthDbPath, backup}, 1000, nil, nil)
			if err != nil || report.RestoredMetadata != (version == 2) || report.RestoredDatabases != 2 {
				t.Fatalf("%+v %v", report, err)
			}
			if fileExists(filepath.Join(target.MetaDir, "target-only.txt")) != (version == 1) {
				t.Fatal("metadata selection ignored")
			}
			if fileExists(filepath.Join(target.IndexDir, "obsolete.sqlite")) || fileExists(filepath.Join(target.ProjectRoot, "data", "folder_push_state", "old.json")) {
				t.Fatal("selected empty/obsolete group not replaced")
			}
			if !fileExists(filepath.Join(target.ProjectRoot, "data", "deployment.key")) {
				t.Fatal("deployment key altered")
			}
		})
	}
}

func TestRestorePostValidationFailureRollsBackAllReplacements(t *testing.T) {
	ctx := context.Background()
	source := fixtureConfig(t)
	options := optionsFor(t, source)
	if _, err := Create(ctx, options); err != nil {
		t.Fatal(err)
	}
	target := fixtureConfig(t)
	write(t, filepath.Join(target.MetaDir, "target.txt"), []byte("target"))
	write(t, target.AuthDbPath+"-journal", make([]byte, 512))
	before, err := snapshot(filepath.Join(target.ProjectRoot, "data"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = restore(ctx, RestoreOptions{target, target.AuthDbPath, options.OutputDir}, 1000, nil, func() error {
		return os.WriteFile(filepath.Join(target.MetaDir, "nested", "custom.csv"), []byte("corrupt"), 0600)
	})
	requireFailure(t, err, "integrity")
	after, err := snapshot(filepath.Join(target.ProjectRoot, "data"))
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(before, after) {
		t.Fatalf("rollback changed data: before=%+v after=%+v", before, after)
	}
}

func TestHeartbeatBoundaryRenewalAndLateRestoreGate(t *testing.T) {
	ctx := context.Background()
	configuration := fixtureConfig(t)
	if err := RecordHeartbeat(ctx, configuration.AuthDbPath, Api, "instance", 10); err != nil {
		t.Fatal(err)
	}
	if err := RecordHeartbeat(ctx, configuration.AuthDbPath, Api, "instance", 20); err != nil {
		t.Fatal(err)
	}
	database, err := storage.Open(configuration.AuthDbPath, true, 1)
	if err != nil {
		t.Fatal(err)
	}
	var started float64
	if err := database.QueryRowContext(ctx, "SELECT started_at FROM service_heartbeats WHERE instance_id='instance'").Scan(&started); err != nil || started != 10 {
		t.Fatalf("%f %v", started, err)
	}
	database.Close()
	for _, test := range []struct {
		now  float64
		want bool
	}{{110, true}, {110.001, false}, {-100, true}} {
		active, err := HasRecentHeartbeat(ctx, configuration.AuthDbPath, test.now, 90)
		if err != nil || active != test.want {
			t.Fatalf("%+v => %v %v", test, active, err)
		}
	}
	if err := DeleteHeartbeat(ctx, configuration.AuthDbPath, Api, "instance"); err != nil {
		t.Fatal(err)
	}
	options := optionsFor(t, configuration)
	if _, err := Create(ctx, options); err != nil {
		t.Fatal(err)
	}
	_, err = restore(ctx, RestoreOptions{configuration, configuration.AuthDbPath, options.OutputDir}, 1000, func() error { return RecordHeartbeat(ctx, configuration.AuthDbPath, Worker, "late", 1001) }, nil)
	if !errors.Is(err, ErrActiveTarget) {
		t.Fatal(err)
	}
	if _, err := HasRecentHeartbeat(ctx, filepath.Join(t.TempDir(), "missing"), math.NaN(), -1); err != nil {
		t.Fatal("missing database must short circuit time validation")
	}
}

func TestManifestRejectsUnsafePathsAndDuplicateFields(t *testing.T) {
	for _, value := range []string{"", "../auth.sqlite", "/auth.sqlite", "meta//x", "meta/./x", "meta\\x", "meta/C:x"} {
		if _, err := parsePath(value); err == nil {
			t.Fatalf("accepted %q", value)
		}
	}
	manifest := Manifest{"litradar-backup", 2, 1, Selection{true, false, false}, []Component{}}
	raw, err := json.Marshal(manifest)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ParseManifest(raw); err != nil {
		t.Fatal(err)
	}
	duplicate := strings.Replace(string(raw), `"version":2`, `"version":2,"version":2`, 1)
	if _, err := ParseManifest([]byte(duplicate)); err == nil {
		t.Fatal("duplicate accepted")
	}
	if _, err := ParseManifest([]byte(`["litradar-backup",2,1,[true,false,false],[]]`)); err != nil {
		t.Fatal(err)
	}
	if _, err := ParseManifest([]byte(`{"format":"litradar-backup","version":1,"created_at":1,"selection":{"index_databases":false,"push_state":false},"components":[]}`)); err != nil {
		t.Fatal(err)
	}
}
