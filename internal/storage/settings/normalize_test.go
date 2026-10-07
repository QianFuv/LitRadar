package settings

import (
	"errors"
	"strings"
	"testing"
)

func TestRegistryDefaultsAndSecretFlags(t *testing.T) {
	if len(definitions) != 20 {
		t.Fatal("managed setting inventory changed")
	}
	for _, definition := range definitions {
		if normalized, err := Normalize(definition.Field, definition.Default); err != nil || normalized != definition.Default {
			t.Fatalf("%s default is not canonical: %s %v", definition.Field, normalized, err)
		}
	}
}

// TestLogRegexGrammarPreservesAcceptance fixes flags, Unicode, set and capture contracts.
func TestLogRegexGrammarPreservesAcceptance(t *testing.T) {
	cases := []struct {
		pattern string
		isValid bool
	}{
		{"", true}, {"a|", true}, {"a**", true}, {"*a", false},
		{"(?:)", true}, {"(", false}, {")", false}, {"a{2}", false},
		{"(?i:a)", true}, {"(?x)a # comment\n b", true}, {"(?i)*", false},
		{"(?ii:a)", false}, {"(?-:a)", false}, {"(?q:a)", false},
		{"(?P<name>a)(?<other>b)", true}, {"(?P<name>a)(?<name>b)", false},
		{"(?<1name>a)", false}, {"(?<_name.1[]>a)", true},
		{`\x41\u0042\U00000043`, true}, {`\uD800`, false}, {`\U00110000`, false},
		{`\xG0`, false}, {`\q`, false}, {`\!`, true},
		{`\d\D\s\S\w\W`, true}, {`\pL`, true}, {`\pQ`, false},
		{`\b`, false}, {`(?-u:\b)`, true}, {`(?-u:\D)`, false},
		{`(?-u:.)`, false}, {`(?-u:\x80)`, false}, {`(?-u:\u0080)`, true},
		{`[a-z]`, true}, {`[z-a]`, false}, {`[]`, false}, {`[]a]`, true},
		{`[--a]`, true}, {`[a&&b]`, true}, {`[a--b]`, true}, {`[a~~b]`, true},
		{`[\d-a]`, false}, {`[[:digit:]]`, true}, {`[[:unknown:]]`, true},
		{`(?-u:[a-z])`, true}, {`(?-u:[^a])`, false}, {`(?-u:[[:^digit:]])`, false},
		{"[汉字]", true}, {"(?-u:[汉字])", false}, {"\xff", false},
		{strings.Repeat("(", 251) + "a" + strings.Repeat(")", 251), false},
	}
	for _, test := range cases {
		t.Run(test.pattern, func(t *testing.T) {
			if actual := validLogRegex(test.pattern); actual != test.isValid {
				t.Fatalf("valid=%v want=%v", actual, test.isValid)
			}
		})
	}
}

// TestNormalizationPreservesValuesAndErrorCategories fixes canonical settings boundaries.
func TestNormalizationPreservesValuesAndErrorCategories(t *testing.T) {
	cases := []struct {
		field   string
		input   string
		want    string
		isValid bool
	}{
		{"secure_cookies", " YES ", "true", true},
		{"secure_cookies", "", "false", true},
		{"secure_cookies", "no", "false", true},
		{"secure_cookies", "sometimes", "", false},
		{"log_format", "json", "json", true},
		{"log_format", " json ", "", false},
		{"log_filter", "", "", true},
		{"log_filter", "off,[task{value=(?i:a)}]=debug", "off,[task{value=(?i:a)}]=debug", true},
		{"log_filter", "task=6", "", false},
		{"log_filter", "[{value=a{2}}]", "", false},
		{"log_filter", "[{value=a=b}]", "[{value=a=b}]", true},
		{"log_filter", "[{value=true}]", "[{value=true}]", true},
		{"cors_allowed_origins", " HTTPS://Example.org:opaque , https://example.org ", "HTTPS://Example.org:opaque,https://example.org", true},
		{"cors_allowed_origins", "null", "", false},
		{"mcp_allowed_origins", "null,http://[::1]:abc", "null,http://[::1]:abc", true},
		{"mcp_allowed_hosts", " a\tb , a\tb ", "a\tb,a\tb", true},
		{"mcp_allowed_hosts", "bad\x7fhost", "", false},
		{"provider_proxy_url", "https://Proxy.Example:443", "https://proxy.example/", true},
		{"provider_proxy_url", "socks5://Proxy.Example", "socks5://proxy.example:1080", true},
		{"provider_proxy_url", "http://user@proxy.example", "", false},
		{"provider_proxy_url", "https://proxy.example/?", "", false},
		{"ai_allowed_base_urls", "https://Example.org/v1,https://example.org/v1/", "https://example.org/v1/", true},
		{"ai_allowed_base_urls", "https://example.org:0", "", false},
		{"audit_retention_days", " +1 ", "1", true},
		{"audit_retention_days", "0", "", false},
		{"provider_proxy_policy", `{"cnki":false,"cnki":true}`, `{"cnki":true}`, true},
		{"provider_proxy_policy", `{"cnki":"false","cnki":true}`, "", false},
		{"index_provider_routes", `{"catalog":"zjlib_cnki"}`, `{"catalog":"zjlib"}`, true},
	}
	for _, test := range cases {
		t.Run(test.field+test.input, func(t *testing.T) {
			actual, err := Normalize(test.field, test.input)
			if (err == nil) != test.isValid || actual != test.want {
				t.Fatalf("value=%q error=%v want=%q valid=%v", actual, err, test.want, test.isValid)
			}
			if err != nil {
				var typed Error
				if !errors.As(err, &typed) || typed.Kind != InvalidSetting {
					t.Fatalf("lost setting error category: %v", err)
				}
			}
		})
	}
}
