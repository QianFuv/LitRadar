package auth

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
)

func testConnection(t *testing.T, filename string) *sql.Conn {
	t.Helper()
	database, err := openMigrationDatabase(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	return connection
}

func mustExecute(t *testing.T, connection *sql.Conn, statement string, arguments ...any) {
	t.Helper()
	if err := execute(context.Background(), connection, statement, arguments...); err != nil {
		t.Fatal(err)
	}
}

func scalar[T any](t *testing.T, connection *sql.Conn, query string, arguments ...any) T {
	t.Helper()
	var result T
	if err := connection.QueryRowContext(context.Background(), query, arguments...).Scan(&result); err != nil {
		t.Fatal(err)
	}
	return result
}

func historicalDatabase(t *testing.T, version int) (string, *sql.Conn) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	connection := testConnection(t, filename)
	mustExecute(t, connection, "PRAGMA foreign_keys=ON")
	for current := 0; current < version; current++ {
		if _, err := migrateStep(context.Background(), connection, current, 0); err != nil {
			t.Fatal(err)
		}
	}
	return filename, connection
}

func TestUnversionedLegacyUsersPromoteOnlySmallestId(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "legacy.sqlite")
	connection := testConnection(t, filename)
	mustExecute(t, connection, `CREATE TABLE users(id INTEGER PRIMARY KEY AUTOINCREMENT,username TEXT NOT NULL UNIQUE COLLATE NOCASE,password_hash TEXT NOT NULL,salt TEXT NOT NULL,created_at REAL NOT NULL,updated_at REAL NOT NULL);
INSERT INTO users VALUES(9,'earlier_created','hash','salt',1,1),(4,'smallest_id','hash','salt',2,2);`)
	if summary, err := Migrate(context.Background(), filename); err != nil || summary != (Summary{0, 20}) {
		t.Fatalf("%+v %v", summary, err)
	}
	if id := scalar[int](t, connection, "SELECT id FROM users WHERE is_admin=1"); id != 4 {
		t.Fatal("legacy bootstrap did not use smallest ID")
	}
	if count := scalar[int](t, connection, "SELECT count(*) FROM users WHERE is_admin<>0"); count != 1 {
		t.Fatal("more than one legacy administrator promoted")
	}
}

func TestExistingInviteLifecycleRequiresOriginalColumnOrder(t *testing.T) {
	filename, connection := historicalDatabase(t, 12)
	mustExecute(t, connection, "PRAGMA user_version=11; ALTER TABLE invite_code_uses ADD COLUMN unexpected INTEGER")
	if _, err := Migrate(context.Background(), filename); err == nil {
		t.Fatal("malformed existing lifecycle table accepted")
	}
	if version := scalar[int](t, connection, "PRAGMA user_version"); version != 11 {
		t.Fatal("failed lifecycle validation advanced schema version")
	}
}

func TestMigrationNeverReconfiguresCurrentOrFutureDatabase(t *testing.T) {
	for _, version := range []int{20, 21} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			filename := filepath.Join(t.TempDir(), "auth.sqlite")
			connection := testConnection(t, filename)
			mustExecute(t, connection, fmt.Sprintf("CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES ('unchanged'); PRAGMA user_version=%d", version))
			connection.Close()
			before, err := os.ReadFile(filename)
			if err != nil {
				t.Fatal(err)
			}
			summary, err := Migrate(context.Background(), filename)
			if version == 20 && (err != nil || summary != (Summary{20, 20})) {
				t.Fatalf("%+v %v", summary, err)
			}
			var unsupported UnsupportedVersion
			if version == 21 && (!errors.As(err, &unsupported) || unsupported.Found != 21) {
				t.Fatal(err)
			}
			after, err := os.ReadFile(filename)
			if err != nil {
				t.Fatal(err)
			}
			if string(before) != string(after) {
				t.Fatal("database bytes changed")
			}
			for _, suffix := range []string{"-wal", "-shm", "-journal"} {
				if _, err := os.Stat(filename + suffix); !errors.Is(err, os.ErrNotExist) {
					t.Fatalf("unexpected sidecar %s: %v", suffix, err)
				}
			}
		})
	}
}

