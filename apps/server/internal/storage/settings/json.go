package settings

import (
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
)

var errJSONShape = errors.New("invalid runtime JSON shape")

// IndexProviderRoutes maps catalog stems to their indexing providers.
type IndexProviderRoutes map[string]string

// ProviderProxyPolicy enables the managed proxy independently for each provider.
type ProviderProxyPolicy map[string]bool

// readObject validates every typed value before applying map duplicate-key semantics.
// Struct callers additionally reject unknown, missing and duplicate members.
func readObject(raw string, fields []string, consume func(string, json.RawMessage) error) error {
	if !jsonvalue.ValidJSON(raw) {
		return errJSONShape
	}
	decoder := json.NewDecoder(strings.NewReader(raw))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return errJSONShape
	}
	seen := map[string]bool{}
	for decoder.More() {
		if err := readObjectMember(decoder, fields, seen, consume); err != nil {
			return err
		}
	}
	if fields != nil && len(seen) != len(fields) {
		return errJSONShape
	}
	return nil
}

// readObjectMember validates strict member names before decoding and consuming their values.
func readObjectMember(decoder *json.Decoder, fields []string, seen map[string]bool, consume func(string, json.RawMessage) error) error {
	token, err := decoder.Token()
	if err != nil {
		return errJSONShape
	}
	name, ok := token.(string)
	if !ok {
		return errJSONShape
	}
	if fields != nil && (seen[name] || !slices.Contains(fields, name)) {
		return errJSONShape
	}
	seen[name] = true
	var value json.RawMessage
	if decoder.Decode(&value) != nil {
		return errJSONShape
	}
	return consume(name, value)
}

func jsonString(raw json.RawMessage) (string, error) {
	var value string
	if len(raw) == 0 || raw[0] != '"' || json.Unmarshal(raw, &value) != nil {
		return "", errJSONShape
	}
	return value, nil
}

func jsonStrings(raw json.RawMessage) ([]string, error) {
	var entries []json.RawMessage
	if len(raw) == 0 || raw[0] != '[' || json.Unmarshal(raw, &entries) != nil {
		return nil, errJSONShape
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
		if !isRuntimeNameByte(character, index) {
			return false
		}
	}
	return true
}

