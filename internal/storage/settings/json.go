package settings

import (
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

var errJsonShape = errors.New("invalid runtime JSON shape")

// readObject validates every typed value before applying map duplicate-key semantics.
// Struct callers additionally reject unknown, missing and duplicate members.
func readObject(raw string, fields []string, consume func(string, json.RawMessage) error) error {
	if !jsonvalue.ValidJson(raw) {
		return errJsonShape
	}
	decoder := json.NewDecoder(strings.NewReader(raw))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return errJsonShape
	}
	seen := map[string]bool{}
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return errJsonShape
		}
		name, ok := token.(string)
		if !ok {
			return errJsonShape
		}
		if fields != nil && (seen[name] || !slices.Contains(fields, name)) {
			return errJsonShape
		}
		seen[name] = true
		var value json.RawMessage
		if decoder.Decode(&value) != nil {
			return errJsonShape
		}
		if err := consume(name, value); err != nil {
			return err
		}
	}
	if fields != nil && len(seen) != len(fields) {
		return errJsonShape
	}
	return nil
}

func jsonString(raw json.RawMessage) (string, error) {
	var value string
	if len(raw) == 0 || raw[0] != '"' || json.Unmarshal(raw, &value) != nil {
		return "", errJsonShape
	}
	return value, nil
}

func jsonStrings(raw json.RawMessage) ([]string, error) {
	var entries []json.RawMessage
	if len(raw) == 0 || raw[0] != '[' || json.Unmarshal(raw, &entries) != nil {
		return nil, errJsonShape
	}
	result := make([]string, len(entries))
	for index, entry := range entries {
		value, err := jsonString(entry)
		if err != nil {
			return nil, err
		}
		result[index] = value
	}
	return result, nil
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

func rewriteProvider(value string) string {
	if value == "zjlib_cnki" {
		return "zjlib"
	}
	return value
}

func sortedKeys[Value any](values map[string]Value) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	return keys
}

func normalizeProxyPolicy(raw string) (string, error) {
	invalid := errors.New("Provider proxy policy must be a JSON object with boolean values and lowercase ASCII Provider names")
	result := map[string]bool{}
	err := readObject(raw, nil, func(key string, value json.RawMessage) error {
		if string(value) != "true" && string(value) != "false" {
			return errJsonShape
		}
		result[key] = string(value) == "true"
		return nil
	})
	if err != nil {
		return "", invalid
	}
	for key := range result {
		if !runtimeName(key) {
			return "", invalid
		}
	}
	return jsonvalue.EncodeJson(result)
}

func normalizeRoutes(raw string) (string, error) {
	invalid := func(detail string) error { return fmt.Errorf("Invalid index_provider_routes: %s", detail) }
	result := map[string]string{}
	if readObject(raw, nil, func(key string, value json.RawMessage) error {
		name, err := jsonString(value)
		result[key] = name
		return err
	}) != nil {
		return "", invalid("value must be a JSON object of catalog and Provider names")
	}
	if len(result) == 0 {
		return "", invalid("at least one catalog route is required")
	}
	for _, catalog := range sortedKeys(result) {
		if !runtimeName(catalog) {
			return "", invalid("catalog stems must use lowercase ASCII names")
		}
		provider := rewriteProvider(result[catalog])
		if !runtimeName(provider) {
			return "", invalid("provider names must use lowercase ASCII names")
		}
		result[catalog] = provider
	}
	return jsonvalue.EncodeJson(result)
}

// ProviderOrders preserves ordered fallbacks and deterministic catalog overrides.
type ProviderOrders struct {
	Default  []string            `json:"default"`
	Catalogs map[string][]string `json:"catalogs"`
}

