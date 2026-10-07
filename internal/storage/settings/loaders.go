package settings

import (
	"context"
	"database/sql"
	"errors"
	"net/url"
	"os"
	"strconv"

	platformsqlite "github.com/QianFuv/LitRadar/internal/platform/sqlite"
)

// LoggingSettings carries unvalidated startup values to the logging initializer.
type LoggingSettings struct {
	LogFilter string
	LogFormat string
}

// LoadLogging reads only logging fields before migrations or deployment-key loading.
// A missing file or table returns defaults without creating a database or parent directory.
func LoadLogging(ctx context.Context, filename string) (LoggingSettings, error) {
	result := LoggingSettings{LogFilter: findDefinition("log_filter").Default, LogFormat: findDefinition("log_format").Default}
	if _, err := os.Stat(filename); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return result, nil
		}
		return LoggingSettings{}, err
	}
	database, err := openLoggingDatabase(filename)
	if err != nil {
		return LoggingSettings{}, err
	}
	defer database.Close()
	database.SetMaxOpenConns(1)
	var hasTable bool
	if err := database.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='runtime_settings')").Scan(&hasTable); err != nil {
		return LoggingSettings{}, err
	}
	if !hasTable {
		return result, nil
	}
	rows, err := database.QueryContext(ctx, "SELECT key,value FROM runtime_settings WHERE key IN ('log_filter','log_format')")
	if err != nil {
		return LoggingSettings{}, err
	}
	defer rows.Close()
	for rows.Next() {
		var field, value strictText
		if err := rows.Scan(&field, &value); err != nil {
			return LoggingSettings{}, err
		}
		if field == "log_filter" {
			result.LogFilter = string(value)
		} else {
			result.LogFormat = string(value)
		}
	}
	return result, rows.Err()
}

// openLoggingDatabase removes write-related pragmas from the read-only startup URI.
func openLoggingDatabase(filename string) (*sql.DB, error) {
	dsn, err := platformsqlite.FileUri(filename, "ro")
	if err != nil {
		return nil, err
	}
	location, err := url.Parse(dsn)
	if err != nil {
		return nil, err
	}
	query := location.Query()
	query.Del("_foreign_keys")
	query.Del("_synchronous")
	location.RawQuery = query.Encode()
	return sql.Open("sqlite3", location.String())
}

// LoadField validates only the requested setting; unrelated secrets cannot block non-secret consumers.
func (repository *Repository) LoadField(ctx context.Context, field string) (Value, error) {
	definition := findDefinition(field)
	if definition == nil {
		return Value{}, settingError(UnknownSetting, "Unknown runtime setting: "+field)
	}
	rows := map[string]storedRow{}
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var value strictText
		err := connection.QueryRowContext(ctx, "SELECT value FROM runtime_settings WHERE key=?", field).Scan(&value)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err == nil {
			rows[field] = storedRow{value: string(value)}
		}
		return err
	})
	if err != nil {
		return Value{}, err
	}
	result, err := repository.internal(definition, rows)
	result.UpdatedAt = nil
	return result, err
}

// AuditRetentionDays reads the validated maintenance period independently of integration secrets.
func (repository *Repository) AuditRetentionDays(ctx context.Context) (uint32, error) {
	value, err := repository.LoadField(ctx, "audit_retention_days")
	if err != nil {
		return 0, err
	}
	number, err := strconv.ParseUint(value.Value, 10, 32)
	return uint32(number), err
}

// DeliveryWorkerConcurrency reads the validated restart-scoped worker limit.
func (repository *Repository) DeliveryWorkerConcurrency(ctx context.Context) (int, error) {
	value, err := repository.LoadField(ctx, "delivery_worker_concurrency")
	if err != nil {
		return 0, err
	}
	return strconv.Atoi(value.Value)
}

// AiBaseUrls returns the approved endpoint catalog without reading unrelated secrets.
func (repository *Repository) AiBaseUrls(ctx context.Context) ([]string, error) {
	var result []string
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		result, err = AiBaseUrlsInConnection(ctx, connection)
		return err
	})
	return result, err
}

// AiBaseUrlsInConnection reads the allowlist in the caller's existing transaction snapshot.
func AiBaseUrlsInConnection(ctx context.Context, connection *sql.Conn) ([]string, error) {
	var stored strictText
	err := connection.QueryRowContext(ctx, "SELECT value FROM runtime_settings WHERE key='ai_allowed_base_urls'").Scan(&stored)
	if errors.Is(err, sql.ErrNoRows) {
		stored = strictText(findDefinition("ai_allowed_base_urls").Default)
	} else if err != nil {
		return nil, err
	}
	normalized, err := Normalize("ai_allowed_base_urls", string(stored))
	if err != nil {
		return nil, err
	}
	return commaEntries(normalized), nil
}
