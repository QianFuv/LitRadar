package settings

import (
	"errors"
	"net/url"
	"strings"
)

func normalizeOidcUrl(value string) (string, error) {
	value = strings.TrimSpace(value)
	if err := ValidateOidcUrl(value, true); err != nil {
		return "", err
	}
	return value, nil
}

// ValidateOidcUrl rejects credentials, query, fragments and insecure production endpoints.
func ValidateOidcUrl(value string, isDevelopment bool) error {
	if value == "" {
		return nil
	}
	location, err := url.Parse(value)
	if err != nil || location.Hostname() == "" || location.Opaque != "" || location.User != nil || location.RawQuery != "" || location.ForceQuery || location.Fragment != "" || strings.ContainsAny(value, "\\\r\n\t #") || (location.Scheme != "https" && !(isDevelopment && location.Scheme == "http")) {
		return errors.New("OIDC URLs must use HTTPS without credentials, query or fragment unless development mode permits HTTP")
	}
	return nil
}
