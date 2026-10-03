package auth

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"reflect"
	"slices"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
)

type providerOrders struct {
	Default  []string            `json:"default"`
	Catalogs map[string][]string `json:"catalogs"`
}

func encodeJson(value any) (string, error) { return domain.EncodeJson(value) }

func stringList(raw json.RawMessage) ([]string, error) {
	if !validJson(string(raw)) {
		return nil, ErrProviderState
	}
	var values []json.RawMessage
	if err := json.Unmarshal(raw, &values); err != nil || values == nil {
		return nil, ErrProviderState
	}
	result := make([]string, len(values))
	for index, value := range values {
		if len(value) == 0 || value[0] != '"' {
			return nil, ErrProviderState
		}
		if err := json.Unmarshal(value, &result[index]); err != nil {
			return nil, ErrProviderState
		}
	}
	return result, nil
}

func parseOrders(raw string) (providerOrders, error) {
	var fields map[string]json.RawMessage
	result := providerOrders{Default: []string{}, Catalogs: map[string][]string{}}
	if !validJson(raw) {
		return result, ErrProviderState
	}
	decoder := json.NewDecoder(strings.NewReader(raw))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return result, ErrProviderState
	}
	fields = map[string]json.RawMessage{}
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return result, ErrProviderState
		}
		name, ok := token.(string)
		if !ok || (name != "default" && name != "catalogs") {
			return result, ErrProviderState
		}
		if _, exists := fields[name]; exists {
			return result, ErrProviderState
		}
		var value json.RawMessage
		if decoder.Decode(&value) != nil {
			return result, ErrProviderState
		}
		fields[name] = value
	}
	if _, err := decoder.Token(); err != nil {
		return result, ErrProviderState
	}
	if _, err := decoder.Token(); err != io.EOF || len(fields) != 2 {
		return result, ErrProviderState
	}
	if value, exists := fields["default"]; exists {
		result.Default, err = stringList(value)
		if err != nil {
			return result, err
		}
	}
	if value, exists := fields["catalogs"]; exists {
		decoder := json.NewDecoder(bytes.NewReader(value))
		token, err := decoder.Token()
		if err != nil || token != json.Delim('{') {
			return result, ErrProviderState
		}
		for decoder.More() {
			token, err := decoder.Token()
			if err != nil {
				return result, ErrProviderState
			}
			key, ok := token.(string)
			if !ok {
				return result, ErrProviderState
			}
			var providers json.RawMessage
			if err := decoder.Decode(&providers); err != nil {
				return result, ErrProviderState
			}
			result.Catalogs[key], err = stringList(providers)
			if err != nil {
				return result, err
			}
		}
	}
	return result, nil
}

func versionSeven(ctx context.Context, connection *sql.Conn) error {
	if err := execute(ctx, connection, "CREATE TABLE IF NOT EXISTS runtime_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL DEFAULT '', updated_at REAL NOT NULL)"); err != nil {
		return err
	}
	type legacyRow struct {
		values  []string
		updated float64
		exists  bool
	}
	rows := make([]legacyRow, 3)
	for index, field := range []string{"article_detail_provider_order", "article_abstract_provider_order", "article_fulltext_provider_order"} {
		var value string
		err := connection.QueryRowContext(ctx, "SELECT value, updated_at FROM runtime_settings WHERE key = ?", field).Scan(&value, &rows[index].updated)
		if errors.Is(err, sql.ErrNoRows) {
			continue
		}
		if err != nil {
			return err
		}
		rows[index].exists = true
		rows[index].values = []string{}
		if strings.TrimSpace(value) == "" {
			continue
		}
		seen := map[string]bool{}
		for _, item := range strings.Split(value, ",") {
			name := strings.TrimSpace(item)
			if !runtimeName(name) || seen[name] {
				return ErrProviderState
			}
			seen[name] = true
			rows[index].values = append(rows[index].values, name)
		}
	}
	if rows[1].exists {
		rows[0] = rows[1]
	}
	for _, item := range []struct {
		key string
		row legacyRow
	}{{"article_abstract_provider_orders", rows[0]}, {"article_fulltext_provider_orders", rows[2]}} {
		if !item.row.exists {
			continue
		}
		value, err := encodeJson(providerOrders{item.row.values, map[string][]string{}})
		if err != nil {
			return err
		}
		if err := execute(ctx, connection, "INSERT INTO runtime_settings (key,value,updated_at) VALUES (?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at", item.key, value, item.row.updated); err != nil {
			return err
		}
	}
	return execute(ctx, connection, "DELETE FROM runtime_settings WHERE key IN ('article_detail_provider_order','article_abstract_provider_order','article_fulltext_provider_order')")
}