func normalizeOrders(field, raw string) (string, error) {
	invalid := func(detail string) error { return fmt.Errorf("Invalid %s: %s", field, detail) }
	result := ProviderOrders{Catalogs: map[string][]string{}}
	err := readObject(raw, []string{"default", "catalogs"}, func(key string, value json.RawMessage) error {
		if key == "default" {
			var err error
			result.Default, err = jsonStrings(value)
			return err
		}
		return readObject(string(value), nil, func(catalog string, order json.RawMessage) error {
			values, err := jsonStrings(order)
			result.Catalogs[catalog] = values
			return err
		})
	})
	if err != nil {
		return "", invalid("value must contain only JSON default and catalogs fields")
	}
	validate := func(order []string) error {
		seen := map[string]bool{}
		for index, value := range order {
			value = rewriteProvider(value)
			order[index] = value
			if !runtimeName(value) {
				return invalid("Provider orders must contain lowercase ASCII names")
			}
			if seen[value] {
				return invalid("Provider orders must not contain duplicates")
			}
			seen[value] = true
		}
		return nil
	}
	if err := validate(result.Default); err != nil {
		return "", err
	}
	for _, catalog := range sortedKeys(result.Catalogs) {
		if !runtimeName(catalog) {
			return "", invalid("catalog stems must use lowercase ASCII names")
		}
		if err := validate(result.Catalogs[catalog]); err != nil {
			return "", err
		}
	}
	return jsonvalue.EncodeJson(result)
}

// TokenBucketPolicy is the exact integer policy used by authentication admission.
type TokenBucketPolicy struct {
	Capacity      uint64 `json:"capacity"`
	RefillTokens  uint64 `json:"refill_tokens"`
	RefillSeconds uint64 `json:"refill_seconds"`
}

// RateLimitPolicy preserves the strict persisted structure and its field order.
type RateLimitPolicy struct {
	LoginIp          TokenBucketPolicy `json:"login_ip"`
	Username         TokenBucketPolicy `json:"username"`
	RegisterIp       TokenBucketPolicy `json:"register_ip"`
	GlobalLogin      TokenBucketPolicy `json:"global_login"`
	GlobalRegister   TokenBucketPolicy `json:"global_register"`
	IpKeyLimit       uint64            `json:"ip_key_limit"`
	UsernameKeyLimit uint64            `json:"username_key_limit"`
}

// ParseRateLimitPolicy validates local buckets and the dominating global breakers.
func ParseRateLimitPolicy(raw string) (RateLimitPolicy, error) {
	invalid := errors.New("Authentication rate-limit policy must be strict bounded JSON")
	var result RateLimitPolicy
	buckets := map[string]*TokenBucketPolicy{"login_ip": &result.LoginIp, "username": &result.Username, "register_ip": &result.RegisterIp, "global_login": &result.GlobalLogin, "global_register": &result.GlobalRegister}
	fields := []string{"login_ip", "username", "register_ip", "global_login", "global_register", "ip_key_limit", "username_key_limit"}
	err := readObject(raw, fields, func(key string, value json.RawMessage) error {
		if bucket, exists := buckets[key]; exists {
			return readObject(string(value), []string{"capacity", "refill_tokens", "refill_seconds"}, func(name string, number json.RawMessage) error {
				parsed, err := strconv.ParseUint(string(number), 10, 64)
				if err != nil {
					return errJsonShape
				}
				switch name {
				case "capacity":
					bucket.Capacity = parsed
				case "refill_tokens":
					bucket.RefillTokens = parsed
				case "refill_seconds":
					bucket.RefillSeconds = parsed
				}
				return nil
			})
		}
		parsed, err := strconv.ParseUint(string(value), 10, 64)
		if err != nil {
			return errJsonShape
		}
		if key == "ip_key_limit" {
			result.IpKeyLimit = parsed
		} else {
			result.UsernameKeyLimit = parsed
		}
		return nil
	})
	if err != nil {
		return RateLimitPolicy{}, invalid
	}
	for _, bucket := range buckets {
		if bucket.Capacity < 1 || bucket.Capacity > 100000 || bucket.RefillTokens < 1 || bucket.RefillTokens > bucket.Capacity || bucket.RefillSeconds < 1 || bucket.RefillSeconds > 86400 {
			return RateLimitPolicy{}, invalid
		}
	}
	dominates := func(global, front TokenBucketPolicy) bool {
		return global.Capacity > front.Capacity && global.RefillTokens*front.RefillSeconds >= front.RefillTokens*global.RefillSeconds
	}
	if result.IpKeyLimit < 1 || result.IpKeyLimit > 65536 || result.UsernameKeyLimit < 1 || result.UsernameKeyLimit > 65536 || !dominates(result.GlobalLogin, result.LoginIp) || !dominates(result.GlobalLogin, result.Username) || !dominates(result.GlobalRegister, result.RegisterIp) || !dominates(result.GlobalRegister, result.Username) {
		return RateLimitPolicy{}, invalid
	}
	return result, nil
}
