package scholarly

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"encoding/json"
	"errors"
	"math"
	"net/url"
	"os"
	"reflect"

	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/provider"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	native "github.com/mattn/go-sqlite3"
)

const worksetApplication = 0x4c524357
const worksetConfiguration = `PRAGMA cache_size=-4096; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
PRAGMA max_page_count=1048576;`
const worksetSchema = `PRAGMA page_size=4096; PRAGMA max_page_count=1048576;
PRAGMA application_id=1280459607; PRAGMA user_version=1;
CREATE TABLE metadata (id INTEGER PRIMARY KEY CHECK (id=1), owner TEXT NOT NULL, state TEXT NOT NULL, previous TEXT);
CREATE TABLE partitions (id INTEGER PRIMARY KEY, parent INTEGER, first_second INTEGER NOT NULL,
last_second INTEGER NOT NULL, depth INTEGER NOT NULL, expected INTEGER, received INTEGER NOT NULL DEFAULT 0,
cursor TEXT, retry INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL);
CREATE TABLE works (key TEXT PRIMARY KEY, payload TEXT NOT NULL, leaf INTEGER NOT NULL,
date TEXT NOT NULL, fingerprint TEXT NOT NULL, anchor TEXT, year INTEGER NOT NULL,
volume TEXT NOT NULL, issue TEXT NOT NULL, group_rank INTEGER NOT NULL DEFAULT 0,
sort_date INTEGER GENERATED ALWAYS AS (-CAST(replace(date,'-','') AS INTEGER)) STORED);
CREATE INDEX works_leaf ON works(leaf);
CREATE INDEX works_order ON works(fingerprint, date DESC, key);
CREATE INDEX works_emit ON works(group_rank,sort_date,key);
CREATE TABLE groups (fingerprint TEXT PRIMARY KEY, anchor TEXT, date TEXT NOT NULL,
year INTEGER NOT NULL, volume TEXT NOT NULL, issue TEXT NOT NULL, rank INTEGER);
CREATE INDEX groups_order ON groups(date DESC,year DESC,volume DESC,issue DESC,fingerprint);
CREATE UNIQUE INDEX groups_rank ON groups(rank);`

type worksetConnector struct {
	instance *native.SQLiteDriver
	dsn      string
}

func (connector *worksetConnector) Connect(ctx context.Context) (driver.Conn, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return connector.instance.Open(connector.dsn)
}
func (connector *worksetConnector) Driver() driver.Driver { return connector.instance }

func worksetStorageError(err error) error {
	if err == nil {
		return nil
	}
	var failure native.Error
	if errors.As(err, &failure) && (failure.Code == native.ErrCorrupt || failure.Code == native.ErrNotADB) {
		return invalidWorkset("Crossref workset database is damaged")
	}
	return &provider.Error{Kind: provider.Internal, Message: "Crossref workset: " + err.Error()}
}

// CrossrefWorkset is a disposable, exclusively used native connection with validated ownership.
type CrossrefWorkset struct {
	database   *sql.DB
	connection *sql.Conn
	root       string
	owner      worksetOwner
	state      CrossrefCheckpoint
}

func connectWorkset(filename, mode string) (*CrossrefWorkset, error) {
	dsn, err := platform.FileUri(filename, mode)
	if err != nil {
		return nil, worksetStorageError(err)
	}
	location, err := url.Parse(dsn)
	if err != nil {
		return nil, worksetStorageError(err)
	}
	query := location.Query()
	for _, key := range []string{"_journal_mode", "_synchronous", "_foreign_keys"} {
		query.Del(key)
	}
	query.Set("_busy_timeout", "5000")
	location.RawQuery = query.Encode()
	database := sql.OpenDB(&worksetConnector{&native.SQLiteDriver{NoFollow: true, DeferSynchronous: true}, location.String()})
	database.SetMaxOpenConns(1)
	connection, err := database.Conn(context.Background())
	if err != nil {
		database.Close()
		return nil, worksetStorageError(err)
	}
	return &CrossrefWorkset{database: database, connection: connection}, nil
}

func (workset *CrossrefWorkset) exec(query string, values ...any) error {
	_, err := workset.connection.ExecContext(context.Background(), query, values...)
	return worksetStorageError(err)
}

func (workset *CrossrefWorkset) row(query string, values ...any) *sql.Row {
	return workset.connection.QueryRowContext(context.Background(), query, values...)
}

