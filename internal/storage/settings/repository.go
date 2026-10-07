package settings

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"log/slog"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	authstore "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

// Repository uses the same auth transaction boundary for settings and required audit events.
type Repository struct {
	auth  *authstore.Repository
	codec *secrets.Codec
}

// New borrows a migrated auth repository and deployment codec without owning their lifecycle.
func New(auth *authstore.Repository, codec *secrets.Codec) *Repository {
	return &Repository{auth, codec}
}

// Value is a trusted backend projection; values are never logged or serialized implicitly.
type Value struct {
	Field     string   `json:"-"`
	Value     string   `json:"-"`
	Source    string   `json:"-"`
	UpdatedAt *float64 `json:"-"`
}

func (value Value) String() string       { return "RuntimeSettingValue([REDACTED])" }
func (value Value) GoString() string     { return value.String() }
func (value Value) LogValue() slog.Value { return slog.StringValue(value.String()) }

// SecretItem exposes an encrypted removal reference and a Unicode-aware public mask.
type SecretItem struct {
	Reference   string `json:"reference"`
	MaskedValue string `json:"masked_value"`
}

func (item SecretItem) String() string       { return "RuntimeSecretItemInfo([REDACTED])" }
func (item SecretItem) GoString() string     { return item.String() }
func (item SecretItem) LogValue() slog.Value { return slog.StringValue(item.String()) }

// Info preserves the complete administrator form and disclosure contract.
type Info struct {
	Field         string       `json:"field"`
	Label         string       `json:"label"`
	Description   string       `json:"description"`
	Group         string       `json:"group"`
	Control       string       `json:"control"`
	ApplyMode     string       `json:"apply_mode"`
	AllowedValues []string     `json:"allowed_values"`
	InputType     string       `json:"input_type"`
	IsSecret      bool         `json:"is_secret"`
	Value         string       `json:"value"`
	HasValue      bool         `json:"has_value"`
	MaskedValue   string       `json:"masked_value"`
	SecretItems   []SecretItem `json:"secret_items"`
	Source        string       `json:"source"`
	UpdatedAt     *float64     `json:"updated_at"`
}

func (info Info) String() string       { return "RuntimeSettingInfo(" + info.Field + ", [REDACTED])" }
func (info Info) GoString() string     { return info.String() }
func (info Info) LogValue() slog.Value { return slog.StringValue(info.String()) }

// PoolUpdate applies removals before stable, deduplicated additions.
type PoolUpdate struct {
	Add    []string `json:"add"`
	Remove []string `json:"remove"`
}

func (update PoolUpdate) String() string       { return "RuntimeSecretPoolUpdate([REDACTED])" }
func (update PoolUpdate) GoString() string     { return update.String() }
func (update PoolUpdate) LogValue() slog.Value { return slog.StringValue(update.String()) }

type storedRow struct {
	value   string
	updated float64
}

type strictText string

func (value *strictText) Scan(source any) error {
	decoded, ok := source.(string)
	if !ok || !utf8.ValidString(decoded) {
		return errors.New("invalid runtime setting text column")
	}
	*value = strictText(decoded)
	return nil
}

type strictReal float64

func (value *strictReal) Scan(source any) error {
	switch decoded := source.(type) {
	case float64:
		*value = strictReal(decoded)
	case int64:
		*value = strictReal(decoded)
	default:
		return errors.New("invalid runtime setting real column")
	}
	return nil
}

func readRows(ctx context.Context, connection *sql.Conn) (map[string]storedRow, error) {
	rows, err := connection.QueryContext(ctx, "SELECT key,value,updated_at FROM runtime_settings")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := map[string]storedRow{}
	for rows.Next() {
		var key, value strictText
		var updated strictReal
		if err := rows.Scan(&key, &value, &updated); err != nil {
			return nil, err
		}
		result[string(key)] = storedRow{string(value), float64(updated)}
	}
	return result, rows.Err()
}

