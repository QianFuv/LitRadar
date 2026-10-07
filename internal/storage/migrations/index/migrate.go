// Package index preserves the separate startup preflight and explicit content upgrade contracts.
package index

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
	"github.com/QianFuv/LitRadar/internal/storage/search"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// ErrIdentityState rejects noncanonical legacy journal identifiers before commit.
var ErrIdentityState = errors.New("index journal identity state is invalid for migration")

// ErrIdentityConflict rejects identifiers claimed by distinct canonical journals.
var ErrIdentityConflict = errors.New("index journal identity ownership conflicts across legacy journal rows")

// ErrInvalidQuery preserves the original invalid-schema and author-rebuild diagnostic.
var ErrInvalidQuery = errors.New("Query is not read-only")

// UnsupportedVersion identifies a future content database without admitting a write.
type UnsupportedVersion struct{ Found int }

// Error identifies the found and maximum supported content versions.
func (failure UnsupportedVersion) Error() string {
	return fmt.Sprintf("unsupported index database schema version %d; this binary supports up to %d", failure.Found, indexschema.Version)
}

// RebuildRequired preserves the exact legacy file selected by the operator.
type RebuildRequired struct {
	Path  string
	Found int
}

// Error names the exact legacy file requiring an operator-managed rebuild.
func (failure RebuildRequired) Error() string {
	return fmt.Sprintf("index database %s uses legacy schema version %d; move or delete that exact file and rebuild it as content schema v%d", failure.Path, failure.Found, indexschema.Version)
}

// Summary records the initially inspected and successfully committed content versions.
type Summary struct {
	FromVersion int `json:"from_version"`
	ToVersion   int `json:"to_version"`
}

type inspection struct {
	version int
	objects int64
}

func withReadOnly(ctx context.Context, filename string, read func(*sql.Conn) error) error {
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	return read(connection)
}

func inspect(ctx context.Context, filename string) (*inspection, error) {
	metadata, err := os.Stat(filename)
	if err != nil || metadata.Size() == 0 {
		return nil, nil
	}
	var result inspection
	err = withReadOnly(ctx, filename, func(connection *sql.Conn) error {
		if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&result.version); err != nil {
			return err
		}
		return connection.QueryRowContext(ctx, "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'").Scan(&result.objects)
	})
	if err != nil {
		return nil, err
	}
	return &result, nil
}

func validate(ctx context.Context, connection *sql.Conn, version int, isFull bool) error {
	var err error
	if isFull {
		err = indexschema.Validate(ctx, connection, version)
	} else {
		err = indexschema.ValidateStructure(ctx, connection, version)
	}
	var structure indexschema.InvalidStructure
	if errors.As(err, &structure) {
		return ErrInvalidQuery
	}
	return err
}

// Preflight upgrades genuine v4/v5 sources but only inspects supported v6-v9 structures.
func Preflight(ctx context.Context, filename string) (string, error) {
	return loggedPreflight(ctx, filename)
}

func preflight(ctx context.Context, filename string) (string, error) {
	observed, err := inspect(ctx, filename)
	if err != nil {
		return "", err
	}
	if observed != nil {
		if observed.version > indexschema.Version {
			return "", UnsupportedVersion{observed.version}
		}
		if observed.version >= indexschema.MinimumSupportedVersion {
			err := withReadOnly(ctx, filename, func(connection *sql.Conn) error { return validate(ctx, connection, observed.version, false) })
			return "schema_only", err
		}
	}
	_, err = Migrate(ctx, filename)
	return "full_integrity", err
}

// Migrate upgrades the entire content chain in one immediate transaction, then validates after commit.
func Migrate(ctx context.Context, filename string) (Summary, error) {
	return loggedMigration(ctx, filename)
}

func migrate(ctx context.Context, filename string) (Summary, error) {
	observed, err := inspect(ctx, filename)
	if err != nil {
		return Summary{}, err
	}
	from, isFinished, err := inspectMigrationEntrance(ctx, filename, observed)
	if isFinished || err != nil {
		return Summary{from, from}, err
	}
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		return Summary{from, from}, err
	}
	database, err := storage.OpenMigration(filename)
	if err != nil {
		return Summary{from, from}, err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return Summary{from, from}, err
	}
	defer connection.Close()
	if from == 0 {
		if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&from); err != nil {
			return Summary{}, err
		}
	}
	return migrateContentConnection(ctx, connection, from, observed != nil && observed.version >= 4)
}

