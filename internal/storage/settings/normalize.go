package settings

import (
	"errors"
	"fmt"
	"net/netip"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
)

// Normalize validates a managed setting and returns its persisted canonical representation.
func Normalize(field, value string) (canonical string, err error) {
	defer func() {
		if err != nil {
			var known Error
			if !errors.As(err, &known) {
				err = settingError(InvalidSetting, err.Error())
			}
		}
	}()
	definition := findDefinition(field)
	if definition == nil {
		return "", settingError(UnknownSetting, "Unknown runtime setting: "+field)
	}
	switch definition.Parser {
	case "SecretPool", "ValuePool":
		return strings.Join(poolValues(value), ","), nil
	case "TrimmedText":
		return strings.TrimSpace(value), nil
	case "ProviderProxyUrl":
		return normalizeProxyUrl(value)
	case "ProviderProxyPolicy":
		return normalizeProxyPolicy(value)
	case "ExactOriginList", "HeaderValueList":
		return normalizeHeaders(field, value, definition.Parser == "ExactOriginList")
	case "Boolean":
		switch asciiLower(strings.TrimSpace(value)) {
		case "":
			return definition.Default, nil
		case "1", "true", "yes", "on":
			return "true", nil
		case "0", "false", "no", "off":
			return "false", nil
		default:
			return "", fmt.Errorf("Invalid boolean runtime setting %s: %s", field, value)
		}
	case "TrustedProxyCidrs":
		return normalizeNetworks(value)
	case "AuthRateLimitPolicy":
		policy, err := ParseRateLimitPolicy(value)
		if err != nil {
			return "", err
		}
		return jsonvalue.EncodeJson(policy)
	case "AuditRetentionDays":
		return normalizeBoundedInteger(value, 3650, "Security audit retention days must be between 1 and 3650")
	case "DeliveryWorkerConcurrency":
		return normalizeBoundedInteger(value, 16, "Delivery worker concurrency must be between 1 and 16")
	case "HttpsBaseUrlList":
		result := []string{}
		for _, entry := range commaEntries(value) {
			canonical, err := CanonicalizeBaseUrl(entry)
			if err != nil {
				return "", err
			}
			if !slices.Contains(result, canonical) {
				result = append(result, canonical)
			}
		}
		return strings.Join(result, ","), nil
	case "IndexProviderRoutes":
		return normalizeRoutes(value)
	case "ProviderOrder":
		return normalizeOrders(field, value)
	case "LogFormat":
		if value == "json" || value == "compact" {
			return value, nil
		}
		return "", errors.New("Invalid LitRadar log format")
	case "LogFilter":
		if validLogFilter(value) {
			return value, nil
		}
		return "", errors.New("Invalid LitRadar log filter")
	default:
		panic("unrecognized runtime setting parser")
	}
}

func asciiLower(value string) string {
	return strings.Map(func(character rune) rune {
		if character >= 'A' && character <= 'Z' {
			return character + 32
		}
		return character
	}, value)
}
func commaEntries(value string) []string {
	result := []string{}
	for _, entry := range strings.Split(value, ",") {
		entry = strings.TrimSpace(entry)
		if entry != "" {
			result = append(result, entry)
		}
	}
	return result
}
func normalizeBoundedInteger(value string, maximum uint64, message string) (string, error) {
	number, err := strconv.ParseUint(strings.TrimPrefix(strings.TrimSpace(value), "+"), 10, 32)
	if err != nil || number < 1 || number > maximum {
		return "", errors.New(message)
	}
	return strconv.FormatUint(number, 10), nil
}

func normalizeNetworks(value string) (string, error) {
	invalid := errors.New("Trusted proxy CIDRs must contain only valid IPv4 or IPv6 networks")
	result := []string{}
	for _, entry := range commaEntries(value) {
		addressText, prefixText, hasPrefix := strings.Cut(entry, "/")
		address, err := netip.ParseAddr(addressText)
		if err != nil || address.Zone() != "" {
			return "", invalid
		}
		prefix := uint64(address.BitLen())
		if hasPrefix {
			prefix, err = strconv.ParseUint(strings.TrimPrefix(prefixText, "+"), 10, 8)
			if err != nil || prefix > uint64(address.BitLen()) {
				return "", invalid
			}
		}
		network := netip.PrefixFrom(address, int(prefix)).Masked().String()
		if !slices.Contains(result, network) {
			result = append(result, network)
		}
	}
	return strings.Join(result, ","), nil
}

func normalizeHeaders(field, value string, isOrigin bool) (string, error) {
	label := map[string]string{"cors_allowed_origins": "CORS origin", "mcp_allowed_origins": "MCP allowed origin", "mcp_allowed_hosts": "MCP allowed host"}[field]
	entries := commaEntries(value)
	for _, entry := range entries {
		isValid := true
		for _, character := range []byte(entry) {
			if character != 9 && (character < 32 || character == 127) {
				isValid = false
				break
			}
		}
		if isValid && isOrigin {
			isValid = field == "mcp_allowed_origins" && entry == "null" || exactHttpOrigin(entry)
		}
		if !isValid {
			return "", fmt.Errorf("Invalid %s: %s", label, entry)
		}
	}
	return strings.Join(entries, ","), nil
}

// exactHttpOrigin preserves http::Uri authority acceptance, including opaque port text.
func exactHttpOrigin(value string) bool {
	if len(value) > 65534 {
		return false
	}
	scheme, authority, hasScheme := strings.Cut(value, "://")
	if !hasScheme || (asciiLower(scheme) != "http" && asciiLower(scheme) != "https") || authority == "" || strings.ContainsAny(authority, "/?#@") {
		return false
	}
	colons := 0
	isOpened, isClosed, hasPercent := false, false, false
	for _, character := range []byte(authority) {
		if !strings.ContainsRune("!$&'()*+,-.0123456789:;=ABCDEFGHIJKLMNOPQRSTUVWXYZ[]_abcdefghijklmnopqrstuvwxyz~%", rune(character)) {
			return false
		}
		switch character {
		case '%':
			hasPercent = true
		case ':':
			colons++
			if colons > 8 {
				return false
			}
		case '[':
			if hasPercent || isOpened {
				return false
			}
			isOpened = true
		case ']':
			if !isOpened || isClosed {
				return false
			}
			isClosed = true
			colons = 0
			hasPercent = false
		}
	}
	if isOpened != isClosed || colons > 1 || hasPercent {
		return false
	}
	if strings.HasPrefix(authority, "[") {
		return true
	}
	host, _, _ := strings.Cut(authority, ":")
	return host != ""
}
