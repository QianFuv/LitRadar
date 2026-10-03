// Package settings owns the managed runtime registry, validation and secret-safe projections.
package settings

type definition struct {
	Field, Label, Description, Group, Control, ApplyMode, InputType string
	IsSecret                                                        bool
	AllowedValues                                                   []string
	Default, Parser                                                 string
}

var definitions = []definition{
	{Field: "openalex_api_key_pool", Label: "OpenAlex API key pool", Description: "OpenAlex authenticated request key pool.", Group: "source_access", Control: "secret_pool", ApplyMode: "next_command", InputType: "password", IsSecret: true, AllowedValues: []string{}, Default: "", Parser: "SecretPool"},
	{Field: "semantic_scholar_api_key_pool", Label: "Semantic Scholar API key pool", Description: "Comma- or semicolon-separated Semantic Scholar REST API keys.", Group: "source_access", Control: "secret_pool", ApplyMode: "next_command", InputType: "password", IsSecret: true, AllowedValues: []string{}, Default: "", Parser: "SecretPool"},
	{Field: "cnki_captcha_token", Label: "CNKI captcha solver token", Description: "jfbym dual-image token used by domestic CNKI index and abstract captcha solving. Probe override: LITRADAR_CNKI_CAPTCHA_TOKEN.", Group: "source_access", Control: "text", ApplyMode: "next_command", InputType: "password", IsSecret: true, AllowedValues: []string{}, Default: "", Parser: "TrimmedText"},
	{Field: "provider_proxy_url", Label: "Provider proxy URL", Description: "Encrypted HTTP, HTTPS, SOCKS5, or SOCKS5h proxy URL used only by enabled Providers. Changes apply after process restart.", Group: "source_access", Control: "text", ApplyMode: "restart_required", InputType: "password", IsSecret: true, AllowedValues: []string{}, Default: "", Parser: "ProviderProxyUrl"},
	{Field: "crossref_mailto_pool", Label: "Crossref mailto pool", Description: "Comma- or semicolon-separated Crossref contact emails.", Group: "source_access", Control: "string_list", ApplyMode: "next_command", InputType: "email", IsSecret: false, AllowedValues: []string{}, Default: "", Parser: "ValuePool"},
	{Field: "cors_allowed_origins", Label: "CORS allowed origins", Description: "Comma-separated exact HTTP(S) origins for credentialed API requests; paths, wildcard, user-info, query, fragment, and null are rejected. Changes apply after API restart.", Group: "server_security", Control: "string_list", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "", Parser: "ExactOriginList"},
	{Field: "mcp_allowed_hosts", Label: "MCP allowed hosts", Description: "Comma-separated hosts accepted by the Streamable HTTP MCP endpoint.", Group: "server_security", Control: "string_list", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "localhost,127.0.0.1,::1", Parser: "HeaderValueList"},
	{Field: "mcp_allowed_origins", Label: "MCP allowed origins", Description: "Comma-separated exact HTTP(S) origins accepted by the Streamable HTTP MCP endpoint; null is also supported. Changes apply after API restart.", Group: "server_security", Control: "string_list", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "", Parser: "ExactOriginList"},
	{Field: "secure_cookies", Label: "Secure session cookies", Description: "Whether session cookies include the Secure attribute.", Group: "server_security", Control: "boolean", ApplyMode: "restart_required", InputType: "boolean", IsSecret: false, AllowedValues: []string{"true", "false"}, Default: "false", Parser: "Boolean"},
	{Field: "trusted_proxy_cidrs", Label: "Trusted proxy CIDRs", Description: "Comma-separated IPv4 or IPv6 CIDRs whose direct peers may supply Forwarded or X-Forwarded-For client chains. Changes apply after API restart.", Group: "server_security", Control: "string_list", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "", Parser: "TrustedProxyCidrs"},
	{Field: "auth_rate_limit_policy", Label: "Authentication rate-limit policy", Description: "Strict JSON token-bucket policy for client IP, normalized username, and process-wide authentication breakers. Changes apply after API restart.", Group: "server_security", Control: "text", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "{\"login_ip\":{\"capacity\":30,\"refill_tokens\":1,\"refill_seconds\":1},\"username\":{\"capacity\":5,\"refill_tokens\":1,\"refill_seconds\":60},\"register_ip\":{\"capacity\":5,\"refill_tokens\":1,\"refill_seconds\":60},\"global_login\":{\"capacity\":1000,\"refill_tokens\":100,\"refill_seconds\":1},\"global_register\":{\"capacity\":250,\"refill_tokens\":25,\"refill_seconds\":1},\"ip_key_limit\":8192,\"username_key_limit\":4096}", Parser: "AuthRateLimitPolicy"},
	{Field: "audit_retention_days", Label: "Security audit retention days", Description: "Number of days retained in the durable security audit table; the runtime applies changes at the next bounded maintenance check.", Group: "observability", Control: "text", ApplyMode: "next_request", InputType: "number", IsSecret: false, AllowedValues: []string{}, Default: "180", Parser: "AuditRetentionDays"},
	{Field: "delivery_worker_concurrency", Label: "Delivery worker concurrency", Description: "Maximum supervised manual delivery child processes owned by one service instance. Changes apply after service restart.", Group: "server_security", Control: "text", ApplyMode: "restart_required", InputType: "number", IsSecret: false, AllowedValues: []string{}, Default: "2", Parser: "DeliveryWorkerConcurrency"},
	{Field: "ai_allowed_base_urls", Label: "AI allowed base URLs", Description: "Comma-separated exact HTTPS base URLs ordinary users may select for OpenAI-compatible requests. Empty disables AI delivery.", Group: "server_security", Control: "string_list", ApplyMode: "next_request", InputType: "url", IsSecret: false, AllowedValues: []string{}, Default: "", Parser: "HttpsBaseUrlList"},
	{Field: "provider_proxy_policy", Label: "Provider proxy policy", Description: "JSON object that independently enables the managed proxy for each Provider. Missing Providers are disabled. Changes apply after process restart.", Group: "provider_routing", Control: "provider_proxy_policy", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "{\"cnki\":false,\"scholarly\":false,\"zjlib\":false}", Parser: "ProviderProxyPolicy"},
	{Field: "index_provider_routes", Label: "Index provider routes", Description: "JSON object mapping each catalog stem to one registered indexing provider.", Group: "provider_routing", Control: "index_provider_routes", ApplyMode: "next_command", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "{\"ccf_computer_journals\":\"scholarly\",\"chinese_journals\":\"cnki\",\"english_journals\":\"scholarly\"}", Parser: "IndexProviderRoutes"},
	{Field: "article_abstract_provider_orders", Label: "Article abstract provider orders", Description: "JSON default and per-catalog Provider orders for live article abstract-page resolution.", Group: "provider_routing", Control: "provider_order", ApplyMode: "next_request", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "{\"default\":[\"scholarly\",\"cnki\"],\"catalogs\":{}}", Parser: "ProviderOrder"},
	{Field: "article_fulltext_provider_orders", Label: "Article full-text provider orders", Description: "JSON default and per-catalog Provider orders for live article full-text resolution.", Group: "provider_routing", Control: "provider_order", ApplyMode: "next_request", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "{\"default\":[\"zjlib\"],\"catalogs\":{}}", Parser: "ProviderOrder"},
	{Field: "log_format", Label: "Log format", Description: "Structured process log output format. Changes apply after process restart.", Group: "observability", Control: "select", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{"json", "compact"}, Default: "json", Parser: "LogFormat"},
	{Field: "log_filter", Label: "Log filter", Description: "Strict tracing-subscriber EnvFilter directives. Changes apply after process restart.", Group: "observability", Control: "text", ApplyMode: "restart_required", InputType: "text", IsSecret: false, AllowedValues: []string{}, Default: "warn,litradar=info,litradar_api=info,litradar_cli=info,litradar_index=info,litradar_sources=info,litradar_storage=info,litradar_worker=info", Parser: "LogFilter"},
}

func findDefinition(field string) *definition {
	for index := range definitions {
		if definitions[index].Field == field {
			return &definitions[index]
		}
	}
	return nil
}

// Default returns the registry default for a known setting.
func Default(field string) (string, bool) {
	definition := findDefinition(field)
	if definition == nil {
		return "", false
	}
	return definition.Default, true
}
