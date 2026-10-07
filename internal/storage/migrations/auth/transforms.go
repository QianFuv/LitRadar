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

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

type providerOrders struct {
	Default  []string            `json:"default"`
	Catalogs map[string][]string `json:"catalogs"`
}

func encodeJson(value any) (string, error) { return jsonvalue.EncodeJson(value) }

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

// parseOrders retains strict outer fields and partial typed values on failure.
func parseOrders(raw string) (providerOrders, error) {
	result := providerOrders{Default: []string{}, Catalogs: map[string][]string{}}
	if !validJson(raw) {
		return result, ErrProviderState
	}
	fields, err := providerOrderFields(raw)
	if err != nil {
		return result, err
	}
	if value, exists := fields["default"]; exists {
		result.Default, err = stringList(value)
		if err != nil {
			return result, err
		}
	}
	if value, exists := fields["catalogs"]; exists {
		if err := decodeProviderCatalogs(value, result.Catalogs); err != nil {
			return result, err
		}
	}
	return result, nil
}

// providerOrderFields validates all outer field names before interpreting either typed value.
func providerOrderFields(raw string) (map[string]json.RawMessage, error) {
	decoder := json.NewDecoder(strings.NewReader(raw))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return nil, ErrProviderState
	}
	fields := map[string]json.RawMessage{}
	for decoder.More() {
		if err := decodeProviderOrderField(decoder, fields); err != nil {
			return nil, err
		}
	}
	if _, err := decoder.Token(); err != nil {
		return nil, ErrProviderState
	}
	if _, err := decoder.Token(); err != io.EOF || len(fields) != 2 {
		return nil, ErrProviderState
	}
	return fields, nil
}

// decodeProviderOrderField rejects unknown and duplicate top-level order fields.
func decodeProviderOrderField(decoder *json.Decoder, fields map[string]json.RawMessage) error {
	token, err := decoder.Token()
	if err != nil {
		return ErrProviderState
	}
	name, ok := token.(string)
	if !ok || (name != "default" && name != "catalogs") {
		return ErrProviderState
	}
	if _, exists := fields[name]; exists {
		return ErrProviderState
	}
	var value json.RawMessage
	if decoder.Decode(&value) != nil {
		return ErrProviderState
	}
	fields[name] = value
	return nil
}

// decodeProviderCatalogs validates every duplicate occurrence before retaining its last value.
func decodeProviderCatalogs(value json.RawMessage, catalogs map[string][]string) error {
	decoder := json.NewDecoder(bytes.NewReader(value))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return ErrProviderState
	}
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return ErrProviderState
		}
		key, ok := token.(string)
		if !ok {
			return ErrProviderState
		}
		var providers json.RawMessage
		if err := decoder.Decode(&providers); err != nil {
			return ErrProviderState
		}
		catalogs[key], err = stringList(providers)
		if err != nil {
			return err
		}
	}
	return nil
}

