package cfp

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

type step map[string]json.RawMessage

func (value step) text(key, fallback string) string {
	var result *string
	json.Unmarshal(value[key], &result)
	if result != nil {
		return *result
	}
	return fallback
}
func (value step) integer(key string, fallback int64) int64 {
	var result *int64
	json.Unmarshal(value[key], &result)
	if result != nil {
		return *result
	}
	return fallback
}
func (value step) flag(key string) bool {
	var result bool
	json.Unmarshal(value[key], &result)
	return result
}
func perform(ctx context.Context, repository *Repository, value step) (any, error) {
	source := value.text("source", "journal:a")
	lease := RefreshLease{SourceKey: source, Generation: value.integer("generation", 1), ExpiresAt: value.integer("expires", 110)}
	switch value.text("op", "") {
	case "import":
		return repository.ImportSeed(ctx, value.text("id", "seed"), []byte(value.text("payload", "")))
	case "begin":
		lease, err := repository.BeginRefresh(ctx, source, value.integer("now", 100), value.integer("seconds", 10))
		return map[string]any{"sourceKey": lease.SourceKey, "generation": lease.Generation, "expiresAt": lease.ExpiresAt}, err
	case "fail":
		return "ok", repository.FailRefresh(ctx, lease, value.text("reason", "failed"), value.flag("unsupported"))
	case "publish", "full":
		publication := Publication{Capture: value.text("capture", "<main>verified</main>"), CaptureFormat: value.text("format", "html"), SourceUrl: value.text("url", "https://example.org/cfp"), ConfigVersion: uint32(value.integer("version", 1)), Sources: []domain.Source{}, EmptyJournals: []domain.EmptyJournal{}}
		if raw, has := value["sources"]; has {
			if err := json.Unmarshal(raw, &publication.Sources); err != nil {
				return nil, err
			}
		}
		if raw, has := value["empty"]; has {
			if err := json.Unmarshal(raw, &publication.EmptyJournals); err != nil {
				return nil, err
			}
		}
		if value.text("op", "") == "full" {
			return "ok", repository.PublishFullText(ctx, lease, publication, value.integer("now", 105), uint64(value.integer("unresolved", 0)))
		}
		return "ok", repository.PublishRefresh(ctx, lease, publication, value.integer("now", 105))
	case "load":
		return repository.LoadJournals(ctx)
	case "originals":
		return repository.LoadOriginals(ctx, source)
	case "noop":
		return "ok", nil
	default:
		return nil, fmt.Errorf("unknown observer operation")
	}
}
func snapshot(t *testing.T, database *sql.DB) map[string]any {
	t.Helper()
	tables := map[string]any{}
	for _, entry := range []struct{ table, order string }{{"cfp_journals", "journal_key"}, {"cfp_journal_aliases", "catalog_id"}, {"cfp_sources", "source_key"}, {"cfp_source_journals", "source_key,journal_key"}, {"cfp_notices", "journal_key,notice_key"}, {"cfp_seed_imports", "seed_id"}} {
		rows, err := database.Query("SELECT * FROM " + entry.table + " ORDER BY " + entry.order)
		if err != nil {
			t.Fatal(err)
		}
		columns, err := rows.Columns()
		if err != nil {
			t.Fatal(err)
		}
		records := [][]any{}
		for rows.Next() {
			values := make([]any, len(columns))
			pointers := make([]any, len(columns))
			for index := range values {
				pointers[index] = &values[index]
			}
			if err = rows.Scan(pointers...); err != nil {
				t.Fatal(err)
			}
			for index, value := range values {
				switch value := value.(type) {
				case int64:
					values[index] = map[string]any{"integer": strconv.FormatInt(value, 10)}
				case float64:
					values[index] = map[string]any{"real": value}
				case string:
					values[index] = map[string]any{"text": value}
				case []byte:
					values[index] = map[string]any{"blob": hex.EncodeToString(value)}
				case nil:
				default:
					t.Fatalf("unexpected SQLite value %T", value)
				}
			}
			records = append(records, values)
		}
		if err = rows.Err(); err != nil {
			t.Fatal(err)
		}
		rows.Close()
		tables[entry.table] = records
	}
	return tables
}
func compareObservation(t *testing.T, actual any, expected json.RawMessage) {
	t.Helper()
	encoded, err := json.Marshal(actual)
	if err != nil {
		t.Fatal(err)
	}
	decode := func(data []byte) any {
		var result any
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.UseNumber()
		if err := decoder.Decode(&result); err != nil {
			t.Fatal(err)
		}
		return result
	}
	if !reflect.DeepEqual(decode(encoded), decode(expected)) {
		t.Fatalf("got %s\nwant %s", encoded, expected)
	}
}
func classify(err error) string {
	if errors.Is(err, ErrStaleRefresh) {
		return ErrStaleRefresh.Error()
	}
	var invalidError *InvalidError
	if errors.As(err, &invalidError) {
		return invalidError.Error()
	}
	var payloadError *PayloadError
	if errors.As(err, &payloadError) || err.Error() == "invalid CFP payload JSON" {
		return "json"
	}
	var syntax *json.SyntaxError
	if errors.As(err, &syntax) {
		return "json"
	}
	return "storage"
}

func TestOriginalStorageHistories(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/cfp/storage-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Name     string
			Steps    []step
			Expected []json.RawMessage
		}
	}
	if err = json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Cases) < 54 {
		t.Fatal("missing histories")
	}
	for _, entry := range fixture.Cases {
		t.Run(entry.Name, func(t *testing.T) {
			ctx := context.Background()
			path := filepath.Join(t.TempDir(), "auth.sqlite")
			if _, err := migration.Migrate(ctx, path); err != nil {
				t.Fatal(err)
			}
			repository, err := Open(path)
			if err != nil {
				t.Fatal(err)
			}
			defer repository.Close()
			for index, value := range entry.Steps {
				if query := value.text("sql", ""); query != "" {
					if _, err = repository.database.Exec(query); err != nil {
						t.Fatal(err)
					}
				}
				outcome, err := perform(ctx, repository, value)
				if err != nil {
					outcome = map[string]string{"error": classify(err)}
				}
				t.Run(strconv.Itoa(index), func(t *testing.T) {
					compareObservation(t, map[string]any{"outcome": outcome, "tables": snapshot(t, repository.database)}, entry.Expected[index])
				})
			}
		})
	}
}