// inspectMigrationEntrance validates existing versions before admitting writable storage.
func inspectMigrationEntrance(ctx context.Context, filename string, observed *inspection) (int, bool, error) {
	from := 0
	if observed != nil {
		from = observed.version
		if from > indexschema.Version {
			return from, true, UnsupportedVersion{from}
		}
		if from == indexschema.Version {
			err := withReadOnly(ctx, filename, func(connection *sql.Conn) error { return validate(ctx, connection, from, true) })
			return from, true, err
		}
		if from >= 4 {
			if err := withReadOnly(ctx, filename, func(connection *sql.Conn) error { return validate(ctx, connection, from, true) }); err != nil {
				return from, true, err
			}
		} else if from != 0 || observed.objects != 0 {
			return from, true, RebuildRequired{filename, from}
		}
	}
	return from, false, nil
}

// migrateContentConnection commits the whole chain before reporting final integrity failures.
func migrateContentConnection(ctx context.Context, connection *sql.Conn, from int, isUpgrade bool) (Summary, error) {
	if _, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;"); err != nil {
		return Summary{from, from}, err
	}
	if isUpgrade {
		if _, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=OFF"); err != nil {
			return Summary{from, from}, err
		}
	}
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return Summary{from, from}, err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	if err := apply(ctx, connection, from, isUpgrade); err != nil {
		return Summary{from, from}, err
	}
	if _, err := connection.ExecContext(ctx, "PRAGMA user_version=9; COMMIT"); err != nil {
		return Summary{from, from}, err
	}
	summary := Summary{from, indexschema.Version}
	if isUpgrade {
		if _, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=ON"); err != nil {
			return summary, err
		}
	}
	return summary, validate(ctx, connection, indexschema.Version, true)
}

func apply(ctx context.Context, connection *sql.Conn, version int, isUpgrade bool) error {
	if !isUpgrade {
		return createContentSchema(ctx, connection)
	}
	if version == 4 {
		if err := versionFive(ctx, connection); err != nil {
			return err
		}
	}
	if version <= 5 {
		if err := versionSix(ctx, connection); err != nil {
			return err
		}
	}
	if version <= 6 {
		if err := replaceSearch(ctx, connection, false); err != nil {
			return err
		}
	}
	if version < 8 {
		if _, err := connection.ExecContext(ctx, "DROP INDEX idx_article_change_events_order"); err != nil {
			return err
		}
	}
	return replaceSearch(ctx, connection, true)
}

// createContentSchema loads the fresh database tokenizer before creating its tables.
func createContentSchema(ctx context.Context, connection *sql.Conn) error {
	if err := storage.LoadSimple(connection); err != nil {
		return err
	}
	_, err := connection.ExecContext(ctx, indexschema.ContentTables)
	return err
}

// versionSix rebuilds historical article tables before checking their foreign keys.
func versionSix(ctx context.Context, connection *sql.Conn) error {
	if _, err := connection.ExecContext(ctx, versionSixSql); err != nil {
		return err
	}
	var violations int64
	if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM pragma_foreign_key_check").Scan(&violations); err != nil {
		return err
	}
	if violations != 0 {
		return ErrInvalidQuery
	}
	return nil
}

func replaceSearch(ctx context.Context, connection *sql.Conn, isSimple bool) error {
	tokenizer := "unicode61 remove_diacritics 2"
	if isSimple {
		if err := storage.LoadSimple(connection); err != nil {
			return err
		}
		tokenizer = "simple 0"
	}
	if _, err := connection.ExecContext(ctx, "DROP TABLE article_search; CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,content='',contentless_delete=1,tokenize='"+tokenizer+"');"); err != nil {
		return err
	}
	if err := search.Rebuild(ctx, connection); err != nil {
		if errors.Is(err, search.ErrInvalidAuthors) {
			return ErrInvalidQuery
		}
		return err
	}
	if !isSimple {
		_, err := connection.ExecContext(ctx, "INSERT INTO article_search(article_search) VALUES('optimize')")
		return err
	}
	return nil
}

// legacyJournalIdentity retains strict SQLite text types until semantic validation.
type legacyJournalIdentity struct {
	catalog, payload  storage.Text
	print, electronic storage.OptionalText
}

// versionFive reads every typed row before validating and persisting identity ownership.
func versionFive(ctx context.Context, connection *sql.Conn) error {
	if _, err := connection.ExecContext(ctx, versionFiveSql); err != nil {
		return err
	}
	journals, err := readLegacyJournalIdentities(ctx, connection)
	if err != nil {
		return err
	}
	owners, err := collectJournalIdentityOwners(journals)
	if err != nil {
		return err
	}
	return persistJournalIdentityOwners(ctx, connection, owners)
}