// Close releases the connection without deleting resumable files.
func (workset *CrossrefWorkset) Close() error {
	connectionErr := workset.connection.Close()
	databaseErr := workset.database.Close()
	if connectionErr != nil {
		return worksetStorageError(connectionErr)
	}
	return worksetStorageError(databaseErr)
}

// CreateCrossrefWorkset exclusively creates a manifest before initializing the bounded database.
func CreateCrossrefWorkset(root, scope string, state CrossrefCheckpoint) (*CrossrefWorkset, error) {
	if err := state.Validate(); err != nil {
		return nil, err
	}
	root, err := prepareWorksetRoot(root)
	if err != nil {
		return nil, err
	}
	owner := makeWorksetOwner(scope, state)
	paths, err := worksetPaths(root, state.Token)
	if err != nil {
		return nil, err
	}
	for _, path := range paths {
		if _, err := os.Lstat(path); err == nil {
			return nil, invalidWorkset("Crossref workset token already exists")
		}
	}
	manifest, err := os.OpenFile(paths[4], os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0666)
	if err != nil {
		return nil, worksetStorageError(err)
	}
	encodedOwner, err := encodeWorksetStruct(owner)
	if err == nil {
		_, err = manifest.Write(encodedOwner)
	}
	if err == nil {
		err = manifest.Sync()
	}
	closeErr := manifest.Close()
	if err != nil {
		return nil, worksetStorageError(err)
	}
	if closeErr != nil {
		return nil, worksetStorageError(closeErr)
	}
	workset, err := connectWorkset(paths[0], "rwc")
	if err != nil {
		return nil, err
	}
	workset.root, workset.owner, workset.state = root, owner, state.Clone()
	if err = workset.exec(worksetConfiguration); err == nil {
		err = workset.exec(worksetSchema)
	}
	encodedState, encodeErr := state.MarshalJSON()
	if err == nil && encodeErr != nil {
		err = worksetStorageError(encodeErr)
	}
	if err == nil {
		err = workset.exec("INSERT INTO metadata VALUES(1, ?1, ?2, NULL)", string(encodedOwner), string(encodedState))
	}
	if err == nil {
		err = workset.initializeRoot()
	}
	if err != nil {
		workset.Close()
		return nil, err
	}
	return workset, nil
}

func (workset *CrossrefWorkset) initializeRoot() error {
	if workset.state.CreatedFrom == nil {
		return nil
	}
	return workset.exec("INSERT INTO partitions(id,parent,first_second,last_second,depth,status) VALUES(1,NULL,?1,?2,0,'probe')", *workset.state.CreatedFrom, workset.state.FrozenAt)
}

func renewedWorksetState(checkpoint CrossrefCheckpoint) (CrossrefCheckpoint, error) {
	state := checkpoint.Clone()
	token, err := newWorksetToken()
	if err != nil {
		return state, err
	}
	state.Token = token
	if state.Sequence == math.MaxUint64 {
		return state, invalidWorkset("Crossref sequence overflow")
	}
	state.Sequence++
	state.restartCollection()
	if state.CreatedFrom == nil {
		state.RootTotal = nil
	}
	return state, nil
}

// OpenCrossrefWorkset validates ownership before restoring or rebuilding recognized damaged data.
func OpenCrossrefWorkset(root, scope string, checkpoint CrossrefCheckpoint) (*CrossrefWorkset, bool, error) {
	if err := checkpoint.Validate(); err != nil {
		return nil, false, err
	}
	root, err := prepareWorksetRoot(root)
	if err != nil {
		return nil, false, err
	}
	owner := makeWorksetOwner(scope, checkpoint)
	paths, err := worksetPaths(root, checkpoint.Token)
	if err != nil {
		return nil, false, err
	}
	_, manifestErr := os.Lstat(paths[4])
	hasManifest := manifestErr == nil
	if !hasManifest {
		for _, path := range paths {
			if _, err := os.Lstat(path); err == nil {
				return nil, false, invalidWorkset("Crossref workset files have no ownership record")
			}
		}
	} else if err := validateWorksetFiles(root, owner); err != nil {
		return nil, false, err
	}
	_, databaseErr := os.Stat(paths[0])
	if hasManifest && databaseErr == nil {
		workset, replay, err := openExistingWorkset(root, owner, checkpoint)
		if err == nil || err.Error() != "Crossref workset database is damaged" {
			return workset, replay, err
		}
	}
	if hasManifest {
		if err := removeWorksetFiles(root, owner); err != nil {
			return nil, false, err
		}
	}
	state, err := renewedWorksetState(checkpoint)
	if err != nil {
		return nil, false, err
	}
	workset, err := CreateCrossrefWorkset(root, scope, state)
	return workset, true, err
}

