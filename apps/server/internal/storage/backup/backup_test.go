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

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	authmigration "github.com/QianFuv/LitRadar/apps/server/internal/storage/migrations/auth"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	contentfixture "github.com/QianFuv/LitRadar/apps/server/internal/testkit/content"
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
	if _, err := authmigration.Migrate(context.Background(), configuration.AuthDBPath); err != nil {
		t.Fatal(err)
	}
	database, err := storage.Open(configuration.AuthDBPath, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'synthetic','hash','salt',1,1)"); err != nil {
		if err := database.Close(); err != nil {
			t.Fatalf("database.Close: %v", err)
		}
		t.Fatal(err)
	}
	if _, err := database.Exec("PRAGMA journal_mode=DELETE"); err != nil {
		if err := database.Close(); err != nil {
			t.Fatalf("database.Close: %v", err)
		}
		t.Fatal(err)
	}
	if err := database.Close(); err != nil {
		t.Fatalf("database.Close: %v", err)
	}
	if err := contentfixture.Create(filepath.Join(configuration.IndexDir, "metadata.sqlite"), 9); err != nil {
		t.Fatal(err)
	}

	write(t, filepath.Join(configuration.MetaDir, "nested", "custom.csv"), []byte("custom metadata\n"))
	write(t, filepath.Join(configuration.ProjectRoot, "data", "push_state", "run.json"), []byte("{}"))
	write(t, filepath.Join(configuration.ProjectRoot, "data", "index-control", "must-not-copy.sqlite"), []byte("control"))
	write(t, filepath.Join(configuration.ProjectRoot, "data", "deployment.key"), []byte("not-a-real-key"))
	return configuration
}
func optionsFor(t *testing.T, configuration config.Config) CreateOptions {
	t.Helper()
	return CreateOptions{Config: configuration, AuthDBPath: configuration.AuthDBPath, OutputDir: filepath.Join(t.TempDir(), "backup"), IncludeIndexDatabases: true, IncludePushState: true}
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
	live, err := storage.Open(configuration.AuthDBPath, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := live.Close(); err != nil {
			t.Errorf("live.Close: %v", err)
		}
	}()
	if _, err := live.ExecContext(ctx, "PRAGMA wal_autocheckpoint=0; UPDATE users SET username='committed-wal' WHERE id=1"); err != nil {
		t.Fatal(err)
	}
	manifest, err := Create(ctx, options)
	if err != nil {
		t.Fatal(err)
	}
	assertSelectedBackupManifest(t, manifest)
	assertCommittedBackupUsername(t, ctx, options.OutputDir)
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

// TestBackupAcceptsCurrentAndHistoricalContentTogether protects versioned index snapshots.
func TestBackupAcceptsCurrentAndHistoricalContentTogether(t *testing.T) {
	configuration := fixtureConfig(t)
	if err := contentfixture.Create(filepath.Join(configuration.IndexDir, "current.sqlite"), 10); err != nil {
		t.Fatal(err)
	}
	options := optionsFor(t, configuration)
	manifest, err := Create(context.Background(), options)
	if err != nil {
		t.Fatal(err)
	}
	hasCurrentIndex := false
	for _, component := range manifest.Components {
		if component.Kind == "index_database" && component.SchemaVersion != nil && *component.SchemaVersion == 10 {
			hasCurrentIndex = true
		}
	}
	if !hasCurrentIndex {
		t.Fatal("current content database was omitted from the backup")
	}
	if _, err := Verify(context.Background(), options.OutputDir); err != nil {
		t.Fatal(err)
	}
}

// TestBackupAuthSchemaVersionBoundary accepts the supported auth snapshot and rejects newer schemas.
func TestBackupAuthSchemaVersionBoundary(t *testing.T) {
	ctx := context.Background()
	configuration := fixtureConfig(t)
	options := optionsFor(t, configuration)
	manifest, err := Create(ctx, options)
	if err != nil {
		t.Fatal(err)
	}
	hasCurrentAuth := false
	for _, component := range manifest.Components {
		if component.Kind == "auth_database" && component.SchemaVersion != nil && *component.SchemaVersion == int64(authmigration.SchemaVersion) {
			hasCurrentAuth = true
		}
	}
	if !hasCurrentAuth {
		t.Fatal("current auth schema was omitted from backup")
	}
	if _, err := Verify(ctx, options.OutputDir); err != nil {
		t.Fatal(err)
	}
	for index := range manifest.Components {
		if manifest.Components[index].Kind == "auth_database" {
			future := int64(authmigration.SchemaVersion + 1)
			manifest.Components[index].SchemaVersion = &future
		}
	}
	if err := writeManifest(options.OutputDir, manifest); err != nil {
		t.Fatal(err)
	}
	_, err = Verify(ctx, options.OutputDir)
	requireFailure(t, err, "unsupported")
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
			future := int64(11)
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
			report, err := restore(ctx, RestoreOptions{target, target.AuthDBPath, backup}, 1000, nil, nil)
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
	write(t, target.AuthDBPath+"-journal", make([]byte, 512))
	before, err := snapshot(filepath.Join(target.ProjectRoot, "data"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = restore(ctx, RestoreOptions{target, target.AuthDBPath, options.OutputDir}, 1000, nil, func() error {
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
	if err := RecordHeartbeat(ctx, configuration.AuthDBPath, API, "instance", 10); err != nil {
		t.Fatal(err)
	}
	if err := RecordHeartbeat(ctx, configuration.AuthDBPath, API, "instance", 20); err != nil {
		t.Fatal(err)
	}
	assertHeartbeatRetainsStartTime(t, ctx, configuration.AuthDBPath)
	assertRecentHeartbeatBoundaries(t, ctx, configuration.AuthDBPath)
	if err := DeleteHeartbeat(ctx, configuration.AuthDBPath, API, "instance"); err != nil {
		t.Fatal(err)
	}
	options := optionsFor(t, configuration)
	if _, err := Create(ctx, options); err != nil {
		t.Fatal(err)
	}
	_, err := restore(ctx, RestoreOptions{configuration, configuration.AuthDBPath, options.OutputDir}, 1000, func() error { return RecordHeartbeat(ctx, configuration.AuthDBPath, Worker, "late", 1001) }, nil)
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
	if _, err := ParseManifest([]byte(`["litradar-backup",2,1,[true,false,false],[]]`)); err == nil {
		t.Fatal(err)
	}
	if _, err := ParseManifest([]byte(`{"format":"litradar-backup","version":1,"created_at":1,"selection":{"index_databases":false,"push_state":false},"components":[]}`)); err != nil {
		t.Fatal(err)
	}
}

func assertCommittedBackupUsername(t *testing.T, ctx context.Context, directory string) {
	t.Helper()
	database, err := storage.Open(filepath.Join(directory, "auth.sqlite"), true, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			t.Errorf("database.Close: %v", err)
		}
	}()
	var name string
	if err := database.QueryRowContext(ctx, "SELECT username FROM users WHERE id=1").Scan(&name); err != nil || name != "committed-wal" {
		t.Fatalf("%q %v", name, err)
	}
}

func assertSelectedBackupManifest(t *testing.T, manifest Manifest) {
	t.Helper()
	if manifest.Version != 2 || manifest.Selection != (Selection{true, true, true}) || len(manifest.Components) != 4 {
		t.Fatalf("%+v", manifest)
	}
}

func assertHeartbeatRetainsStartTime(t *testing.T, ctx context.Context, filename string) {
	t.Helper()
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		t.Fatal(err)
	}
	var started float64
	if err := database.QueryRowContext(ctx, "SELECT started_at FROM service_heartbeats WHERE instance_id='instance'").Scan(&started); err != nil || started != 10 {
		t.Fatalf("%f %v", started, err)
	}
	if err := database.Close(); err != nil {
		t.Fatalf("database.Close: %v", err)
	}
}

func assertRecentHeartbeatBoundaries(t *testing.T, ctx context.Context, filename string) {
	t.Helper()
	for _, test := range []struct {
		now  float64
		want bool
	}{{110, true}, {110.001, false}, {-100, true}} {
		active, err := HasRecentHeartbeat(ctx, filename, test.now, 90)
		if err != nil || active != test.want {
			t.Fatalf("%+v => %v %v", test, active, err)
		}
	}
}
