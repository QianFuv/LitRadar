package meta

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"slices"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// CatalogReport distinguishes adoption and customization from actual file replacement.
type CatalogReport struct {
	Filename string `json:"filename"`
	Action   string `json:"action"`
}

// Report is returned only after committed state and successful rollback-file cleanup.
type Report struct {
	BundleVersion int64           `json:"bundle_version"`
	Catalogs      []CatalogReport `json:"catalogs"`
}

type state struct {
	version storage.Integer
	digest  string
}
type hooks struct {
	beforeReplacement func(string) error
	beforeStateWrite  func(string) error
}

// Prepare validates the whole bundle before serializing catalog classification and state/file changes.
func Prepare(ctx context.Context, configuration config.Config, directory string) (Report, error) {
	return prepare(ctx, configuration, directory, hooks{})
}

// prepare keeps the SQL transaction and compensation owners alive through final file cleanup.
func prepare(ctx context.Context, configuration config.Config, directory string, hook hooks) (Report, error) {
	bundle, err := validateBundle(directory)
	if err != nil {
		return Report{}, err
	}
	database, err := storage.Open(configuration.AuthDbPath, false, 1)
	if err != nil {
		return Report{}, err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return Report{}, err
	}
	defer connection.Close()
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return Report{}, err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	if err := admitBundleVersion(ctx, connection, bundle.version); err != nil {
		return Report{}, err
	}
	if err := ensureMetadataDirectory(configuration.MetaDir); err != nil {
		return Report{}, err
	}
	changes := []*replacement{}
	report, err := applyCatalogs(ctx, connection, configuration.MetaDir, bundle, hook, &changes)
	if err == nil {
		_, err = connection.ExecContext(ctx, "COMMIT")
	}
	if err != nil {
		connection.ExecContext(context.Background(), "ROLLBACK")
		return Report{}, rollbackAfter(err, changes)
	}
	if err := finishReplacements(changes); err != nil {
		return Report{}, err
	}
	return report, nil
}

// admitBundleVersion rejects downgrade before any persistent directory or file change.
func admitBundleVersion(ctx context.Context, connection *sql.Conn, version int64) error {
	var maximum storage.OptionalInteger
	if err := connection.QueryRowContext(ctx, "SELECT MAX(bundle_version) FROM managed_meta_catalogs").Scan(&maximum); err != nil {
		return err
	}
	if maximum.Value != nil && *maximum.Value > version {
		return Downgrade{*maximum.Value, version}
	}
	return nil
}

// ensureMetadataDirectory creates a missing directory and rejects an existing link or non-directory.
func ensureMetadataDirectory(directory string) error {
	info, err := os.Lstat(directory)
	if errors.Is(err, os.ErrNotExist) {
		return os.MkdirAll(directory, 0777)
	}
	if err != nil {
		return err
	}
	if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return InvalidBundle{"persistent metadata path " + directory + " must be a directory"}
	}
	return nil
}

// finishReplacements cleans committed recovery files in forward catalog order.
func finishReplacements(changes []*replacement) error {
	for _, change := range changes {
		if err := change.finish(); err != nil {
			return err
		}
	}
	return nil
}

// applyCatalogs returns a complete report only after every ordered catalog succeeds.
func applyCatalogs(ctx context.Context, connection *sql.Conn, directory string, bundle bundle, hook hooks, changes *[]*replacement) (Report, error) {
	report := Report{BundleVersion: bundle.version, Catalogs: make([]CatalogReport, 0, len(bundle.catalogs))}
	for _, catalog := range bundle.catalogs {
		action, err := applyCatalog(ctx, connection, directory, bundle.version, catalog, hook, changes)
		if err != nil {
			return Report{}, err
		}
		report.Catalogs = append(report.Catalogs, CatalogReport{catalog.filename, action})
	}
	return report, nil
}

// readCatalogState admits stored version and digest before inspecting the destination.
func readCatalogState(ctx context.Context, connection *sql.Conn, filename string) (state, bool, error) {
	var previous state
	var stored storage.Text
	err := connection.QueryRowContext(ctx, "SELECT bundle_version,applied_sha256 FROM managed_meta_catalogs WHERE filename=?", filename).Scan(&previous.version, &stored)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return state{}, false, err
	}
	hasState := err == nil
	previous.digest = string(stored)
	return previous, hasState, nil
}

// readCatalogDestination preserves missing-file admission and invalid-UTF-8 customization.
func readCatalogDestination(target string) (string, bool, error) {
	info, err := os.Lstat(target)
	isMissing := errors.Is(err, os.ErrNotExist)
	if err != nil && !isMissing {
		return "", false, err
	}
	if !isMissing && !info.Mode().IsRegular() {
		return "", false, InvalidBundle{"persistent catalog " + target + " must be a regular file"}
	}
	destination := ""
	if !isMissing {
		data, err := os.ReadFile(target)
		if err != nil {
			return "", false, err
		}
		destination, _ = CanonicalSha256(data)
	}
	return destination, isMissing, nil
}

// classifyCatalog gives missing/current content precedence over historical ownership.
func classifyCatalog(candidate catalog, version int64, previous state, hasState, isMissing bool, destination string) (string, bool, bool) {
	switch {
	case isMissing:
		return "created", true, true
	case destination == candidate.sha256:
		if hasState && previous.digest == candidate.sha256 && int64(previous.version) == version {
			return "unchanged", false, false
		}
		return "adopted", false, true
	case destination != "" && (slices.Contains(candidate.legacy, destination) || hasState && previous.digest == destination):
		return "updated", true, true
	}
	return "customized", false, false
}

// applyCatalog records a completed replacement before invoking its state-write hook.
func applyCatalog(ctx context.Context, connection *sql.Conn, directory string, version int64, candidate catalog, hook hooks, changes *[]*replacement) (string, error) {
	previous, hasState, err := readCatalogState(ctx, connection, candidate.filename)
	if err != nil {
		return "", err
	}
	target := filepath.Join(directory, candidate.filename)
	destination, isMissing, err := readCatalogDestination(target)
	if err != nil {
		return "", err
	}
	action, shouldReplace, shouldStore := classifyCatalog(candidate, version, previous, hasState, isMissing, destination)
	if shouldReplace {
		if err := replaceCatalog(target, candidate, hook, changes); err != nil {
			return "", err
		}
	}
	if shouldStore {
		if err := storeCatalogState(ctx, connection, candidate, version, hook); err != nil {
			return "", err
		}
	}
	return action, nil
}

// replaceCatalog invokes the replacement hook and appends only an applied recovery object.
func replaceCatalog(target string, candidate catalog, hook hooks, changes *[]*replacement) error {
	if hook.beforeReplacement != nil {
		if err := hook.beforeReplacement(candidate.filename); err != nil {
			return err
		}
	}
	change, err := replaceFile(target, candidate.data)
	if err != nil {
		return err
	}
	*changes = append(*changes, change)
	return nil
}

// storeCatalogState invokes its hook before updating managed ownership in the open transaction.
func storeCatalogState(ctx context.Context, connection *sql.Conn, candidate catalog, version int64, hook hooks) error {
	if hook.beforeStateWrite != nil {
		if err := hook.beforeStateWrite(candidate.filename); err != nil {
			return err
		}
	}
	_, err := connection.ExecContext(ctx, `INSERT INTO managed_meta_catalogs(filename,bundle_version,applied_sha256) VALUES(?,?,?) ON CONFLICT(filename) DO UPDATE SET bundle_version=excluded.bundle_version,applied_sha256=excluded.applied_sha256`, candidate.filename, version, candidate.sha256)
	return err
}
