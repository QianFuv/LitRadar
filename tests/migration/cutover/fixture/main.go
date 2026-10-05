// Command fixture prepares and observes synthetic copied state for final-image migration tests.
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	authstore "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	"github.com/QianFuv/LitRadar/internal/testkit/primitives"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	if len(os.Args) != 3 {
		return errors.New("expected mode and owned fixture directory")
	}
	mode, root := os.Args[1], os.Args[2]
	switch mode {
	case "certificates":
		for _, name := range []string{"ai", "push"} {
			if err := primitives.Run("certificate", filepath.Join(root, name)); err != nil {
				return err
			}
		}
		return nil
	case "prepare":
		return prepare(root)
	case "snapshot":
		return snapshot(root)
	case "database":
		return databaseRequest(root)
	case "index-seed":
		return prepareIndex(root, false)
	case "index-notify-seed":
		return prepareIndex(root, true)
	case "cfp-seed":
		return prepareCfp(root)
	default:
		return errors.New("unknown mode")
	}
}

func databaseRequest(root string) error {
	marker, err := os.ReadFile(filepath.Join(root, ".litradar-e2e-root"))
	if err != nil || string(marker) != "litradar-full-stack-e2e-v1\n" {
		return errors.New("synthetic root marker required")
	}
	var request struct {
		Path string
		Sql  string
	}
	if err := json.NewDecoder(os.Stdin).Decode(&request); err != nil {
		return err
	}
	if !filepath.IsLocal(request.Path) {
		return errors.New("database must stay inside copied root")
	}
	library, err := filepath.Abs("libs/simple/windows/simple.dll")
	if err != nil {
		return err
	}
	mode := "ro"
	if request.Sql != "" {
		mode = "rw"
	}
	database, err := platform.Open(platform.Config{Filename: filepath.Join(root, request.Path), Mode: mode, NoFollow: true, MaxConnections: 1, SimpleLibrary: library})
	if err != nil {
		return err
	}
	defer database.Close()
	if request.Sql != "" {
		_, err := database.Exec(request.Sql)
		return err
	}
	rows, err := database.Query("SELECT name, type, sql FROM sqlite_schema ORDER BY name")
	if err != nil {
		return err
	}
	schema, tables := []any{}, []string{}
	for rows.Next() {
		var name, kind string
		var sql *string
		if err := rows.Scan(&name, &kind, &sql); err != nil {
			rows.Close()
			return err
		}
		schema = append(schema, map[string]any{"name": name, "type": kind, "sql": sql})
		if kind == "table" {
			tables = append(tables, name)
		}
	}
	if err := rows.Err(); err != nil {
		rows.Close()
		return err
	}
	rows.Close()
	var version int
	if err := database.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		return err
	}
	result := map[string]any{"schema": schema, "version": version}
	for _, table := range tables {
		rows, err := database.Query(`SELECT * FROM "` + strings.ReplaceAll(table, `"`, `""`) + `"`)
		if err != nil {
			return err
		}
		columns, err := rows.Columns()
		if err != nil {
			rows.Close()
			return err
		}
		values := []string{}
		for rows.Next() {
			fields, pointers := make([]any, len(columns)), make([]any, len(columns))
			for index := range fields {
				pointers[index] = &fields[index]
			}
			if err := rows.Scan(pointers...); err != nil {
				rows.Close()
				return err
			}
			cells := []any{}
			for _, field := range fields {
				var kind, value string
				switch typed := field.(type) {
				case nil:
					kind = "null"
				case string:
					kind, value = "text", typed
					if !utf8.ValidString(typed) {
						kind, value = "text-bytes", hex.EncodeToString([]byte(typed))
					}
				case int64:
					kind, value = "integer", strconv.FormatInt(typed, 10)
				case float64:
					kind, value = "real", strconv.FormatFloat(typed, 'g', -1, 64)
				case []byte:
					kind, value = "blob", hex.EncodeToString(typed)
				default:
					rows.Close()
					return fmt.Errorf("unexpected SQLite storage type %T", field)
				}
				cells = append(cells, []string{kind, value})
			}
			encoded, err := json.Marshal(cells)
			if err != nil {
				rows.Close()
				return err
			}
			values = append(values, string(encoded))
		}
		if err := rows.Err(); err != nil {
			rows.Close()
			return err
		}
		rows.Close()
		sort.Strings(values)
		result[table] = map[string]any{"columns": columns, "rows": values}
	}
	return json.NewEncoder(os.Stdout).Encode(result)
}

func prepare(root string) error {
	marker, err := os.ReadFile(filepath.Join(root, ".litradar-e2e-root"))
	if err != nil || string(marker) != "litradar-full-stack-e2e-v1\n" {
		return errors.New("synthetic root marker required")
	}
	ctx := context.Background()
	filename := filepath.Join(root, "data", "auth.sqlite")
	codec, err := secrets.Load(filepath.Join(root, "secret.key"))
	if err != nil {
		return err
	}
	defer codec.Close()
	repository, err := authstore.Open(filename)
	if err != nil {
		return err
	}
	defer repository.Close()
	endpoint := "https://www.pushplus.plus:8443/v1"
	if _, err := settings.New(repository, codec).Update(ctx, nil, map[string]*string{"ai_allowed_base_urls": &endpoint}, nil, nil); err != nil {
		return err
	}
	notifications, err := delivery.Open(filename)
	if err != nil {
		return err
	}
	defer notifications.Close()
	update := domain.DefaultNotificationSettingsUpdate()
	update.Keywords = []string{"evidence"}
	update.SelectedDatabases = []string{"full-stack.sqlite"}
	update.DeliveryMethod, update.AiBaseUrl, update.AiModel = "pushplus", endpoint, "synthetic-model"
	update.AiRetryAttempts = 1
	token, key := "synthetic-only", "synthetic-ai-only"
	update.PushplusToken = domain.SecretUpdate{IsPresent: true, Value: &token}
	update.AiApiKey = domain.SecretUpdate{IsPresent: true, Value: &key}
	if _, err := notifications.UpsertNotificationSettings(ctx, codec, 1, update); err != nil {
		return err
	}
	return nil
}

func snapshot(filename string) error {
	database, err := platform.Open(platform.Config{Filename: filename, Mode: "ro", NoFollow: true, MaxConnections: 1})
	if err != nil {
		return err
	}
	defer database.Close()
	result := map[string]any{}
	for _, table := range []string{"delivery_runs", "delivery_run_items", "delivery_dedupe", "delivery_checkpoints", "users", "sqlite_sequence", "security_audit_events"} {
		rows, err := database.Query("SELECT * FROM " + table + " ORDER BY rowid")
		if err != nil {
			return err
		}
		columns, err := rows.Columns()
		if err != nil {
			rows.Close()
			return err
		}
		values := []map[string]any{}
		for rows.Next() {
			fields, pointers := make([]any, len(columns)), make([]any, len(columns))
			for index := range fields {
				pointers[index] = &fields[index]
			}
			if err := rows.Scan(pointers...); err != nil {
				rows.Close()
				return err
			}
			entry := map[string]any{}
			for index, column := range columns {
				if integer, ok := fields[index].(int64); ok {
					entry[column] = strconv.FormatInt(integer, 10)
				} else {
					entry[column] = fields[index]
				}
			}
			values = append(values, entry)
		}
		if err := rows.Err(); err != nil {
			rows.Close()
			return err
		}
		rows.Close()
		result[table] = values
	}
	return json.NewEncoder(os.Stdout).Encode(result)
}