func (repository *Repository) internal(definition *definition, rows map[string]storedRow) (Value, error) {
	result := Value{Field: definition.Field, Value: definition.Default, Source: "default"}
	if row, exists := rows[definition.Field]; exists {
		result.Value = row.value
		result.Source = "database"
		result.UpdatedAt = &row.updated
		if definition.IsSecret {
			decrypted, err := repository.codec.Decrypt(row.value, secrets.RuntimeContext(definition.Field))
			if err != nil {
				return Value{}, err
			}
			result.Value = decrypted
		}
	}
	if !definition.IsSecret || definition.Field == "provider_proxy_url" {
		value, err := Normalize(definition.Field, result.Value)
		if err != nil {
			return Value{}, err
		}
		result.Value = value
	}
	return result, nil
}

// Load returns all known effective values in registry order, ignoring unrecognized persisted keys.
func (repository *Repository) Load(ctx context.Context) ([]Value, error) {
	var rows map[string]storedRow
	if err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error { var err error; rows, err = readRows(ctx, connection); return err }); err != nil {
		return nil, err
	}
	result := make([]Value, 0, len(definitions))
	for index := range definitions {
		value, err := repository.internal(&definitions[index], rows)
		if err != nil {
			return nil, err
		}
		result = append(result, value)
	}
	return result, nil
}

// List omits secret values while producing fresh authenticated per-item removal references.
func (repository *Repository) List(ctx context.Context) ([]Info, error) {
	values, err := repository.Load(ctx)
	if err != nil {
		return nil, err
	}
	result := make([]Info, 0, len(values))
	for index, value := range values {
		item, err := repository.settingInfo(&definitions[index], value)
		if err != nil {
			return nil, err
		}
		result = append(result, item)
	}
	return result, nil
}

// settingInfo constructs one administrator projection with fresh authenticated pool references.
func (repository *Repository) settingInfo(definition *definition, value Value) (Info, error) {
	item := Info{Field: definition.Field, Label: definition.Label, Description: definition.Description, Group: definition.Group, Control: definition.Control, ApplyMode: definition.ApplyMode, AllowedValues: append([]string{}, definition.AllowedValues...), InputType: definition.InputType, IsSecret: definition.IsSecret, Value: value.Value, HasValue: strings.TrimSpace(value.Value) != "", SecretItems: []SecretItem{}, Source: value.Source, UpdatedAt: value.UpdatedAt}
	if isSecretPool(definition) {
		for _, entry := range poolValues(value.Value) {
			reference, err := repository.codec.Encrypt(entry, secrets.PoolReferenceContext(definition.Field))
			if err != nil {
				return Info{}, err
			}
			item.SecretItems = append(item.SecretItems, SecretItem{reference, mask(entry)})
		}
		item.HasValue = len(item.SecretItems) > 0
	}
	if definition.IsSecret {
		item.Value = ""
		if item.HasValue {
			item.MaskedValue = "••••"
		}
	}
	return item, nil
}