// isRuntimeNameByte permits a leading lowercase letter or digit and later separators.
func isRuntimeNameByte(character byte, index int) bool {
	return character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || index > 0 && (character == '.' || character == '_' || character == '-')
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
	invalid := settingError(InvalidSetting, "Provider proxy policy must be a JSON object with boolean values and lowercase ASCII Provider names")
	result := ProviderProxyPolicy{}
	err := readObject(raw, nil, func(key string, value json.RawMessage) error {
		if string(value) != "true" && string(value) != "false" {
			return errJSONShape
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
	return jsonvalue.EncodeJSON(result)
}

func normalizeRoutes(raw string) (string, error) {
	invalid := func(detail string) error {
		return settingError(InvalidSetting, fmt.Sprintf("Invalid index_provider_routes: %s", detail))
	}
	result := IndexProviderRoutes{}
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
		provider := result[catalog]
		if !runtimeName(provider) {
			return "", invalid("provider names must use lowercase ASCII names")
		}
	}
	return jsonvalue.EncodeJSON(result)
}

// ProviderOrders preserves ordered fallbacks and deterministic catalog overrides.
type ProviderOrders struct {
	Default  []string            `json:"default" nullable:"false"`
	Catalogs map[string][]string `json:"catalogs" nullable:"false"`
}

func normalizeOrders(field, raw string) (string, error) {
	invalid := func(detail string) error {
		return settingError(InvalidSetting, fmt.Sprintf("Invalid %s: %s", field, detail))
	}
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
		for _, value := range order {
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
	return jsonvalue.EncodeJSON(result)
}

// TokenBucketPolicy is the exact integer policy used by authentication admission.
type TokenBucketPolicy struct {
	Capacity      uint64 `json:"capacity"`
	RefillTokens  uint64 `json:"refill_tokens"`
	RefillSeconds uint64 `json:"refill_seconds"`
}

// RateLimitPolicy preserves the strict persisted structure and its field order.
type RateLimitPolicy struct {
	LoginIP          TokenBucketPolicy `json:"login_ip"`
	Username         TokenBucketPolicy `json:"username"`
	RegisterIP       TokenBucketPolicy `json:"register_ip"`
	GlobalLogin      TokenBucketPolicy `json:"global_login"`
	GlobalRegister   TokenBucketPolicy `json:"global_register"`
	IPKeyLimit       uint64            `json:"ip_key_limit"`
	UsernameKeyLimit uint64            `json:"username_key_limit"`
}

// ParseRateLimitPolicy validates local buckets and the dominating global breakers.
func ParseRateLimitPolicy(raw string) (RateLimitPolicy, error) {
	invalid := settingError(InvalidSetting, "Authentication rate-limit policy must be strict bounded JSON")
	var result RateLimitPolicy
	buckets := map[string]*TokenBucketPolicy{"login_ip": &result.LoginIP, "username": &result.Username, "register_ip": &result.RegisterIP, "global_login": &result.GlobalLogin, "global_register": &result.GlobalRegister}
	fields := []string{"login_ip", "username", "register_ip", "global_login", "global_register", "ip_key_limit", "username_key_limit"}
	err := readObject(raw, fields, func(key string, value json.RawMessage) error {
		return readRatePolicyValue(key, value, buckets, &result)
	})
	if err != nil {
		return RateLimitPolicy{}, invalid
	}
	for _, bucket := range buckets {
		if !isBoundedBucket(*bucket) {
			return RateLimitPolicy{}, invalid
		}
	}
	if !hasBoundedPolicyKeys(result) || !hasDominatingGlobalBuckets(result) {
		return RateLimitPolicy{}, invalid
	}
	return result, nil
}

// readRatePolicyValue assigns exact uint64 fields after strict nested bucket decoding.
func readRatePolicyValue(key string, value json.RawMessage, buckets map[string]*TokenBucketPolicy, result *RateLimitPolicy) error {
	if bucket, exists := buckets[key]; exists {
		return readTokenBucket(value, bucket)
	}
	parsed, err := strconv.ParseUint(string(value), 10, 64)
	if err != nil {
		return errJSONShape
	}
	if key == "ip_key_limit" {
		result.IPKeyLimit = parsed
	} else {
		result.UsernameKeyLimit = parsed
	}
	return nil
}

// readTokenBucket rejects missing, repeated or noninteger bucket fields.
func readTokenBucket(value json.RawMessage, bucket *TokenBucketPolicy) error {
	return readObject(string(value), []string{"capacity", "refill_tokens", "refill_seconds"}, func(name string, number json.RawMessage) error {
		parsed, err := strconv.ParseUint(string(number), 10, 64)
		if err != nil {
			return errJSONShape
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

// isBoundedBucket validates multiplication-safe limits before dominance is evaluated.
func isBoundedBucket(bucket TokenBucketPolicy) bool {
	return bucket.Capacity >= 1 && bucket.Capacity <= 100000 && bucket.RefillTokens >= 1 && bucket.RefillTokens <= bucket.Capacity && bucket.RefillSeconds >= 1 && bucket.RefillSeconds <= 86400
}

// hasBoundedPolicyKeys enforces finite IP and username cache capacities.
func hasBoundedPolicyKeys(policy RateLimitPolicy) bool {
	return policy.IPKeyLimit >= 1 && policy.IPKeyLimit <= 65536 && policy.UsernameKeyLimit >= 1 && policy.UsernameKeyLimit <= 65536
}

// bucketDominates requires greater capacity and at least the local refill rate.
func bucketDominates(global, front TokenBucketPolicy) bool {
	return global.Capacity > front.Capacity && global.RefillTokens*front.RefillSeconds >= front.RefillTokens*global.RefillSeconds
}

// hasDominatingGlobalBuckets checks both global breakers against each relevant local bucket.
func hasDominatingGlobalBuckets(policy RateLimitPolicy) bool {
	return bucketDominates(policy.GlobalLogin, policy.LoginIP) && bucketDominates(policy.GlobalLogin, policy.Username) && bucketDominates(policy.GlobalRegister, policy.RegisterIP) && bucketDominates(policy.GlobalRegister, policy.Username)
}