func runtimeName(value string) bool {
	if len(value) < 2 || len(value) > 128 {
		return false
	}
	for index, character := range []byte(value) {
		if character >= 'a' && character <= 'z' || character >= '0' && character <= '9' {
			continue
		}
		if index > 0 && (character == '.' || character == '_' || character == '-') {
			continue
		}
		return false
	}
	return true
}

func versionEight(ctx context.Context, connection *sql.Conn, fromVersion int) error {
	if fromVersion >= 1 && fromVersion <= 7 {
		updated := nowSeconds()
		for _, item := range []struct{ key, value string }{
			{"index_provider_routes", `{"ccf_computer_journals":"scholarly","chinese_journals":"cnki","english_journals":"scholarly"}`},
			{"article_abstract_provider_orders", `{"default":["scholarly","cnki"],"catalogs":{}}`},
			{"article_fulltext_provider_orders", `{"default":["zjlib_cnki"],"catalogs":{}}`},
		} {
			if err := execute(ctx, connection, "INSERT OR IGNORE INTO runtime_settings (key,value,updated_at) VALUES (?,?,?)", item.key, item.value, updated); err != nil {
				return err
			}
		}
	}
	return rewriteProviders(ctx, connection, false)
}

func rewriteProviders(ctx context.Context, connection *sql.Conn, retire bool) error {
	query := "SELECT key,value FROM runtime_settings WHERE key IN ('index_provider_routes','article_abstract_provider_orders','article_fulltext_provider_orders'"
	if retire {
		query += ",'provider_proxy_policy'"
	}
	rows, err := connection.QueryContext(ctx, query+")")
	if err != nil {
		return err
	}
	type entry struct{ key, value string }
	entries := []entry{}
	for rows.Next() {
		var item entry
		if err := rows.Scan(&item.key, &item.value); err != nil {
			rows.Close()
			return err
		}
		entries = append(entries, item)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return err
	}
	rewrite := func(name string) string {
		if retire {
			if name == "cnki_oversea" {
				return "cnki"
			}
			return name
		}
		switch name {
		case "cnki":
			return "cnki_oversea"
		case "zjlib_cnki":
			return "zjlib"
		default:
			return name
		}
	}
	for _, item := range entries {
		if !validJson(item.value) {
			return ErrProviderState
		}
		var transformed any
		switch item.key {
		case "index_provider_routes":
			var routes map[string]json.RawMessage
			if err := json.Unmarshal([]byte(item.value), &routes); err != nil || routes == nil {
				return ErrProviderState
			}
			result := map[string]string{}
			for catalog, provider := range routes {
				var name string
				if len(provider) == 0 || provider[0] != '"' || json.Unmarshal(provider, &name) != nil {
					return ErrProviderState
				}
				result[catalog] = rewrite(name)
			}
			transformed = result
		case "provider_proxy_policy":
			var policy map[string]json.RawMessage
			if err := json.Unmarshal([]byte(item.value), &policy); err != nil || policy == nil {
				return ErrProviderState
			}
			result := map[string]bool{}
			for name, raw := range policy {
				if string(raw) != "true" && string(raw) != "false" {
					return ErrProviderState
				}
				result[name] = string(raw) == "true"
			}
			if enabled, exists := result["cnki_oversea"]; exists {
				delete(result, "cnki_oversea")
				if _, exists := result["cnki"]; !exists {
					result["cnki"] = enabled
				}
			}
			transformed = result
		default:
			orderJson := item.value
			if retire {
				var value map[string]json.RawMessage
				if json.Unmarshal([]byte(orderJson), &value) != nil || value == nil {
					return ErrProviderState
				}
				orderJson, err = encodeJson(value)
				if err != nil {
					return ErrProviderState
				}
			}
			orders, err := parseOrders(orderJson)
			if err != nil {
				return err
			}
			convert := func(names []string) []string {
				result := []string{}
				seen := map[string]bool{}
				for _, name := range names {
					name = rewrite(name)
					if retire && seen[name] {
						continue
					}
					seen[name] = true
					result = append(result, name)
				}
				return result
			}
			orders.Default = convert(orders.Default)
			for catalog, providers := range orders.Catalogs {
				orders.Catalogs[catalog] = convert(providers)
			}
			transformed = orders
		}
		rewritten, err := encodeJson(transformed)
		if err != nil {
			return ErrProviderState
		}
		if retire {
			var before, after any
			if json.Unmarshal([]byte(item.value), &before) != nil || json.Unmarshal([]byte(rewritten), &after) != nil {
				return ErrProviderState
			}
			if reflect.DeepEqual(before, after) {
				continue
			}
		} else if rewritten == item.value {
			continue
		}
		if err := execute(ctx, connection, "UPDATE runtime_settings SET value=? WHERE key=?", rewritten, item.key); err != nil {
			return err
		}
	}
	return nil
}

