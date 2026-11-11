// Package webhost serves the LitRadar single-page client from the Wails server build.
//
// It provides the static asset handler with SPA fallback and caching policy, the
// security-header middleware that also hides the Wails runtime endpoints, and the
// environment-driven configuration of the web host.
package webhost

import (
	"fmt"
	"strconv"
	"strings"
)

// Environment variables read by [LoadConfig]. The listen address uses the Wails
// built-in WAILS_SERVER_HOST and WAILS_SERVER_PORT variables, which the Wails
// server overrides itself; they are read here only to fail fast on bad values.
const (
	EnvServerHost     = "WAILS_SERVER_HOST"
	EnvServerPort     = "WAILS_SERVER_PORT"
	EnvOriginPatterns = "LITRADAR_CLIENT_ORIGIN_PATTERNS"
	EnvHstsMaxAge     = "LITRADAR_CLIENT_HSTS_MAX_AGE"
)

// Default listen address of the web host.
const (
	DefaultHost = "localhost"
	DefaultPort = 8080
)

// Config holds the runtime settings of the web host.
type Config struct {
	// Host is the listen address.
	Host string
	// Port is the listen port.
	Port int
	// OriginPatterns lists extra origins trusted by the Wails host guard, such as
	// the public origin of a reverse proxy that rewrites the Host header.
	OriginPatterns []string
	// HstsMaxAge is the Strict-Transport-Security max-age in seconds; zero omits the header.
	HstsMaxAge int
}

// LoadConfig reads the web host configuration through lookup, which usually wraps
// os.LookupEnv, and rejects malformed values instead of silently ignoring them.
func LoadConfig(lookup func(string) (string, bool)) (Config, error) {
	config := Config{Host: DefaultHost, Port: DefaultPort}
	if value, ok := lookup(EnvServerHost); ok && strings.TrimSpace(value) != "" {
		config.Host = strings.TrimSpace(value)
	}
	if value, ok := lookup(EnvServerPort); ok && strings.TrimSpace(value) != "" {
		port, err := strconv.Atoi(strings.TrimSpace(value))
		if err != nil || port < 1 || port > 65535 {
			return Config{}, fmt.Errorf("%s must be a port between 1 and 65535", EnvServerPort)
		}
		config.Port = port
	}
	if value, ok := lookup(EnvOriginPatterns); ok {
		for pattern := range strings.SplitSeq(value, ",") {
			if pattern = strings.TrimSpace(pattern); pattern != "" {
				config.OriginPatterns = append(config.OriginPatterns, pattern)
			}
		}
	}
	if value, ok := lookup(EnvHstsMaxAge); ok && strings.TrimSpace(value) != "" {
		maxAge, err := strconv.Atoi(strings.TrimSpace(value))
		if err != nil || maxAge < 0 {
			return Config{}, fmt.Errorf("%s must be a non-negative number of seconds", EnvHstsMaxAge)
		}
		config.HstsMaxAge = maxAge
	}
	return config, nil
}