func TestConcurrentMigrationRechecksLockedVersion(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	connection := testConnection(t, filename)
	mustExecute(t, connection, "PRAGMA journal_mode=WAL")
	var workers sync.WaitGroup
	failures := make(chan error, 6)
	for worker := 0; worker < 6; worker++ {
		workers.Add(1)
		go func() { defer workers.Done(); _, err := Migrate(context.Background(), filename); failures <- err }()
	}
	workers.Wait()
	close(failures)
	for err := range failures {
		if err != nil {
			t.Fatal(err)
		}
	}
	if version := scalar[int](t, connection, "PRAGMA user_version"); version != 20 {
		t.Fatal(version)
	}
}

func TestProviderMigrationsPreservePrecedenceEmptyOverridesAndTimestamps(t *testing.T) {
	filename, connection := historicalDatabase(t, 6)
	mustExecute(t, connection, `INSERT INTO runtime_settings VALUES ('article_detail_provider_order','scholarly',10),('article_abstract_provider_order','cnki,scholarly',20),('article_fulltext_provider_order','',30)`)
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	for _, item := range []struct {
		key, value string
		updated    float64
	}{
		{"article_abstract_provider_orders", `{"default":["cnki","scholarly"],"catalogs":{}}`, 20},
		{"article_fulltext_provider_orders", `{"default":[],"catalogs":{}}`, 30},
	} {
		if value := scalar[string](t, connection, "SELECT value FROM runtime_settings WHERE key=?", item.key); value != item.value {
			t.Fatal(value)
		}
		if updated := scalar[float64](t, connection, "SELECT updated_at FROM runtime_settings WHERE key=?", item.key); updated != item.updated {
			t.Fatal(updated)
		}
	}
	if count := scalar[int](t, connection, "SELECT count(*) FROM runtime_settings WHERE key LIKE '%_provider_order'"); count != 0 {
		t.Fatal(count)
	}
}

func TestMalformedLegacyProviderOrderRollsBackOnlyItsVersion(t *testing.T) {
	filename, connection := historicalDatabase(t, 5)
	mustExecute(t, connection, `INSERT INTO runtime_settings VALUES ('article_detail_provider_order','duplicate,duplicate',10)`)
	_, err := Migrate(context.Background(), filename)
	if !errors.Is(err, ErrProviderState) {
		t.Fatal(err)
	}
	if version := scalar[int](t, connection, "PRAGMA user_version"); version != 6 {
		t.Fatal(version)
	}
	if value := scalar[string](t, connection, "SELECT value FROM runtime_settings WHERE key='article_detail_provider_order'"); value != "duplicate,duplicate" {
		t.Fatal(value)
	}
	if count := scalar[int](t, connection, "SELECT count(*) FROM sqlite_schema WHERE name='managed_meta_catalogs'"); count != 1 {
		t.Fatal("prior successful version rolled back")
	}
}

func TestProviderRenameFailureRollsBackImplicitDefaults(t *testing.T) {
	filename, connection := historicalDatabase(t, 7)
	mustExecute(t, connection, `INSERT INTO runtime_settings VALUES ('index_provider_routes','{"bad":true}',10)`)
	if _, err := Migrate(context.Background(), filename); !errors.Is(err, ErrProviderState) {
		t.Fatal(err)
	}
	if version := scalar[int](t, connection, "PRAGMA user_version"); version != 7 {
		t.Fatal(version)
	}
	if count := scalar[int](t, connection, "SELECT count(*) FROM runtime_settings"); count != 1 {
		t.Fatal(count)
	}
}