// readLegacyJournalIdentities closes the ordered typed read before any semantic checks.
func readLegacyJournalIdentities(ctx context.Context, connection *sql.Conn) ([]legacyJournalIdentity, error) {
	rows, err := connection.QueryContext(ctx, "SELECT catalog_id,issns_json,issn,eissn FROM journals ORDER BY catalog_id")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	journals := []legacyJournalIdentity{}
	for rows.Next() {
		var journal legacyJournalIdentity
		if err := rows.Scan(&journal.catalog, &journal.payload, &journal.print, &journal.electronic); err != nil {
			return nil, err
		}
		journals = append(journals, journal)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	rows.Close()
	return journals, nil
}

// collectJournalIdentityOwners validates journals in their original catalog order.
func collectJournalIdentityOwners(journals []legacyJournalIdentity) (map[string]string, error) {
	owners := map[string]string{}
	for _, journal := range journals {
		if err := registerLegacyJournalIdentity(owners, journal); err != nil {
			return nil, err
		}
	}
	return owners, nil
}

// registerJournalIdentity permits repeated keys only for the same canonical journal.
func registerJournalIdentity(owners map[string]string, kind, value, owner string) error {
	key := kind + "\x00" + value
	if prior, exists := owners[key]; exists && prior != owner {
		return ErrIdentityConflict
	}
	owners[key] = owner
	return nil
}

// registerLegacyJournalIdentity validates the catalog before sorted canonical ISSNs.
func registerLegacyJournalIdentity(owners map[string]string, journal legacyJournalIdentity) error {
	owner := string(journal.catalog)
	if len(owner) < 3 || !config.IsRuntimeName(owner) {
		return ErrIdentityState
	}
	if err := registerJournalIdentity(owners, "catalog_id", owner, owner); err != nil {
		return err
	}
	issns, err := legacyJournalIssns(journal)
	if err != nil {
		return err
	}
	slices.Sort(issns)
	for _, issn := range slices.Compact(issns) {
		if !canonicalIssn(issn) {
			return ErrIdentityState
		}
		if err := registerJournalIdentity(owners, "issn", issn, owner); err != nil {
			return err
		}
	}
	return nil
}

// legacyJournalIssns decodes every JSON string before appending nullable legacy columns.
func legacyJournalIssns(journal legacyJournalIdentity) ([]string, error) {
	if !jsonvalue.ValidJson(string(journal.payload)) || !strings.HasPrefix(strings.TrimSpace(string(journal.payload)), "[") {
		return nil, ErrIdentityState
	}
	var raw []json.RawMessage
	if json.Unmarshal([]byte(journal.payload), &raw) != nil {
		return nil, ErrIdentityState
	}
	issns := []string{}
	for _, value := range raw {
		var issn string
		if !strings.HasPrefix(strings.TrimSpace(string(value)), `"`) || json.Unmarshal(value, &issn) != nil {
			return nil, ErrIdentityState
		}
		issns = append(issns, issn)
	}
	for _, value := range []*string{journal.print.Value, journal.electronic.Value} {
		if value != nil {
			issns = append(issns, *value)
		}
	}
	return issns, nil
}

// persistJournalIdentityOwners inserts ownership only after all rows pass validation.
func persistJournalIdentityOwners(ctx context.Context, connection *sql.Conn, owners map[string]string) error {
	keys := make([]string, 0, len(owners))
	for key := range owners {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	for _, key := range keys {
		kind, value, _ := strings.Cut(key, "\x00")
		if _, err := connection.ExecContext(ctx, "INSERT INTO journal_identity_keys(identity_kind,identity_value,canonical_catalog_id) VALUES(?,?,?)", kind, value, owners[key]); err != nil {
			return err
		}
	}
	return nil
}

func canonicalIssn(value string) bool {
	if len(value) != 9 || value[4] != '-' {
		return false
	}
	compact := value[:4] + value[5:]
	total := 0
	for index := 0; index < 7; index++ {
		if compact[index] < '0' || compact[index] > '9' {
			return false
		}
		total += int(compact[index]-'0') * (8 - index)
	}
	check := (11 - total%11) % 11
	expected := byte('0' + check)
	if check == 10 {
		expected = 'X'
	}
	return compact[7] == expected
}