// versionSeven validates all historical orders before applying abstract precedence.
func versionSeven(ctx context.Context, connection *sql.Conn) error {
	if err := execute(ctx, connection, "CREATE TABLE IF NOT EXISTS runtime_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL DEFAULT '', updated_at REAL NOT NULL)"); err != nil {
		return err
	}
	rows, err := readLegacyProviderOrders(ctx, connection)
	if err != nil {
		return err
	}
	if rows[1].exists {
		rows[0] = rows[1]
	}
	for _, item := range []struct {
		key string
		row legacyProviderRow
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
		if isRuntimeNameLetterOrDigit(character) {
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

// rewriteProviders closes historical reads before transforming and updating selected values.
func rewriteProviders(ctx context.Context, connection *sql.Conn, retire bool) error {
	entries, err := readProviderSettings(ctx, connection, retire)
	if err != nil {
		return err
	}
	for _, item := range entries {
		rewritten, err := rewriteProviderValue(item.key, item.value, retire)
		if err != nil {
			return err
		}
		shouldUpdate, err := providerSettingChanged(item.value, rewritten, retire)
		if err != nil {
			return err
		}
		if !shouldUpdate {
			continue
		}
		if err := execute(ctx, connection, "UPDATE runtime_settings SET value=? WHERE key=?", rewritten, item.key); err != nil {
			return err
		}
	}
	return nil
}

// providerSettingRow carries the original SQL query order until all reads are closed.
type providerSettingRow struct{ key, value string }

// readProviderSettings collects selected values before updates on the same connection.
func readProviderSettings(ctx context.Context, connection *sql.Conn, retire bool) ([]providerSettingRow, error) {
	query := "SELECT key,value FROM runtime_settings WHERE key IN ('index_provider_routes','article_abstract_provider_orders','article_fulltext_provider_orders'"
	if retire {
		query += ",'provider_proxy_policy'"
	}
	rows, err := connection.QueryContext(ctx, query+")")
	if err != nil {
		return nil, err
	}
	entries := []providerSettingRow{}
	for rows.Next() {
		var item providerSettingRow
		if err := rows.Scan(&item.key, &item.value); err != nil {
			rows.Close()
			return nil, err
		}
		entries = append(entries, item)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return nil, err
	}
	return entries, nil
}

// rewriteProviderValue preserves each setting's distinct raw-map and typed-list admission.
func rewriteProviderValue(key, value string, retire bool) (string, error) {
	if !validJson(value) {
		return "", ErrProviderState
	}
	var transformed any
	var err error
	switch key {
	case "index_provider_routes":
		transformed, err = rewriteProviderRoutes(value, retire)
	case "provider_proxy_policy":
		transformed, err = retireProviderProxyPolicy(value)
	default:
		transformed, err = rewriteProviderOrders(value, retire)
	}
	if err != nil {
		return "", err
	}
	rewritten, err := encodeJson(transformed)
	if err != nil {
		return "", ErrProviderState
	}
	return rewritten, nil
}

// rewriteProviderName applies only the rename set belonging to the current migration direction.
func rewriteProviderName(name string, retire bool) string {
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

// rewriteProviderRoutes retains raw-map duplicate semantics before validating provider strings.
func rewriteProviderRoutes(value string, retire bool) (map[string]string, error) {
	var routes map[string]json.RawMessage
	if err := json.Unmarshal([]byte(value), &routes); err != nil || routes == nil {
		return nil, ErrProviderState
	}
	result := map[string]string{}
	for catalog, provider := range routes {
		var name string
		if len(provider) == 0 || provider[0] != '"' || json.Unmarshal(provider, &name) != nil {
			return nil, ErrProviderState
		}
		result[catalog] = rewriteProviderName(name, retire)
	}
	return result, nil
}

// retireProviderProxyPolicy gives an explicit domestic policy precedence over the retired provider.
func retireProviderProxyPolicy(value string) (map[string]bool, error) {
	var policy map[string]json.RawMessage
	if err := json.Unmarshal([]byte(value), &policy); err != nil || policy == nil {
		return nil, ErrProviderState
	}
	result := map[string]bool{}
	for name, raw := range policy {
		if string(raw) != "true" && string(raw) != "false" {
			return nil, ErrProviderState
		}
		result[name] = string(raw) == "true"
	}
	if enabled, exists := result["cnki_oversea"]; exists {
		delete(result, "cnki_oversea")
		if _, exists := result["cnki"]; !exists {
			result["cnki"] = enabled
		}
	}
	return result, nil
}

// rewriteProviderOrders coalesces outer duplicates only for retirement before typed parsing.
func rewriteProviderOrders(orderJson string, retire bool) (providerOrders, error) {
	if retire {
		var value map[string]json.RawMessage
		if json.Unmarshal([]byte(orderJson), &value) != nil || value == nil {
			return providerOrders{}, ErrProviderState
		}
		var err error
		orderJson, err = encodeJson(value)
		if err != nil {
			return providerOrders{}, ErrProviderState
		}
	}
	orders, err := parseOrders(orderJson)
	if err != nil {
		return orders, err
	}
	orders.Default = rewriteProviderOrder(orders.Default, retire)
	for catalog, providers := range orders.Catalogs {
		orders.Catalogs[catalog] = rewriteProviderOrder(providers, retire)
	}
	return orders, nil
}

// rewriteProviderOrder preserves sequence order and deduplicates only retired names.
func rewriteProviderOrder(names []string, retire bool) []string {
	result := []string{}
	seen := map[string]bool{}
	for _, name := range names {
		name = rewriteProviderName(name, retire)
		if retire && seen[name] {
			continue
		}
		seen[name] = true
		result = append(result, name)
	}
	return result
}

// providerSettingChanged uses decoded equality only for retirement to retain original formatting.
func providerSettingChanged(original, rewritten string, retire bool) (bool, error) {
	if retire {
		var before, after any
		if json.Unmarshal([]byte(original), &before) != nil || json.Unmarshal([]byte(rewritten), &after) != nil {
			return false, ErrProviderState
		}
		return !reflect.DeepEqual(before, after), nil
	}
	return rewritten != original, nil
}

func versionTwelve(ctx context.Context, connection *sql.Conn) error {
	names, err := columns(ctx, connection, "invite_codes")
	if err != nil {
		return err
	}
	if slices.Contains(names, "expires_at") {
		return validateExistingInviteLifecycle(ctx, connection, names)
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
		if err := validateNotificationLists(ctx, connection); err != nil {
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

// legacyProviderRow retains explicit existence and the original timestamp for CSV orders.
type legacyProviderRow struct {
	values  []string
	updated float64
	exists  bool
}

// readLegacyProviderOrders loads detail, abstract and fulltext rows in their historical order.
func readLegacyProviderOrders(ctx context.Context, connection *sql.Conn) ([]legacyProviderRow, error) {
	rows := make([]legacyProviderRow, 3)
	for index, field := range []string{"article_detail_provider_order", "article_abstract_provider_order", "article_fulltext_provider_order"} {
		var value string
		err := connection.QueryRowContext(ctx, "SELECT value, updated_at FROM runtime_settings WHERE key = ?", field).Scan(&value, &rows[index].updated)
		if errors.Is(err, sql.ErrNoRows) {
			continue
		}
		if err != nil {
			return nil, err
		}
		rows[index].exists = true
		rows[index].values = []string{}
		if strings.TrimSpace(value) == "" {
			continue
		}
		rows[index].values, err = legacyProviderNames(value)
		if err != nil {
			return nil, err
		}
	}
	return rows, nil
}

// legacyProviderNames rejects trimmed duplicates and empty CSV segments without sorting.
func legacyProviderNames(value string) ([]string, error) {
	values := []string{}
	seen := map[string]bool{}
	for _, item := range strings.Split(value, ",") {
		name := strings.TrimSpace(item)
		if !runtimeName(name) || seen[name] {
			return nil, ErrProviderState
		}
		seen[name] = true
		values = append(values, name)
	}
	return values, nil
}

// isRuntimeNameLetterOrDigit admits the same lowercase ASCII letters and leading digits.
func isRuntimeNameLetterOrDigit(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= '0' && character <= '9'
}

// validateExistingInviteLifecycle preserves exact column order and the index-only existing path.
func validateExistingInviteLifecycle(ctx context.Context, connection *sql.Conn, names []string) error {
	uses, err := columns(ctx, connection, "invite_code_uses")
	if err != nil {
		return err
	}
	if !slices.Equal(names, []string{"id", "code", "created_by", "used_by", "used_at", "created_at", "expires_at", "revoked_at", "max_uses", "use_count"}) || !slices.Equal(uses, []string{"id", "invite_code_id", "user_id", "used_at"}) {
		return errors.New("invalid query")
	}
	return execute(ctx, connection, inviteLifecycleIndexesSql)
}

// validateNotificationLists closes ordered historical rows before replacement-table DDL.
func validateNotificationLists(ctx context.Context, connection *sql.Conn) error {
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
	return nil
}