func TestProviderRetirementPreservesDomesticPolicyAndExplicitEmpty(t *testing.T) {
	filename, connection := historicalDatabase(t, 18)
	mustExecute(t, connection, `INSERT INTO runtime_settings VALUES ('provider_proxy_policy','{"cnki":false,"cnki_oversea":true,"scholarly":true}',10),('article_abstract_provider_orders','{"default":["cnki_oversea","cnki","scholarly","cnki"],"catalogs":{"empty":[]}}',20)`)
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	if value := scalar[string](t, connection, "SELECT value FROM runtime_settings WHERE key='provider_proxy_policy'"); value != `{"cnki":false,"scholarly":true}` {
		t.Fatal(value)
	}
	if value := scalar[string](t, connection, "SELECT value FROM runtime_settings WHERE key='article_abstract_provider_orders'"); value != `{"default":["cnki","scholarly"],"catalogs":{"empty":[]}}` {
		t.Fatal(value)
	}
	if value := scalar[int](t, connection, "SELECT updated_at FROM runtime_settings WHERE key='article_abstract_provider_orders'"); value != 20 {
		t.Fatal(value)
	}
}

func TestProviderOrdersRejectMalformedTypedState(t *testing.T) {
	for _, raw := range []string{`{}`, `{"default":[]}`, `{"default":[],"catalogs":{},"extra":true}`, `{"default":null,"catalogs":{}}`, `{"default":[null],"catalogs":{}}`, `{"default":[],"catalogs":null}`, `{"default":[],"catalogs":{"x":null}}`, `{"default":[],"default":[],"catalogs":{}}`} {
		if _, err := parseOrders(raw); !errors.Is(err, ErrProviderState) {
			t.Fatalf("accepted %s: %v", raw, err)
		}
	}
}

func TestEmptySchedulerHistoryRetainsAutoincrementHighWater(t *testing.T) {
	filename, connection := historicalDatabase(t, 19)
	mustExecute(t, connection, `INSERT INTO scheduled_task_runs(id,task_id,task_name,scheduled_for,status) VALUES (1000,1,'removed',1,'pending'); DELETE FROM scheduled_task_runs`)
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	mustExecute(t, connection, `INSERT INTO scheduled_task_runs(task_id,task_name,scheduled_for,status) VALUES (1,'scheduled',1,'pending')`)
	if value := scalar[int64](t, connection, "SELECT id FROM scheduled_task_runs"); value != 1001 {
		t.Fatal(value)
	}
	if err := execute(context.Background(), connection, `INSERT INTO scheduled_task_runs(task_id,task_name,scheduled_for,status) VALUES (1,'duplicate',1,'pending')`); err == nil {
		t.Fatal("duplicate scheduled slot accepted")
	}
}

func TestSchedulerMigrationPreservesNonemptyHistoryAndManualSlots(t *testing.T) {
	filename, connection := historicalDatabase(t, 19)
	mustExecute(t, connection, `INSERT INTO scheduled_task_runs
		(id,task_id,task_name,scheduled_for,status,worker_id,claim_expires_at,claimed_at,started_at,finished_at,output_summary)
		VALUES (7,42,'retained job',600,'unknown','worker-a',700.5,601.25,602.5,603.75,'ambiguous delivery');
		INSERT INTO scheduled_task_runs(id,task_id,task_name,scheduled_for,status) VALUES (2000,42,'deleted job',900,'pending');
		DELETE FROM scheduled_task_runs WHERE id=2000`)
	if summary, err := Migrate(context.Background(), filename); err != nil || summary != (Summary{19, 20}) {
		t.Fatalf("migration failed: %+v %v", summary, err)
	}
	var id, taskId, scheduledFor int64
	var taskName, status, worker, output, trigger string
	var expiry, claimed, started, finished float64
	err := connection.QueryRowContext(context.Background(), `SELECT id,task_id,task_name,scheduled_for,status,worker_id,
		claim_expires_at,claimed_at,started_at,finished_at,output_summary,trigger_kind FROM scheduled_task_runs WHERE id=7`).Scan(
		&id, &taskId, &taskName, &scheduledFor, &status, &worker, &expiry, &claimed, &started, &finished, &output, &trigger)
	if err != nil {
		t.Fatal(err)
	}
	if id != 7 || taskId != 42 || taskName != "retained job" || scheduledFor != 600 || status != "unknown" || worker != "worker-a" ||
		expiry != 700.5 || claimed != 601.25 || started != 602.5 || finished != 603.75 || output != "ambiguous delivery" || trigger != "scheduled" {
		t.Fatal("migration changed persisted scheduler history")
	}
	if err := execute(context.Background(), connection, `INSERT INTO scheduled_task_runs(task_id,task_name,scheduled_for,status) VALUES (42,'duplicate',600,'pending')`); err == nil {
		t.Fatal("duplicate scheduled slot accepted")
	}
	mustExecute(t, connection, `INSERT INTO scheduled_task_runs(task_id,task_name,scheduled_for,status,trigger_kind)
		VALUES (42,'manual',600,'pending','manual'),(42,'manual',600,'pending','manual')`)
	if value := scalar[int64](t, connection, "SELECT min(id) FROM scheduled_task_runs WHERE trigger_kind='manual'"); value != 2001 {
		t.Fatalf("lost sequence high water: %d", value)
	}
	if count := scalar[int](t, connection, "SELECT count(*) FROM scheduled_task_runs"); count != 3 {
		t.Fatalf("history or manual runs lost: %d", count)
	}
	if summary, err := Migrate(context.Background(), filename); err != nil || summary != (Summary{20, 20}) {
		t.Fatalf("repeat migration failed: %+v %v", summary, err)
	}
}