func versionTwelve(ctx context.Context, connection *sql.Conn) error {
	names, err := columns(ctx, connection, "invite_codes")
	if err != nil {
		return err
	}
	if slices.Contains(names, "expires_at") {
		uses, err := columns(ctx, connection, "invite_code_uses")
		if err != nil {
			return err
		}
		if !slices.Equal(names, []string{"id", "code", "created_by", "used_by", "used_at", "created_at", "expires_at", "revoked_at", "max_uses", "use_count"}) || !slices.Equal(uses, []string{"id", "invite_code_id", "user_id", "used_at"}) {
			return errors.New("invalid query")
		}
		return execute(ctx, connection, inviteLifecycleIndexesSql)
	}
	migrated := nowSeconds()
	if err := execute(ctx, connection, inviteLifecycleTablesSql); err != nil {
		return err
	}
	if err := execute(ctx, connection, `INSERT INTO invite_codes_v12 (id,code,created_by,used_by,used_at,created_at,expires_at,revoked_at,max_uses,use_count)
SELECT id,code,created_by,used_by,CASE WHEN used_by IS NOT NULL THEN COALESCE(used_at,created_at) ELSE used_at END,created_at,MAX(created_at+?1,?2),NULL,1,CASE WHEN used_by IS NOT NULL OR used_at IS NOT NULL THEN 1 ELSE 0 END FROM invite_codes`, 604800, migrated+604800); err != nil {
		return err
	}
	if err := execute(ctx, connection, `UPDATE invite_codes_v12 SET revoked_at=MAX(?1,created_at) WHERE created_by IS NOT NULL AND revoked_at IS NULL AND id NOT IN (SELECT MAX(id) FROM invite_codes_v12 WHERE created_by IS NOT NULL GROUP BY created_by)`, migrated); err != nil {
		return err
	}
	if err := execute(ctx, connection, `INSERT INTO invite_code_uses (invite_code_id,user_id,used_at) SELECT id,used_by,COALESCE(used_at,created_at) FROM invite_codes_v12 WHERE used_by IS NOT NULL OR used_at IS NOT NULL;
DROP TABLE invite_codes; ALTER TABLE invite_codes_v12 RENAME TO invite_codes;`); err != nil {
		return err
	}
	if err := execute(ctx, connection, inviteLifecycleIndexesSql); err != nil {
		return err
	}
	return checkForeignKeys(ctx, connection)
}

func versionFourteen(ctx context.Context, connection *sql.Conn) error {
	names, err := columns(ctx, connection, "notification_settings")
	if err != nil {
		return err
	}
	if len(names) > 0 {
		rows, err := connection.QueryContext(ctx, "SELECT keywords,directions,selected_databases FROM notification_settings ORDER BY id")
		if err != nil {
			return err
		}
		for rows.Next() {
			var values [3]string
			if err := rows.Scan(&values[0], &values[1], &values[2]); err != nil {
				rows.Close()
				return err
			}
			for _, value := range values {
				if _, err := stringList(json.RawMessage(value)); err != nil {
					rows.Close()
					return ErrNotificationState
				}
			}
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return err
		}
	}
	if err := execute(ctx, connection, notificationSettingsV14TableSql); err != nil {
		return err
	}
	if len(names) > 0 {
		fields := "id,user_id,keywords,directions,selected_databases,delivery_method,pushplus_token,pushplus_template,pushplus_topic,pushplus_channel,sync_to_tracking_folder,ai_base_url,ai_api_key,ai_model,ai_system_prompt,ai_backup_base_url,ai_backup_api_key,ai_backup_model,ai_backup_system_prompt,ai_retry_attempts,enabled,created_at,updated_at"
		if err := execute(ctx, connection, "INSERT INTO notification_settings_v14 ("+fields+") SELECT "+fields+" FROM notification_settings; DROP TABLE notification_settings;"); err != nil {
			return err
		}
	}
	if err := execute(ctx, connection, "ALTER TABLE notification_settings_v14 RENAME TO notification_settings; CREATE INDEX idx_notification_settings_user ON notification_settings(user_id);"); err != nil {
		return err
	}
	if err := execute(ctx, connection, notificationSettingsV14TriggersSql); err != nil {
		return err
	}
	return checkForeignKeys(ctx, connection)
}
