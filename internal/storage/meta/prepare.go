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
	var maximum storage.OptionalInteger
	if err := connection.QueryRowContext(ctx, "SELECT MAX(bundle_version) FROM managed_meta_catalogs").Scan(&maximum); err != nil {
		return Report{}, err
	}
	if maximum.Value != nil && *maximum.Value > bundle.version {
		return Report{}, Downgrade{*maximum.Value, bundle.version}
	}
	info, err := os.Lstat(configuration.MetaDir)
	if errors.Is(err, os.ErrNotExist) {
		err = os.MkdirAll(configuration.MetaDir, 0777)
	} else if err == nil && (!info.IsDir() || info.Mode()&os.ModeSymlink != 0) {
		err = InvalidBundle{"persistent metadata path " + configuration.MetaDir + " must be a directory"}
	}
	if err != nil {
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
	for _, change := range changes {
		if err := change.finish(); err != nil {
			return Report{}, err
		}
	}
	return report, nil
}

func applyCatalogs(ctx context.Context, connection *sql.Conn, directory string, bundle bundle, hook hooks, changes *[]*replacement) (Report, error) {
	report := Report{BundleVersion: bundle.version, Catalogs: make([]CatalogReport, 0, len(bundle.catalogs))}
	for _, catalog := range bundle.catalogs {
		var previous state
		var stored storage.Text
		err := connection.QueryRowContext(ctx, "SELECT bundle_version,applied_sha256 FROM managed_meta_catalogs WHERE filename=?", catalog.filename).Scan(&previous.version, &stored)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return Report{}, err
		}
		hasState := err == nil
		previous.digest = string(stored)
		target := filepath.Join(directory, catalog.filename)
		info, err := os.Lstat(target)
		isMissing := errors.Is(err, os.ErrNotExist)
		if err != nil && !isMissing {
			return Report{}, err
		}
		if !isMissing && !info.Mode().IsRegular() {
			return Report{}, InvalidBundle{"persistent catalog " + target + " must be a regular file"}
		}
		destination := ""
		if !isMissing {
			data, err := os.ReadFile(target)
			if err != nil {
				return Report{}, err
			}
			destination, _ = CanonicalSha256(data)
		}
		action := "customized"
		shouldReplace, shouldStore := false, false
		switch {
		case isMissing:
			action, shouldReplace, shouldStore = "created", true, true
		case destination == catalog.sha256:
			if hasState && previous.digest == catalog.sha256 && int64(previous.version) == bundle.version {
				action = "unchanged"
			} else {
				action, shouldStore = "adopted", true
			}
		case destination != "" && (slices.Contains(catalog.legacy, destination) || hasState && previous.digest == destination):
			action, shouldReplace, shouldStore = "updated", true, true
		}
		if shouldReplace {
			if hook.beforeReplacement != nil {
				if err := hook.beforeReplacement(catalog.filename); err != nil {
					return Report{}, err
				}
			}
			change, err := replaceFile(target, catalog.data)
			if err != nil {
				return Report{}, err
			}
			*changes = append(*changes, change)
		}
		if shouldStore {
			if hook.beforeStateWrite != nil {
				if err := hook.beforeStateWrite(catalog.filename); err != nil {
					return Report{}, err
				}
			}
			if _, err := connection.ExecContext(ctx, `INSERT INTO managed_meta_catalogs(filename,bundle_version,applied_sha256) VALUES(?,?,?) ON CONFLICT(filename) DO UPDATE SET bundle_version=excluded.bundle_version,applied_sha256=excluded.applied_sha256`, catalog.filename, bundle.version, catalog.sha256); err != nil {
				return Report{}, err
			}
		}
		report.Catalogs = append(report.Catalogs, CatalogReport{catalog.filename, action})
	}
	return report, nil
}