func TestNotificationCorruptionRollsBackSchemaAndPreservesRows(t *testing.T) {
	for _, value := range []string{`null`, `[null]`, `[1]`, `{}`, `not-json`} {
		t.Run(value, func(t *testing.T) {
			filename, connection := historicalDatabase(t, 13)
			mustExecute(t, connection, `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'user','hash','salt',1,1); INSERT INTO notification_settings(user_id,created_at,updated_at) VALUES(1,1,1)`)
			mustExecute(t, connection, "UPDATE notification_settings SET directions=?", value)
			_, err := Migrate(context.Background(), filename)
			if !errors.Is(err, ErrNotificationState) {
				t.Fatal(err)
			}
			if version := scalar[int](t, connection, "PRAGMA user_version"); version != 13 {
				t.Fatal(version)
			}
			if saved := scalar[string](t, connection, "SELECT directions FROM notification_settings"); saved != value {
				t.Fatal(saved)
			}
			if count := scalar[int](t, connection, "SELECT count(*) FROM sqlite_schema WHERE name='notification_settings_v14'"); count != 0 {
				t.Fatal(count)
			}
		})
	}
}

func TestVersionFailureRollsBackAllEarlierStatementsInThatVersion(t *testing.T) {
	for _, item := range []struct {
		version          int
		conflict, absent string
	}{
		{8, "security_audit_maintenance", "security_audit_events"},
		{9, "delivery_leases", "delivery_checkpoints"},
		{11, "invite_code_uses", "invite_codes_v12"},
		{17, "cfp_seed_imports", "cfp_journals"},
		{19, "scheduled_task_runs_v20", "idx_scheduled_task_runs_slot"},
	} {
		t.Run(fmt.Sprint(item.version), func(t *testing.T) {
			filename, connection := historicalDatabase(t, item.version)
			mustExecute(t, connection, "CREATE TABLE "+item.conflict+"(marker TEXT)")
			if _, err := Migrate(context.Background(), filename); err == nil {
				t.Fatal("conflict accepted")
			}
			if version := scalar[int](t, connection, "PRAGMA user_version"); version != item.version {
				t.Fatal(version)
			}
			if count := scalar[int](t, connection, "SELECT count(*) FROM sqlite_schema WHERE name=?", item.absent); count != 0 {
				t.Fatal("partial schema committed")
			}
		})
	}
}

