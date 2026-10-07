package backup

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestManifestFailureKeepsCompletedHeaderSelectionAndComponents(t *testing.T) {
	cases := []struct {
		raw  string
		want Manifest
	}{
		{`{"format":"prefix","version":"bad"}`, Manifest{Format: "prefix"}},
		{`{"format":"prefix","version":2,"created_at":7,"selection":{"metadata":true,"index_databases":"bad","push_state":false},"components":[]}`, Manifest{Format: "prefix", Version: 2, CreatedAt: 7, Selection: Selection{Metadata: true}}},
		{`{"format":"prefix","version":2,"created_at":7,"selection":{"metadata":true,"index_databases":false,"push_state":false},"components":[{"kind":"metadata","path":"meta/one","size":1,"sha256":"first"},{"kind":"metadata","path":"meta/two","size":"bad","sha256":"second"}]}`, Manifest{Format: "prefix", Version: 2, CreatedAt: 7, Selection: Selection{Metadata: true}, Components: []Component{{Kind: "metadata", Path: "meta/one", Size: 1, Sha256: "first"}}}},
	}
	for _, test := range cases {
		value, err := ParseManifest([]byte(test.raw))
		if !errors.Is(err, ErrManifestJson) || !reflect.DeepEqual(value, test.want) {
			t.Fatal("partial manifest mutation changed", value, err)
		}
	}
}

func executeBackupCharacterizationSql(t *testing.T, database *sql.DB, statement string) {
	t.Helper()
	if _, err := database.Exec(statement); err != nil {
		t.Fatal(err)
	}
}

func assertBackupHeartbeat(t *testing.T, filename string, want bool) {
	t.Helper()
	active, err := HasRecentHeartbeat(context.Background(), filename, 100, 90)
	if err != nil || active != want {
		t.Fatal("heartbeat short circuit changed", active, err)
	}
}

func TestActiveServiceHeartbeatPrecedesMalformedLegacyTable(t *testing.T) {
	configuration := fixtureConfig(t)
	database, err := storage.Open(configuration.AuthDbPath, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	executeBackupCharacterizationSql(t, database, "DROP TABLE scheduler_workers; CREATE TABLE scheduler_workers(unrelated TEXT); DROP TABLE service_heartbeats; CREATE TABLE service_heartbeats(service TEXT,instance_id TEXT,started_at REAL,heartbeat_at REAL); INSERT INTO service_heartbeats(service,instance_id,started_at,heartbeat_at) VALUES('api','active',100,100)")
	assertBackupHeartbeat(t, configuration.AuthDbPath, true)
	executeBackupCharacterizationSql(t, database, "DELETE FROM service_heartbeats; INSERT INTO service_heartbeats(service,instance_id,started_at,heartbeat_at) VALUES('other','ignored',100,100)")
	if _, err := HasRecentHeartbeat(context.Background(), configuration.AuthDbPath, 100, 90); err == nil {
		t.Fatal("unrelated service masked malformed legacy table")
	}
	executeBackupCharacterizationSql(t, database, "DROP TABLE scheduler_workers; CREATE TABLE scheduler_workers(heartbeat_at REAL); INSERT INTO scheduler_workers VALUES(100)")
	assertBackupHeartbeat(t, configuration.AuthDbPath, true)
}

func TestCopyGroupMissingSourceCreatesEmptyDestinationAndRunsCallback(t *testing.T) {
	root := t.TempDir()
	called := false
	components, err := copyGroup(filepath.Join(root, "missing"), root, "destination", "metadata", "metadata", func(string) error { called = true; return nil })
	if err != nil || !called || components == nil || len(components) != 0 {
		t.Fatal("missing group defaults changed", components, err)
	}
	info, err := os.Stat(filepath.Join(root, "destination"))
	if err != nil || !info.IsDir() {
		t.Fatal("missing source did not publish empty staging directory", err)
	}
}

func TestCopyGroupCallbackFailurePrecedesSourceMutationDiagnosis(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(root, "source")
	write(t, filepath.Join(source, "first.csv"), []byte("first"))
	sentinel := errors.New("callback failure")
	components, err := copyGroup(source, root, "destination", "metadata", "metadata", func(string) error {
		write(t, filepath.Join(source, "late.csv"), []byte("late"))
		return sentinel
	})
	if !errors.Is(err, sentinel) || components != nil {
		t.Fatal("callback error priority changed", components, err)
	}
}

func TestFailedReplacementRestoresOriginalAndLeavesFlagsClear(t *testing.T) {
	for _, hasOriginal := range []bool{false, true} {
		root := t.TempDir()
		target := filepath.Join(root, "target")
		if hasOriginal {
			write(t, target, []byte("original"))
		}
		item := replacement{target: target, staged: filepath.Join(root, "missing"), rollback: filepath.Join(root, "rollback")}
		if err := item.apply(); err == nil {
			t.Fatal("missing stage accepted")
		}
		if item.hadOriginal || item.isApplied {
			t.Fatal("failed replacement retained applied flags", item)
		}
		if err := item.compensate(); err != nil {
			t.Fatal(err)
		}
		assertFailedReplacementTarget(t, target, hasOriginal)
	}
}

func assertFailedReplacementTarget(t *testing.T, target string, hasOriginal bool) {
	t.Helper()
	raw, err := os.ReadFile(target)
	if hasOriginal {
		if err != nil || string(raw) != "original" {
			t.Fatal("original not restored", err)
		}
	} else if !os.IsNotExist(err) {
		t.Fatal("failed replacement created target", err)
	}
}

func TestRestoreVerificationPrecedesActiveTargetAdmission(t *testing.T) {
	configuration := fixtureConfig(t)
	if err := RecordHeartbeat(context.Background(), configuration.AuthDbPath, Api, "active", 100); err != nil {
		t.Fatal(err)
	}
	directory := t.TempDir()
	write(t, filepath.Join(directory, "manifest.json"), []byte("{"))
	report, err := restore(context.Background(), RestoreOptions{configuration, configuration.AuthDbPath, directory}, 100, nil, nil)
	if !errors.Is(err, ErrManifestJson) || report != (RestoreReport{}) {
		t.Fatal("activity masked invalid backup", report, err)
	}
}