func openExistingWorkset(root string, owner worksetOwner, checkpoint CrossrefCheckpoint) (result *CrossrefWorkset, didReplay bool, failure error) {
	paths, err := worksetPaths(root, owner.Token)
	if err != nil {
		return nil, false, err
	}
	workset, err := connectWorkset(paths[0], "rw")
	if err != nil {
		return nil, false, err
	}
	defer func() {
		if result == nil {
			workset.Close()
		}
	}()
	var application, version storage.Integer
	if err := workset.row("PRAGMA application_id").Scan(&application); err != nil {
		return nil, false, worksetStorageError(err)
	}
	if err := workset.row("PRAGMA user_version").Scan(&version); err != nil {
		return nil, false, worksetStorageError(err)
	}
	if version < 0 || version > math.MaxUint32 {
		return nil, false, worksetStorageError(errors.New("integer out of range"))
	}
	if application != worksetApplication || version != 1 {
		return nil, false, invalidWorkset("Crossref workset database is foreign or has an unsupported schema")
	}
	var ownerText, stateText storage.Text
	var previous storage.OptionalText
	if err := workset.row("SELECT owner,state,previous FROM metadata WHERE id=1").Scan(&ownerText, &stateText, &previous); err != nil {
		return nil, false, worksetStorageError(err)
	}
	var actualOwner worksetOwner
	if decodeWorksetStruct([]byte(ownerText), &actualOwner) != nil {
		return nil, false, invalidWorkset("invalid Crossref workset metadata")
	}
	if !reflect.DeepEqual(owner, actualOwner) {
		return nil, false, invalidWorkset("Crossref workset database context does not match")
	}
	var stored CrossrefCheckpoint
	if json.Unmarshal([]byte(stateText), &stored) != nil || stored.Validate() != nil {
		return nil, false, invalidWorkset("Crossref workset database is damaged")
	}
	if err := workset.exec(worksetConfiguration); err != nil {
		return nil, false, err
	}
	isEmitting := checkpoint.Phase.Kind == "emit" && stored.Phase.Kind == "emit" && stored.Phase.After == nil && checkpoint.Phase.Upper == stored.Phase.Upper && reflect.DeepEqual(checkpoint.Phase.Lower, stored.Phase.Lower) && reflect.DeepEqual(checkpoint.CreatedFrom, stored.CreatedFrom) && reflect.DeepEqual(checkpoint.RootTotal, stored.RootTotal) && checkpoint.Generation == stored.Generation && reflect.DeepEqual(checkpoint.Candidate, stored.Candidate) && checkpoint.Sequence >= stored.Sequence
	if !reflect.DeepEqual(stored, checkpoint) && !isEmitting {
		encoded, err := checkpoint.MarshalJSON()
		if err != nil {
			return nil, false, worksetStorageError(err)
		}
		if previous.Value == nil || *previous.Value != string(encoded) {
			return nil, false, invalidWorkset("Crossref workset does not match the confirmed core checkpoint")
		}
		didReplay = true
	}
	if stored.Phase.Kind == "emit" {
		if err := workset.exec("PRAGMA query_only=ON"); err != nil {
			return nil, false, err
		}
	}
	workset.root, workset.owner, workset.state = root, owner, stored
	if isEmitting {
		workset.state = checkpoint.Clone()
	}
	return workset, didReplay, nil
}

// Checkpoint returns independently owned state for the core's acknowledgement.
func (workset *CrossrefWorkset) Checkpoint() CrossrefCheckpoint { return workset.state.Clone() }

// Discard closes the connection before revalidating and removing its five owned paths.
func (workset *CrossrefWorkset) Discard() error {
	if err := workset.Close(); err != nil {
		return err
	}
	return removeWorksetFiles(workset.root, workset.owner)
}

// Recollect replaces owned disposable state while preserving the frozen traversal context.
func (workset *CrossrefWorkset) Recollect() (CrossrefCheckpoint, error) {
	state := workset.Checkpoint()
	if err := workset.Discard(); err != nil {
		return CrossrefCheckpoint{}, err
	}
	state, err := renewedWorksetState(state)
	if err != nil {
		return CrossrefCheckpoint{}, err
	}
	replacement, err := CreateCrossrefWorkset(workset.root, workset.owner.Scope, state)
	if err != nil {
		return CrossrefCheckpoint{}, err
	}
	result := replacement.Checkpoint()
	if err := replacement.Close(); err != nil {
		return CrossrefCheckpoint{}, err
	}
	return result, nil
}