func TestInviteMigrationPreservesUsesAndOnlyNewestUnrevoked(t *testing.T) {
	filename, connection := historicalDatabase(t, 11)
	mustExecute(t, connection, `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'creator','hash','salt',1,1),(2,'member','hash','salt',1,1);
INSERT INTO invite_codes(id,code,created_by,used_by,used_at,created_at) VALUES(1,'used',1,2,NULL,1),(2,'latest',1,NULL,NULL,2)`)
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	if used := scalar[int](t, connection, "SELECT use_count FROM invite_codes WHERE id=1"); used != 1 {
		t.Fatal(used)
	}
	if usedAt := scalar[float64](t, connection, "SELECT used_at FROM invite_code_uses WHERE invite_code_id=1"); usedAt != 1 {
		t.Fatal(usedAt)
	}
	if latest := scalar[int](t, connection, "SELECT id FROM invite_codes WHERE revoked_at IS NULL"); latest != 2 {
		t.Fatal(latest)
	}
	if remaining := scalar[float64](t, connection, "SELECT min(expires_at) FROM invite_codes") - nowSeconds(); remaining < 604790 {
		t.Fatal(remaining)
	}
}

func TestTypedSchemaConstraintsSurviveMigration(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	connection := testConnection(t, filename)
	mustExecute(t, connection, "PRAGMA foreign_keys=ON")
	mustExecute(t, connection, `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'creator','hash','salt',1,1)`)
	for _, statement := range []string{
		`INSERT INTO notification_settings(user_id,keywords,created_at,updated_at) VALUES(1,'[null]',1,1)`,
		`INSERT INTO notification_settings(user_id,keywords,created_at,updated_at) VALUES(1,'{}',1,1)`,
		`UPDATE users SET token_generation=-1 WHERE id=1`,
		`INSERT INTO delivery_runs(external_id,workflow,scope_key,db_name,trigger_kind,mode,status,created_at,updated_at) VALUES('x','notify','s','db','scheduled','execute','running',1,1)`,
	} {
		if err := execute(context.Background(), connection, statement); err == nil {
			t.Fatalf("constraint missing: %s", statement)
		}
	}
	mustExecute(t, connection, `INSERT INTO notification_settings(user_id,keywords,created_at,updated_at) VALUES(1,'["ok"]',1,1)`)
	if err := execute(context.Background(), connection, `UPDATE notification_settings SET directions='[1]'`); err == nil {
		t.Fatal("update trigger missing")
	}
}

func TestVersionEightMaterializesOnlyHistoricalEntryDefaults(t *testing.T) {
	for _, version := range []int{0, 1, 7, 8} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			filename, connection := historicalDatabase(t, version)
			if _, err := Migrate(context.Background(), filename); err != nil {
				t.Fatal(err)
			}
			count := scalar[int](t, connection, "SELECT count(*) FROM runtime_settings")
			expected := 0
			if version >= 1 && version <= 7 {
				expected = 3
			}
			if count != expected {
				t.Fatalf("got %d want %d", count, expected)
			}
		})
	}
}

func TestTrackingNormalizationRollbackAndLegacyCommandDisable(t *testing.T) {
	filename, connection := historicalDatabase(t, 1)
	mustExecute(t, connection, `INSERT INTO scheduled_tasks(name,command,cron,created_at,updated_at) VALUES ('old','unsafe legacy command','* * * * *',1,1)`)
	if _, err := Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	if value := scalar[string](t, connection, "SELECT legacy_command FROM scheduled_tasks"); value != "unsafe legacy command" {
		t.Fatal(value)
	}
	if value := scalar[int](t, connection, "SELECT enabled FROM scheduled_tasks"); value != 0 {
		t.Fatal(value)
	}
	if value := scalar[string](t, connection, "SELECT timezone||':'||timeout_seconds||':'||coalesce FROM scheduled_tasks"); value != "UTC:3600:1" {
		t.Fatal(value)
	}

	filename, connection = historicalDatabase(t, 10)
	mustExecute(t, connection, `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'user','hash','salt',1,1); INSERT INTO folders(user_id,name,is_tracking,created_at,updated_at) VALUES(1,'first',1,1,1),(1,'second',1,1,1); CREATE INDEX idx_folders_one_tracking_per_user ON folders(name)`)
	if _, err := Migrate(context.Background(), filename); err == nil {
		t.Fatal("conflicting index accepted")
	}
	if count := scalar[int](t, connection, "SELECT count(*) FROM folders WHERE is_tracking=1"); count != 2 {
		t.Fatal("normalization escaped rollback")
	}
}

