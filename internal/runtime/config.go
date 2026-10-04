// Package runtime owns service startup, shared resources and coordinated shutdown.
package runtime

import (
	"errors"
	"fmt"
	"net/netip"
	"strings"

	"github.com/QianFuv/LitRadar/internal/api"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

// Config carries explicit launch settings separately from persisted server policy.
type Config struct {
	Storage                                   config.Config
	Host                                      string
	Port                                      uint16
	SecretKeyFile, BundledMetaDir, Executable string
	SchedulerIntervalSeconds                  uint64
	IsDevelopment, AreSecureCookiesRequired   bool
	ApiOptions                                api.Options
}

// NewConfig derives the original service defaults without touching deployment data.
func NewConfig(projectRoot, host string, port uint16, secretKeyFile string) (Config, error) {
	configuration := Config{Storage: config.FromProjectRoot(projectRoot), Host: host, Port: port, SecretKeyFile: secretKeyFile, SchedulerIntervalSeconds: 30}
	values := []settings.Value{}
	for _, field := range []string{"cors_allowed_origins", "mcp_allowed_hosts", "mcp_allowed_origins", "secure_cookies", "trusted_proxy_cidrs", "auth_rate_limit_policy"} {
		value, _ := settings.Default(field)
		values = append(values, settings.Value{Field: field, Value: value})
	}
	err := configuration.ApplyRuntimeSettings(values)
	return configuration, err
}

// ValidateDevelopment rejects public development listeners before any storage preparation.
func (configuration Config) ValidateDevelopment() error {
	if configuration.IsDevelopment && (configuration.Host != "127.0.0.1" || configuration.AreSecureCookiesRequired) {
		return errors.New("Development mode requires --host 127.0.0.1 and cannot use --require-secure-cookies")
	}
	return nil
}

// BindAddress preserves explicit host spelling, including caller-supplied IPv6 brackets.
func (configuration Config) BindAddress() string {
	return fmt.Sprintf("%s:%d", configuration.Host, configuration.Port)
}

// ApplyRuntimeSettings validates every known setting and applies server policy in source order.
func (configuration *Config) ApplyRuntimeSettings(values []settings.Value) error {
	for _, value := range values {
		if _, exists := settings.Default(value.Field); !exists {
			continue
		}
		canonical, err := settings.Normalize(value.Field, value.Value)
		if err != nil {
			return err
		}
		switch value.Field {
		case "cors_allowed_origins":
			configuration.ApiOptions.CorsOrigins = commaValues(canonical)
		case "mcp_allowed_hosts":
			configuration.ApiOptions.McpHosts = commaValues(canonical)
		case "mcp_allowed_origins":
			configuration.ApiOptions.McpOrigins = commaValues(canonical)
		case "secure_cookies":
			configuration.ApiOptions.AreCookiesSecure = canonical == "true"
		case "trusted_proxy_cidrs":
			prefixes := []netip.Prefix{}
			for _, value := range commaValues(canonical) {
				prefix, err := netip.ParsePrefix(value)
				if err != nil {
					return err
				}
				prefixes = append(prefixes, prefix)
			}
			configuration.ApiOptions.TrustedProxies = prefixes
		case "auth_rate_limit_policy":
			policy, err := settings.ParseRateLimitPolicy(canonical)
			if err != nil {
				return err
			}
			configuration.ApiOptions.RateLimit = policy
		}
	}
	if configuration.AreSecureCookiesRequired && !configuration.ApiOptions.AreCookiesSecure {
		return errors.New("Secure session cookies are required; set secure_cookies to true before startup")
	}
	return nil
}

func commaValues(value string) []string {
	if value == "" {
		return []string{}
	}
	return strings.Split(value, ",")
}