// Update preserves omitted/blank/null secret semantics and commits settings together with their audit.
// The returned projection is read after commit; a projection failure cannot roll back an already committed update.
func (repository *Repository) Update(ctx context.Context, actor *identity.Id, values map[string]*string, pools map[string]PoolUpdate, audit *domain.AuditEvent) ([]Info, error) {
	now := float64(time.Now().UnixNano()) / 1e9
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if actor != nil {
			if err := authstore.RequireAdministrator(ctx, connection, *actor); err != nil {
				return err
			}
		}
		existing, err := readRows(ctx, connection)
		if err != nil {
			return err
		}
		fields := []string{}
		seen := map[string]bool{}
		for field := range values {
			fields = append(fields, field)
			seen[field] = true
		}
		for field := range pools {
			if !seen[field] {
				fields = append(fields, field)
			}
		}
		slices.Sort(fields)
		pending := map[string]string{}
		for _, field := range fields {
			definition := findDefinition(field)
			if definition == nil {
				return settingError(UnknownSetting, "Unknown runtime setting: "+field)
			}
			current, err := repository.internal(definition, existing)
			if err != nil {
				return err
			}
			value := current.Value
			if update, exists := values[field]; exists {
				if definition.IsSecret {
					if update == nil {
						value = ""
					} else if strings.TrimSpace(*update) != "" {
						value = strings.TrimSpace(*update)
					}
				} else {
					if update == nil {
						return settingError(CannotClearNonSecret, "Only secret runtime settings may be cleared: "+field)
					}
					value = *update
				}
			}
			if update, exists := pools[field]; exists {
				value, err = repository.applyPool(definition, value, update)
				if err != nil {
					return err
				}
			}
			if !definition.IsSecret || field == "provider_proxy_url" {
				value, err = Normalize(field, value)
				if err != nil {
					return err
				}
			}
			pending[field] = value
		}
		if err := repository.validateProxy(pending, existing); err != nil {
			return err
		}
		for _, field := range fields {
			stored := pending[field]
			if findDefinition(field).IsSecret {
				stored, err = repository.codec.Encrypt(stored, secrets.RuntimeContext(field))
				if err != nil {
					return err
				}
			}
			if _, err := connection.ExecContext(ctx, "INSERT INTO runtime_settings(key,value,updated_at) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", field, stored, now); err != nil {
				return err
			}
		}
		return authstore.InsertAudit(ctx, connection, audit)
	})
	if err != nil {
		return nil, err
	}
	return repository.List(ctx)
}

func (repository *Repository) validateProxy(pending map[string]string, existing map[string]storedRow) error {
	effective := func(field string) (string, error) {
		if value, exists := pending[field]; exists {
			return value, nil
		}
		value, err := repository.internal(findDefinition(field), existing)
		return value.Value, err
	}
	proxy, err := effective("provider_proxy_url")
	if err != nil {
		return err
	}
	policy, err := effective("provider_proxy_policy")
	if err != nil {
		return err
	}
	var enabled map[string]bool
	if err := json.Unmarshal([]byte(policy), &enabled); err != nil {
		return err
	}
	if proxy == "" {
		for _, value := range enabled {
			if value {
				return settingError(InvalidSetting, "Provider proxy URL is required when a Provider proxy is enabled")
			}
		}
	}
	return nil
}

func (repository *Repository) applyPool(definition *definition, value string, update PoolUpdate) (string, error) {
	invalid := func() error {
		return settingError(InvalidSecretPool, "Invalid runtime secret pool update: "+definition.Field)
	}
	if !isSecretPool(definition) {
		return "", invalid()
	}
	pool := poolValues(value)
	removals := map[string]bool{}
	for _, reference := range update.Remove {
		item, err := repository.codec.Decrypt(reference, secrets.PoolReferenceContext(definition.Field))
		if err != nil || item == "" || !slices.Contains(pool, item) {
			return "", invalid()
		}
		removals[item] = true
	}
	pool = slices.DeleteFunc(pool, func(item string) bool { return removals[item] })
	for _, addition := range update.Add {
		for _, item := range poolValues(addition) {
			if !slices.Contains(pool, item) {
				pool = append(pool, item)
			}
		}
	}
	return strings.Join(pool, "\n"), nil
}

func isSecretPool(definition *definition) bool {
	return definition.IsSecret && strings.HasSuffix(definition.Field, "_pool")
}
func poolValues(value string) []string {
	result := []string{}
	for _, part := range strings.FieldsFunc(value, func(character rune) bool { return character == ',' || character == ';' || character == '\n' }) {
		item := strings.TrimSpace(part)
		if item != "" && !slices.Contains(result, item) {
			result = append(result, item)
		}
	}
	return result
}
func mask(value string) string {
	characters := []rune(value)
	if len(characters) <= 5 {
		return strings.Repeat("*", len(characters))
	}
	return string(characters[:5]) + strings.Repeat("*", len(characters)-5)
}