func TestProviderRetirementMalformedStateLeavesAllRowsUntouched(t *testing.T) {
	filename, connection := historicalDatabase(t, 18)
	mustExecute(t, connection, `INSERT INTO runtime_settings VALUES ('index_provider_routes','{"x":"cnki_oversea"}',10),('provider_proxy_policy','{"cnki_oversea":null}',20)`)
	if _, err := Migrate(context.Background(), filename); !errors.Is(err, ErrProviderState) {
		t.Fatal(err)
	}
	if value := scalar[string](t, connection, "SELECT value FROM runtime_settings WHERE key='index_provider_routes'"); !strings.Contains(value, "cnki_oversea") {
		t.Fatal(value)
	}
	if version := scalar[int](t, connection, "PRAGMA user_version"); version != 18 {
		t.Fatal(version)
	}
}

func TestProviderOrderFieldOrderIsStable(t *testing.T) {
	orders, err := parseOrders(`{"catalogs":{"z":[],"a":["cnki"]},"default":[]}`)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := encodeJson(orders)
	if err != nil || encoded != `{"default":[],"catalogs":{"a":["cnki"],"z":[]}}` {
		t.Fatalf("%s %v", encoded, err)
	}
	if !reflect.DeepEqual(orders.Default, []string{}) {
		t.Fatal("empty became null")
	}
}

func TestMalformedSchemaDoesNotPreemptVersionCheck(t *testing.T) {
	for _, version := range []int{20, 21} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			filename := filepath.Join(t.TempDir(), "auth.sqlite")
			connection := testConnection(t, filename)
			mustExecute(t, connection, fmt.Sprintf("CREATE TABLE marker(value TEXT); PRAGMA user_version=%d; PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql='CREATE TABLE marker(' WHERE name='marker'", version))
			connection.Close()
			before, err := os.ReadFile(filename)
			if err != nil {
				t.Fatal(err)
			}
			summary, err := Migrate(context.Background(), filename)
			if version == 20 && (err != nil || summary != (Summary{20, 20})) {
				t.Fatalf("current short circuit: %+v %v", summary, err)
			}
			var unsupported UnsupportedVersion
			if version == 21 && (!errors.As(err, &unsupported) || unsupported.Found != 21) {
				t.Fatalf("future priority: %v", err)
			}
			after, err := os.ReadFile(filename)
			if err != nil {
				t.Fatal(err)
			}
			if string(before) != string(after) {
				t.Fatal("database bytes changed")
			}
		})
	}
}

func TestStrictJsonRejectsLossySyntaxAndValidatesOverwrittenValues(t *testing.T) {
	for _, raw := range []string{`["\ud800"]`, `["\udc00"]`, string([]byte{'[', '"', 255, '"', ']'}), `{"x":1e999,"x":"cnki"}`, strings.Repeat("[", 128) + "0" + strings.Repeat("]", 128)} {
		if validJson(raw) {
			t.Fatalf("accepted invalid input %q", raw)
		}
	}
	if _, err := parseOrders(`{"default":[],"catalogs":{"x":[null],"x":[]}}`); err == nil {
		t.Fatal("invalid overwritten typed map value accepted")
	}
	orders, err := parseOrders(`{"default":[],"catalogs":{"x":["cnki"],"x":[]}}`)
	if err != nil || len(orders.Catalogs["x"]) != 0 {
		t.Fatalf("valid last map value: %+v %v", orders, err)
	}
	for _, value := range []string{"\u2028", "\u2029", `\u2028`, `\u2029`, "😀"} {
		encoded, err := encodeJson([]string{value})
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := stringList(json.RawMessage(encoded))
		if err != nil || !reflect.DeepEqual(decoded, []string{value}) {
			t.Fatalf("changed value %q: %q %v", value, encoded, err)
		}
		if (value == "\u2028" || value == "\u2029") && !strings.Contains(encoded, value) {
			t.Fatalf("separator escaped: %q", encoded)
		}
	}
}
