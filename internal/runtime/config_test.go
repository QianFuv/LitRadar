package runtime

import (
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func TestServiceConfigurationUsesPersistedSecurityPolicy(t *testing.T) {
	configuration := defaultServiceConfiguration(t)
	configuration.AreSecureCookiesRequired = true
	if err := configuration.ApplyRuntimeSettings(nil); err == nil {
		t.Fatal("hardened startup accepted insecure cookies")
	}
	values := []settings.Value{{Field: "secure_cookies", Value: "yes"}, {Field: "cors_allowed_origins", Value: "https://one.example, https://two.example"}, {Field: "trusted_proxy_cidrs", Value: "192.0.2.22/24,::1"}, {Field: "mcp_allowed_origins", Value: "null"}, {Field: "future_unknown_setting", Value: "ignored"}}
	if err := configuration.ApplyRuntimeSettings(values); err != nil {
		t.Fatal(err)
	}
	if !configuration.ApiOptions.AreCookiesSecure || configuration.ApiOptions.TrustedProxies[0].String() != "192.0.2.0/24" || len(configuration.ApiOptions.CorsOrigins) != 2 {
		t.Fatal(configuration.ApiOptions)
	}
	for _, value := range []settings.Value{{Field: "cors_allowed_origins", Value: "https://one.example/path"}, {Field: "log_format", Value: "invalid"}, {Field: "delivery_worker_concurrency", Value: "0"}} {
		if err := configuration.ApplyRuntimeSettings([]settings.Value{value}); err == nil {
			t.Fatal("ignored invalid startup setting", value.Field)
		}
	}
}

func TestDevelopmentRequiresExactLoopbackAndNoHardenedFlag(t *testing.T) {
	configuration, err := NewConfig(t.TempDir(), "127.0.0.1", 8000, "secret.key")
	if err != nil {
		t.Fatal(err)
	}
	configuration.IsDevelopment = true
	if err := configuration.ValidateDevelopment(); err != nil {
		t.Fatal(err)
	}
	for _, host := range []string{"0.0.0.0", "localhost", "::1", "[::1]", "127.0.0.2"} {
		configuration.Host = host
		if err := configuration.ValidateDevelopment(); err == nil || !strings.Contains(err.Error(), "Development mode requires") {
			t.Fatal(host, err)
		}
	}
	configuration.Host = "127.0.0.1"
	configuration.AreSecureCookiesRequired = true
	if err := configuration.ValidateDevelopment(); err == nil {
		t.Fatal("development accepted production requirement")
	}
}

// defaultServiceConfiguration checks all original default listener and security policy fields.
func defaultServiceConfiguration(t *testing.T) Config {
	t.Helper()
	configuration, err := NewConfig(t.TempDir(), "127.0.0.1", 8000, "secret.key")
	if err != nil {
		t.Fatal(err)
	}
	if configuration.BindAddress() != "127.0.0.1:8000" || configuration.ApiOptions.AreCookiesSecure || configuration.ApiOptions.RateLimit.LoginIp.Capacity != 30 || !reflect.DeepEqual(configuration.ApiOptions.McpHosts, []string{"localhost", "127.0.0.1", "::1"}) {
		t.Fatal(configuration.ApiOptions)
	}
	return configuration
}
