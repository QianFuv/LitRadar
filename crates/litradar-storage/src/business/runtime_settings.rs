//! Managed runtime setting repositories.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use http::{HeaderValue, Uri};
use litradar_domain::{RuntimeSettingApplyMode, RuntimeSettingControl, RuntimeSettingGroup};
use rusqlite::OpenFlags;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;
use url::Url;

use super::shared::*;
use super::*;

/// Stable key for one managed runtime setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeSettingKey {
    /// OpenAlex authenticated request key pool.
    OpenAlexApiKeyPool,
    /// Semantic Scholar authenticated request key pool.
    SemanticScholarApiKeyPool,
    /// Domestic CNKI captcha solver token.
    CnkiCaptchaToken,
    /// Crossref polite contact email pool.
    CrossrefMailtoPool,
    /// Credentialed API CORS origins.
    CorsAllowedOrigins,
    /// Hosts accepted by the MCP endpoint.
    McpAllowedHosts,
    /// Browser origins accepted by the MCP endpoint.
    McpAllowedOrigins,
    /// Secure session-cookie switch.
    SecureCookies,
    /// Reverse-proxy networks trusted to supply client forwarding chains.
    TrustedProxyCidrs,
    /// Process-local authentication limiter policy.
    AuthRateLimitPolicy,
    /// Durable security audit retention period in days.
    AuditRetentionDays,
    /// Maximum manual delivery child processes owned by one service instance.
    DeliveryWorkerConcurrency,
    /// OpenAI-compatible base URLs available to ordinary users.
    AiAllowedBaseUrls,
    /// Catalog-to-index-Provider routes.
    IndexProviderRoutes,
    /// Article abstract Provider orders.
    ArticleAbstractProviderOrders,
    /// Article full-text Provider orders.
    ArticleFullTextProviderOrders,
    /// Process log output format.
    LogFormat,
    /// Process tracing filter directives.
    LogFilter,
}

impl RuntimeSettingKey {
    /// All managed runtime setting keys in administrator display order.
    pub const ALL: [Self; 18] = [
        Self::OpenAlexApiKeyPool,
        Self::SemanticScholarApiKeyPool,
        Self::CnkiCaptchaToken,
        Self::CrossrefMailtoPool,
        Self::CorsAllowedOrigins,
        Self::McpAllowedHosts,
        Self::McpAllowedOrigins,
        Self::SecureCookies,
        Self::TrustedProxyCidrs,
        Self::AuthRateLimitPolicy,
        Self::AuditRetentionDays,
        Self::DeliveryWorkerConcurrency,
        Self::AiAllowedBaseUrls,
        Self::IndexProviderRoutes,
        Self::ArticleAbstractProviderOrders,
        Self::ArticleFullTextProviderOrders,
        Self::LogFormat,
        Self::LogFilter,
    ];

    /// Return the persisted field name for this setting.
    ///
    /// # Returns
    ///
    /// Stable database and API field name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAlexApiKeyPool => "openalex_api_key_pool",
            Self::SemanticScholarApiKeyPool => "semantic_scholar_api_key_pool",
            Self::CnkiCaptchaToken => "cnki_captcha_token",
            Self::CrossrefMailtoPool => "crossref_mailto_pool",
            Self::CorsAllowedOrigins => "cors_allowed_origins",
            Self::McpAllowedHosts => "mcp_allowed_hosts",
            Self::McpAllowedOrigins => "mcp_allowed_origins",
            Self::SecureCookies => "secure_cookies",
            Self::TrustedProxyCidrs => "trusted_proxy_cidrs",
            Self::AuthRateLimitPolicy => "auth_rate_limit_policy",
            Self::AuditRetentionDays => "audit_retention_days",
            Self::DeliveryWorkerConcurrency => "delivery_worker_concurrency",
            Self::AiAllowedBaseUrls => "ai_allowed_base_urls",
            Self::IndexProviderRoutes => "index_provider_routes",
            Self::ArticleAbstractProviderOrders => "article_abstract_provider_orders",
            Self::ArticleFullTextProviderOrders => "article_fulltext_provider_orders",
            Self::LogFormat => "log_format",
            Self::LogFilter => "log_filter",
        }
    }

    /// Resolve a managed setting key from its persisted field name.
    ///
    /// # Arguments
    ///
    /// * `field` - Database or API field name.
    ///
    /// # Returns
    ///
    /// Matching managed key, or `None` for unmanaged fields.
    pub fn from_field(field: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.as_str() == field)
    }
}

/// Typed value produced by the shared runtime-setting parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedRuntimeSettingValue {
    /// Canonical boolean value.
    Boolean(bool),
    /// Ordered canonical string-list value.
    StringList(Vec<String>),
    /// Parsed networks trusted to supply client forwarding chains.
    TrustedProxyCidrs(Vec<TrustedProxyCidr>),
    /// Parsed process-local authentication limiter policy.
    AuthRateLimitPolicy(AuthRateLimitPolicy),
    /// Parsed bounded unsigned integer value.
    UnsignedInteger(u32),
    /// Canonical scalar or structured text value.
    Text(String),
}

impl ParsedRuntimeSettingValue {
    /// Serialize a parsed value for database persistence.
    ///
    /// # Returns
    ///
    /// Canonical textual representation.
    pub fn into_text(self) -> String {
        match self {
            Self::Boolean(value) => value.to_string(),
            Self::StringList(values) => values.join(","),
            Self::TrustedProxyCidrs(values) => values
                .into_iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(","),
            Self::AuthRateLimitPolicy(value) => serde_json::to_string(&value)
                .expect("authentication rate-limit policy serialization should be infallible"),
            Self::UnsignedInteger(value) => value.to_string(),
            Self::Text(value) => value,
        }
    }
}

/// Canonical IPv4 or IPv6 network trusted to supply forwarding headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TrustedProxyCidr {
    network: IpAddr,
    prefix_length: u8,
}

impl TrustedProxyCidr {
    /// Return whether an address belongs to this trusted network.
    ///
    /// # Arguments
    ///
    /// * `address` - Candidate peer address.
    ///
    /// # Returns
    ///
    /// True when the address matches this network and address family.
    pub fn contains(self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                masked_ipv4(address, self.prefix_length) == network
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                masked_ipv6(address, self.prefix_length) == network
            }
            _ => false,
        }
    }
}

impl fmt::Display for TrustedProxyCidr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.network, self.prefix_length)
    }
}

/// Token-bucket capacity and rational refill rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenBucketPolicy {
    /// Maximum whole tokens held by the bucket.
    pub capacity: u32,
    /// Whole tokens restored during each refill period.
    pub refill_tokens: u32,
    /// Refill period length in seconds.
    pub refill_seconds: u64,
}

/// Bounded process-local authentication rate-limit policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRateLimitPolicy {
    /// Per-client-IP login bucket.
    pub login_ip: TokenBucketPolicy,
    /// Per-normalized-username bucket used independently for each auth operation.
    pub username: TokenBucketPolicy,
    /// Per-client-IP registration bucket.
    pub register_ip: TokenBucketPolicy,
    /// High-threshold process-wide login breaker.
    pub global_login: TokenBucketPolicy,
    /// High-threshold process-wide registration breaker.
    pub global_register: TokenBucketPolicy,
    /// Maximum combined login and registration IP keys retained in memory.
    pub ip_key_limit: usize,
    /// Maximum combined login and registration username keys retained in memory.
    pub username_key_limit: usize,
}

impl Default for AuthRateLimitPolicy {
    /// Return the audited process-local limiter defaults.
    fn default() -> Self {
        Self {
            login_ip: TokenBucketPolicy {
                capacity: 30,
                refill_tokens: 1,
                refill_seconds: 1,
            },
            username: TokenBucketPolicy {
                capacity: 5,
                refill_tokens: 1,
                refill_seconds: 60,
            },
            register_ip: TokenBucketPolicy {
                capacity: 5,
                refill_tokens: 1,
                refill_seconds: 60,
            },
            global_login: TokenBucketPolicy {
                capacity: 1_000,
                refill_tokens: 100,
                refill_seconds: 1,
            },
            global_register: TokenBucketPolicy {
                capacity: 250,
                refill_tokens: 25,
                refill_seconds: 1,
            },
            ip_key_limit: 8_192,
            username_key_limit: 4_096,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RuntimeSettingParser {
    SecretPool,
    TrimmedText,
    ValuePool,
    ExactOriginList { is_null_allowed: bool },
    HeaderValueList,
    Boolean,
    TrustedProxyCidrs,
    AuthRateLimitPolicy,
    AuditRetentionDays,
    DeliveryWorkerConcurrency,
    HttpsBaseUrlList,
    IndexProviderRoutes,
    ProviderOrder,
    LogFormat,
    LogFilter,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeConfigDefinition {
    field: &'static str,
    label: &'static str,
    group: RuntimeSettingGroup,
    control: RuntimeSettingControl,
    apply_mode: RuntimeSettingApplyMode,
    allowed_values: &'static [&'static str],
    input_type: &'static str,
    is_secret: bool,
    description: &'static str,
    default_value: &'static str,
    parser: RuntimeSettingParser,
}

/// Default strict tracing filter used before an administrator stores an override.
pub const DEFAULT_RUNTIME_LOG_FILTER: &str = concat!(
    "warn,",
    "litradar=info,",
    "litradar_api=info,",
    "litradar_cli=info,",
    "litradar_index=info,",
    "litradar_sources=info,",
    "litradar_storage=info,",
    "litradar_worker=info"
);

/// Default structured process log format.
pub const DEFAULT_RUNTIME_LOG_FORMAT: &str = "json";

/// Default number of manual delivery child processes per service instance.
pub const DEFAULT_DELIVERY_WORKER_CONCURRENCY: usize = 2;

/// Maximum configurable manual delivery child processes per service instance.
pub const MAX_DELIVERY_WORKER_CONCURRENCY: usize = 16;

/// Canonical default authentication limiter policy JSON.
pub const DEFAULT_AUTH_RATE_LIMIT_POLICY_JSON: &str = concat!(
    "{\"login_ip\":{\"capacity\":30,\"refill_tokens\":1,\"refill_seconds\":1},",
    "\"username\":{\"capacity\":5,\"refill_tokens\":1,\"refill_seconds\":60},",
    "\"register_ip\":{\"capacity\":5,\"refill_tokens\":1,\"refill_seconds\":60},",
    "\"global_login\":{\"capacity\":1000,\"refill_tokens\":100,\"refill_seconds\":1},",
    "\"global_register\":{\"capacity\":250,\"refill_tokens\":25,\"refill_seconds\":1},",
    "\"ip_key_limit\":8192,\"username_key_limit\":4096}"
);

const BOOLEAN_ALLOWED_VALUES: [&str; 2] = ["true", "false"];
const LOG_FORMAT_ALLOWED_VALUES: [&str; 2] = ["json", "compact"];

/// Non-secret logging settings loaded before database migrations or command dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLoggingSettings {
    /// Strict tracing-subscriber filter directives.
    pub log_filter: String,
    /// Structured output format name.
    pub log_format: String,
}

impl Default for RuntimeLoggingSettings {
    /// Return the startup-safe logging defaults.
    fn default() -> Self {
        Self {
            log_filter: DEFAULT_RUNTIME_LOG_FILTER.to_string(),
            log_format: DEFAULT_RUNTIME_LOG_FORMAT.to_string(),
        }
    }
}

const RUNTIME_CONFIG_DEFINITIONS: [RuntimeConfigDefinition; 18] = [
    RuntimeConfigDefinition {
        field: "openalex_api_key_pool",
        label: "OpenAlex API key pool",
        group: RuntimeSettingGroup::SourceAccess,
        control: RuntimeSettingControl::SecretPool,
        apply_mode: RuntimeSettingApplyMode::NextCommand,
        allowed_values: &[],
        input_type: "password",
        is_secret: true,
        description: "OpenAlex authenticated request key pool.",
        default_value: "",
        parser: RuntimeSettingParser::SecretPool,
    },
    RuntimeConfigDefinition {
        field: "semantic_scholar_api_key_pool",
        label: "Semantic Scholar API key pool",
        group: RuntimeSettingGroup::SourceAccess,
        control: RuntimeSettingControl::SecretPool,
        apply_mode: RuntimeSettingApplyMode::NextCommand,
        allowed_values: &[],
        input_type: "password",
        is_secret: true,
        description: "Comma- or semicolon-separated Semantic Scholar REST API keys.",
        default_value: "",
        parser: RuntimeSettingParser::SecretPool,
    },
    RuntimeConfigDefinition {
        field: "cnki_captcha_token",
        label: "CNKI captcha solver token",
        group: RuntimeSettingGroup::SourceAccess,
        control: RuntimeSettingControl::Text,
        apply_mode: RuntimeSettingApplyMode::NextCommand,
        allowed_values: &[],
        input_type: "password",
        is_secret: true,
        description: "jfbym dual-image token used by domestic CNKI index and abstract captcha solving. Probe override: LITRADAR_CNKI_CAPTCHA_TOKEN.",
        default_value: "",
        parser: RuntimeSettingParser::TrimmedText,
    },
    RuntimeConfigDefinition {
        field: "crossref_mailto_pool",
        label: "Crossref mailto pool",
        group: RuntimeSettingGroup::SourceAccess,
        control: RuntimeSettingControl::StringList,
        apply_mode: RuntimeSettingApplyMode::NextCommand,
        allowed_values: &[],
        input_type: "email",
        is_secret: false,
        description: "Comma- or semicolon-separated Crossref contact emails.",
        default_value: "",
        parser: RuntimeSettingParser::ValuePool,
    },
    RuntimeConfigDefinition {
        field: "cors_allowed_origins",
        label: "CORS allowed origins",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::StringList,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "Comma-separated exact HTTP(S) origins for credentialed API requests; paths, wildcard, user-info, query, fragment, and null are rejected. Changes apply after API restart.",
        default_value: "",
        parser: RuntimeSettingParser::ExactOriginList {
            is_null_allowed: false,
        },
    },
    RuntimeConfigDefinition {
        field: "mcp_allowed_hosts",
        label: "MCP allowed hosts",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::StringList,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "Comma-separated hosts accepted by the Streamable HTTP MCP endpoint.",
        default_value: "localhost,127.0.0.1,::1",
        parser: RuntimeSettingParser::HeaderValueList,
    },
    RuntimeConfigDefinition {
        field: "mcp_allowed_origins",
        label: "MCP allowed origins",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::StringList,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "Comma-separated exact HTTP(S) origins accepted by the Streamable HTTP MCP endpoint; null is also supported. Changes apply after API restart.",
        default_value: "",
        parser: RuntimeSettingParser::ExactOriginList {
            is_null_allowed: true,
        },
    },
    RuntimeConfigDefinition {
        field: "secure_cookies",
        label: "Secure session cookies",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::Boolean,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &BOOLEAN_ALLOWED_VALUES,
        input_type: "boolean",
        is_secret: false,
        description: "Whether session cookies include the Secure attribute.",
        default_value: "false",
        parser: RuntimeSettingParser::Boolean,
    },
    RuntimeConfigDefinition {
        field: "trusted_proxy_cidrs",
        label: "Trusted proxy CIDRs",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::StringList,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "Comma-separated IPv4 or IPv6 CIDRs whose direct peers may supply Forwarded or X-Forwarded-For client chains. Changes apply after API restart.",
        default_value: "",
        parser: RuntimeSettingParser::TrustedProxyCidrs,
    },
    RuntimeConfigDefinition {
        field: "auth_rate_limit_policy",
        label: "Authentication rate-limit policy",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::Text,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "Strict JSON token-bucket policy for client IP, normalized username, and process-wide authentication breakers. Changes apply after API restart.",
        default_value: DEFAULT_AUTH_RATE_LIMIT_POLICY_JSON,
        parser: RuntimeSettingParser::AuthRateLimitPolicy,
    },
    RuntimeConfigDefinition {
        field: "audit_retention_days",
        label: "Security audit retention days",
        group: RuntimeSettingGroup::Observability,
        control: RuntimeSettingControl::Text,
        apply_mode: RuntimeSettingApplyMode::NextRequest,
        allowed_values: &[],
        input_type: "number",
        is_secret: false,
        description: "Number of days retained in the durable security audit table; the runtime applies changes at the next bounded maintenance check.",
        default_value: "180",
        parser: RuntimeSettingParser::AuditRetentionDays,
    },
    RuntimeConfigDefinition {
        field: "delivery_worker_concurrency",
        label: "Delivery worker concurrency",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::Text,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &[],
        input_type: "number",
        is_secret: false,
        description: "Maximum supervised manual delivery child processes owned by one service instance. Changes apply after service restart.",
        default_value: "2",
        parser: RuntimeSettingParser::DeliveryWorkerConcurrency,
    },
    RuntimeConfigDefinition {
        field: "ai_allowed_base_urls",
        label: "AI allowed base URLs",
        group: RuntimeSettingGroup::ServerSecurity,
        control: RuntimeSettingControl::StringList,
        apply_mode: RuntimeSettingApplyMode::NextRequest,
        allowed_values: &[],
        input_type: "url",
        is_secret: false,
        description: "Comma-separated exact HTTPS base URLs ordinary users may select for OpenAI-compatible requests. Empty disables AI delivery.",
        default_value: "",
        parser: RuntimeSettingParser::HttpsBaseUrlList,
    },
    RuntimeConfigDefinition {
        field: "index_provider_routes",
        label: "Index provider routes",
        group: RuntimeSettingGroup::ProviderRouting,
        control: RuntimeSettingControl::IndexProviderRoutes,
        apply_mode: RuntimeSettingApplyMode::NextCommand,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "JSON object mapping each catalog stem to one registered indexing provider.",
        default_value: "{\"ccf_computer_journals\":\"scholarly\",\"chinese_journals\":\"cnki\",\"english_journals\":\"scholarly\"}",
        parser: RuntimeSettingParser::IndexProviderRoutes,
    },
    RuntimeConfigDefinition {
        field: "article_abstract_provider_orders",
        label: "Article abstract provider orders",
        group: RuntimeSettingGroup::ProviderRouting,
        control: RuntimeSettingControl::ProviderOrder,
        apply_mode: RuntimeSettingApplyMode::NextRequest,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "JSON default and per-catalog Provider orders for live article abstract-page resolution.",
        default_value: "{\"default\":[\"scholarly\",\"cnki\"],\"catalogs\":{}}",
        parser: RuntimeSettingParser::ProviderOrder,
    },
    RuntimeConfigDefinition {
        field: "article_fulltext_provider_orders",
        label: "Article full-text provider orders",
        group: RuntimeSettingGroup::ProviderRouting,
        control: RuntimeSettingControl::ProviderOrder,
        apply_mode: RuntimeSettingApplyMode::NextRequest,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "JSON default and per-catalog Provider orders for live article full-text resolution.",
        default_value: "{\"default\":[\"zjlib\"],\"catalogs\":{}}",
        parser: RuntimeSettingParser::ProviderOrder,
    },
    RuntimeConfigDefinition {
        field: "log_format",
        label: "Log format",
        group: RuntimeSettingGroup::Observability,
        control: RuntimeSettingControl::Select,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &LOG_FORMAT_ALLOWED_VALUES,
        input_type: "text",
        is_secret: false,
        description: "Structured process log output format. Changes apply after process restart.",
        default_value: DEFAULT_RUNTIME_LOG_FORMAT,
        parser: RuntimeSettingParser::LogFormat,
    },
    RuntimeConfigDefinition {
        field: "log_filter",
        label: "Log filter",
        group: RuntimeSettingGroup::Observability,
        control: RuntimeSettingControl::Text,
        apply_mode: RuntimeSettingApplyMode::RestartRequired,
        allowed_values: &[],
        input_type: "text",
        is_secret: false,
        description: "Strict tracing-subscriber EnvFilter directives. Changes apply after process restart.",
        default_value: DEFAULT_RUNTIME_LOG_FILTER,
        parser: RuntimeSettingParser::LogFilter,
    },
];

/// Return the declared default text for a managed runtime setting.
///
/// # Arguments
///
/// * `key` - Managed setting key.
///
/// # Returns
///
/// Canonical default text owned by the runtime-setting registry.
pub fn runtime_setting_default(key: RuntimeSettingKey) -> &'static str {
    runtime_definition_by_key(key).default_value
}

/// Parse and canonicalize one managed runtime setting.
///
/// # Arguments
///
/// * `key` - Managed setting key.
/// * `value` - Submitted or persisted textual value.
///
/// # Returns
///
/// Typed canonical value, or a safe validation error.
pub fn parse_runtime_setting(
    key: RuntimeSettingKey,
    value: &str,
) -> Result<ParsedRuntimeSettingValue, BusinessRepositoryError> {
    let definition = runtime_definition_by_key(key);
    match definition.parser {
        RuntimeSettingParser::SecretPool | RuntimeSettingParser::ValuePool => Ok(
            ParsedRuntimeSettingValue::StringList(runtime_pool_from_text(value)),
        ),
        RuntimeSettingParser::TrimmedText => {
            Ok(ParsedRuntimeSettingValue::Text(value.trim().to_string()))
        }
        RuntimeSettingParser::ExactOriginList { is_null_allowed } => {
            parse_exact_origin_list(key, value, is_null_allowed)
                .map(ParsedRuntimeSettingValue::StringList)
        }
        RuntimeSettingParser::HeaderValueList => {
            parse_header_value_list(key, value).map(ParsedRuntimeSettingValue::StringList)
        }
        RuntimeSettingParser::Boolean => {
            parse_runtime_bool(key, value).map(ParsedRuntimeSettingValue::Boolean)
        }
        RuntimeSettingParser::TrustedProxyCidrs => {
            parse_trusted_proxy_cidrs(value).map(ParsedRuntimeSettingValue::TrustedProxyCidrs)
        }
        RuntimeSettingParser::AuthRateLimitPolicy => {
            parse_auth_rate_limit_policy(value).map(ParsedRuntimeSettingValue::AuthRateLimitPolicy)
        }
        RuntimeSettingParser::AuditRetentionDays => value
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|days| {
                (MIN_AUDIT_RETENTION_DAYS..=MAX_AUDIT_RETENTION_DAYS).contains(days)
            })
            .map(ParsedRuntimeSettingValue::UnsignedInteger)
            .ok_or_else(|| {
                BusinessRepositoryError::InvalidRuntimeSetting(format!(
                    "Security audit retention days must be between {MIN_AUDIT_RETENTION_DAYS} and {MAX_AUDIT_RETENTION_DAYS}"
                ))
            }),
        RuntimeSettingParser::DeliveryWorkerConcurrency => value
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|workers| {
                (1..=u32::try_from(MAX_DELIVERY_WORKER_CONCURRENCY)
                    .expect("delivery worker maximum should fit u32"))
                    .contains(workers)
            })
            .map(ParsedRuntimeSettingValue::UnsignedInteger)
            .ok_or_else(|| {
                BusinessRepositoryError::InvalidRuntimeSetting(format!(
                    "Delivery worker concurrency must be between 1 and {MAX_DELIVERY_WORKER_CONCURRENCY}"
                ))
            }),
        RuntimeSettingParser::HttpsBaseUrlList => {
            parse_https_base_url_list(value).map(ParsedRuntimeSettingValue::StringList)
        }
        RuntimeSettingParser::IndexProviderRoutes => {
            normalize_index_provider_routes(value).map(ParsedRuntimeSettingValue::Text)
        }
        RuntimeSettingParser::ProviderOrder => {
            normalize_provider_order_configuration(definition.field, value)
                .map(ParsedRuntimeSettingValue::Text)
        }
        RuntimeSettingParser::LogFormat if LOG_FORMAT_ALLOWED_VALUES.contains(&value) => {
            Ok(ParsedRuntimeSettingValue::Text(value.to_string()))
        }
        RuntimeSettingParser::LogFormat => Err(BusinessRepositoryError::InvalidRuntimeSetting(
            "Invalid LitRadar log format".to_string(),
        )),
        RuntimeSettingParser::LogFilter => EnvFilter::try_new(value)
            .map(|_| ParsedRuntimeSettingValue::Text(value.to_string()))
            .map_err(|_| {
                BusinessRepositoryError::InvalidRuntimeSetting(
                    "Invalid LitRadar log filter".to_string(),
                )
            }),
    }
}

/// Parse and canonicalize one HTTPS outbound base URL.
///
/// # Arguments
///
/// * `value` - Candidate base URL.
///
/// # Returns
///
/// Canonical URL with a trailing path separator, or a safe validation error.
pub fn canonicalize_outbound_base_url(value: &str) -> Result<String, BusinessRepositoryError> {
    let value = value.trim();
    let url = Url::parse(value).map_err(|_| invalid_ai_base_url())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || outbound_authority_has_userinfo(value)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port() == Some(0)
        || url.query().is_some()
        || url.fragment().is_some()
        || url.cannot_be_a_base()
    {
        return Err(invalid_ai_base_url());
    }
    let canonical = url.to_string();
    if url.path().ends_with('/') {
        Ok(canonical)
    } else {
        Ok(format!("{canonical}/"))
    }
}

fn outbound_authority_has_userinfo(value: &str) -> bool {
    value
        .split_once("://")
        .map(|(_, remainder)| {
            remainder
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
        .unwrap_or(false)
}

fn parse_https_base_url_list(value: &str) -> Result<Vec<String>, BusinessRepositoryError> {
    let mut endpoints = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let endpoint = canonicalize_outbound_base_url(entry)?;
        if seen.insert(endpoint.clone()) {
            endpoints.push(endpoint);
        }
    }
    Ok(endpoints)
}

fn parse_trusted_proxy_cidrs(
    value: &str,
) -> Result<Vec<TrustedProxyCidr>, BusinessRepositoryError> {
    let mut networks = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let network = parse_trusted_proxy_cidr(entry)?;
        if seen.insert(network) {
            networks.push(network);
        }
    }
    Ok(networks)
}

fn parse_trusted_proxy_cidr(value: &str) -> Result<TrustedProxyCidr, BusinessRepositoryError> {
    let (address_text, prefix_text) = match value.split_once('/') {
        Some(parts) => (parts.0, Some(parts.1)),
        None => (value, None),
    };
    let address = address_text
        .parse::<IpAddr>()
        .map_err(|_| invalid_trusted_proxy_cidrs())?;
    let maximum_prefix = if address.is_ipv4() { 32 } else { 128 };
    let prefix_length = prefix_text
        .map(|prefix| prefix.parse::<u8>())
        .transpose()
        .map_err(|_| invalid_trusted_proxy_cidrs())?
        .unwrap_or(maximum_prefix);
    if prefix_length > maximum_prefix {
        return Err(invalid_trusted_proxy_cidrs());
    }
    let network = match address {
        IpAddr::V4(address) => IpAddr::V4(masked_ipv4(address, prefix_length)),
        IpAddr::V6(address) => IpAddr::V6(masked_ipv6(address, prefix_length)),
    };
    Ok(TrustedProxyCidr {
        network,
        prefix_length,
    })
}

fn masked_ipv4(address: Ipv4Addr, prefix_length: u8) -> Ipv4Addr {
    let mask = if prefix_length == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_length)
    };
    Ipv4Addr::from(u32::from(address) & mask)
}

fn masked_ipv6(address: Ipv6Addr, prefix_length: u8) -> Ipv6Addr {
    let mask = if prefix_length == 0 {
        0
    } else {
        u128::MAX << (128 - prefix_length)
    };
    Ipv6Addr::from(u128::from(address) & mask)
}

fn invalid_trusted_proxy_cidrs() -> BusinessRepositoryError {
    BusinessRepositoryError::InvalidRuntimeSetting(
        "Trusted proxy CIDRs must contain only valid IPv4 or IPv6 networks".to_string(),
    )
}

fn parse_auth_rate_limit_policy(
    value: &str,
) -> Result<AuthRateLimitPolicy, BusinessRepositoryError> {
    let policy = serde_json::from_str::<AuthRateLimitPolicy>(value)
        .map_err(|_| invalid_auth_rate_limit_policy())?;
    for bucket in [
        policy.login_ip,
        policy.username,
        policy.register_ip,
        policy.global_login,
        policy.global_register,
    ] {
        if bucket.capacity == 0
            || bucket.capacity > 100_000
            || bucket.refill_tokens == 0
            || bucket.refill_tokens > 100_000
            || bucket.refill_tokens > bucket.capacity
            || bucket.refill_seconds == 0
            || bucket.refill_seconds > 86_400
        {
            return Err(invalid_auth_rate_limit_policy());
        }
    }
    if !(1..=65_536).contains(&policy.ip_key_limit)
        || !(1..=65_536).contains(&policy.username_key_limit)
        || !global_breaker_dominates(policy.global_login, policy.login_ip)
        || !global_breaker_dominates(policy.global_login, policy.username)
        || !global_breaker_dominates(policy.global_register, policy.register_ip)
        || !global_breaker_dominates(policy.global_register, policy.username)
    {
        return Err(invalid_auth_rate_limit_policy());
    }
    Ok(policy)
}

fn global_breaker_dominates(global: TokenBucketPolicy, front: TokenBucketPolicy) -> bool {
    global.capacity > front.capacity
        && u128::from(global.refill_tokens) * u128::from(front.refill_seconds)
            >= u128::from(front.refill_tokens) * u128::from(global.refill_seconds)
}

fn invalid_auth_rate_limit_policy() -> BusinessRepositoryError {
    BusinessRepositoryError::InvalidRuntimeSetting(
        "Authentication rate-limit policy must be strict bounded JSON".to_string(),
    )
}

fn invalid_ai_base_url() -> BusinessRepositoryError {
    BusinessRepositoryError::InvalidRuntimeSetting(
        "AI allowed base URLs must be exact HTTPS base URLs without credentials, query, fragment, or port zero"
            .to_string(),
    )
}

fn parse_exact_origin_list(
    key: RuntimeSettingKey,
    value: &str,
    is_null_allowed: bool,
) -> Result<Vec<String>, BusinessRepositoryError> {
    let origins = parse_header_value_list(key, value)?;
    for origin in &origins {
        if is_null_allowed && origin == "null" {
            continue;
        }
        if !is_exact_http_origin(origin) {
            return Err(invalid_runtime_list_entry(key, origin));
        }
    }
    Ok(origins)
}

fn parse_header_value_list(
    key: RuntimeSettingKey,
    value: &str,
) -> Result<Vec<String>, BusinessRepositoryError> {
    let mut values = Vec::new();
    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        HeaderValue::from_str(entry).map_err(|_| invalid_runtime_list_entry(key, entry))?;
        values.push(entry.to_string());
    }
    Ok(values)
}

fn invalid_runtime_list_entry(key: RuntimeSettingKey, value: &str) -> BusinessRepositoryError {
    let label = match key {
        RuntimeSettingKey::CorsAllowedOrigins => "CORS origin",
        RuntimeSettingKey::McpAllowedHosts => "MCP allowed host",
        RuntimeSettingKey::McpAllowedOrigins => "MCP allowed origin",
        RuntimeSettingKey::AiAllowedBaseUrls => "AI allowed base URL",
        _ => "runtime setting list entry",
    };
    BusinessRepositoryError::InvalidRuntimeSetting(format!("Invalid {label}: {value}"))
}

fn is_exact_http_origin(origin: &str) -> bool {
    let Some((scheme, authority_text)) = origin.split_once("://") else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || authority_text.is_empty()
        || authority_text.contains(['/', '?', '#', '@'])
    {
        return false;
    }
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    uri.scheme_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(scheme))
        && uri.authority().is_some()
        && uri.host().is_some_and(|host| !host.is_empty())
        && uri.path() == "/"
        && uri.query().is_none()
}

fn parse_runtime_bool(
    key: RuntimeSettingKey,
    value: &str,
) -> Result<bool, BusinessRepositoryError> {
    let definition = runtime_definition_by_key(key);
    let default = definition.default_value.trim().eq_ignore_ascii_case("true");
    match value.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(BusinessRepositoryError::InvalidRuntimeSetting(format!(
            "Invalid boolean runtime setting {}: {value}",
            key.as_str()
        ))),
    }
}

/// List managed runtime settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Runtime setting payloads.
pub fn list_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let rows = read_runtime_setting_rows(&connection)?;
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .map(|definition| {
            public_runtime_setting_from_definition(definition, rows.get(definition.field), codec)
        })
        .collect()
}

/// Load non-secret logging settings without creating or migrating the auth database.
///
/// # Arguments
///
/// * `auth_db_path` - Selected auth database path.
///
/// # Returns
///
/// Stored logging values, or startup-safe defaults when the database or table does not exist.
pub fn load_runtime_logging_settings(
    auth_db_path: impl AsRef<Path>,
) -> Result<RuntimeLoggingSettings, BusinessRepositoryError> {
    let auth_db_path = auth_db_path.as_ref();
    match fs::metadata(auth_db_path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RuntimeLoggingSettings::default());
        }
        Err(error) => return Err(error.into()),
    }

    let connection = Connection::open_with_flags(auth_db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_runtime_settings = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'runtime_settings')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_runtime_settings {
        return Ok(RuntimeLoggingSettings::default());
    }

    let mut settings = RuntimeLoggingSettings::default();
    let mut statement = connection.prepare(
        "SELECT key, value FROM runtime_settings WHERE key IN ('log_filter', 'log_format')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (field, value) = row?;
        match field.as_str() {
            "log_filter" => settings.log_filter = value,
            "log_format" => settings.log_format = value,
            _ => {}
        }
    }
    Ok(settings)
}

/// Load the administrator-approved AI endpoint catalog.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
///
/// # Returns
///
/// Canonical exact HTTPS base URLs in administrator order.
pub fn load_ai_allowed_base_urls(
    auth_db_path: impl AsRef<Path>,
) -> Result<Vec<String>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    ai_allowed_base_urls_from_connection(&connection)
}

/// Load the effective durable security audit retention period.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
///
/// # Returns
///
/// Validated retention period in days.
pub fn load_audit_retention_days(
    auth_db_path: impl AsRef<Path>,
) -> Result<u32, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let stored = connection
        .query_row(
            "SELECT value FROM runtime_settings WHERE key = ?1",
            [RuntimeSettingKey::AuditRetentionDays.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let value = stored
        .as_deref()
        .unwrap_or_else(|| runtime_setting_default(RuntimeSettingKey::AuditRetentionDays));
    match parse_runtime_setting(RuntimeSettingKey::AuditRetentionDays, value)? {
        ParsedRuntimeSettingValue::UnsignedInteger(days) => Ok(days),
        _ => unreachable!("audit retention parser must return an unsigned integer"),
    }
}

/// Load the restart-scoped manual delivery worker concurrency.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
///
/// # Returns
///
/// Validated per-instance child-process limit.
pub fn load_delivery_worker_concurrency(
    auth_db_path: impl AsRef<Path>,
) -> Result<usize, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let stored = connection
        .query_row(
            "SELECT value FROM runtime_settings WHERE key = ?1",
            [RuntimeSettingKey::DeliveryWorkerConcurrency.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let value = stored
        .as_deref()
        .unwrap_or_else(|| runtime_setting_default(RuntimeSettingKey::DeliveryWorkerConcurrency));
    match parse_runtime_setting(RuntimeSettingKey::DeliveryWorkerConcurrency, value)? {
        ParsedRuntimeSettingValue::UnsignedInteger(workers) => {
            usize::try_from(workers).map_err(|_| {
                invalid_runtime_setting("delivery_worker_concurrency", "value is too large")
            })
        }
        _ => unreachable!("delivery worker parser must return an unsigned integer"),
    }
}

/// Load approved AI endpoints through an existing business transaction.
pub(super) fn ai_allowed_base_urls_from_connection(
    connection: &Connection,
) -> Result<Vec<String>, BusinessRepositoryError> {
    let stored = connection
        .query_row(
            "SELECT value FROM runtime_settings WHERE key = ?1",
            [RuntimeSettingKey::AiAllowedBaseUrls.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let value = stored
        .as_deref()
        .unwrap_or_else(|| runtime_setting_default(RuntimeSettingKey::AiAllowedBaseUrls));
    match parse_runtime_setting(RuntimeSettingKey::AiAllowedBaseUrls, value)? {
        ParsedRuntimeSettingValue::StringList(endpoints) => Ok(endpoints),
        _ => unreachable!("AI endpoint registry parser must return a string list"),
    }
}

/// Load managed runtime settings for trusted backend consumers.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
///
/// # Returns
///
/// Effective values with secret fields decrypted in non-serializable types.
pub fn load_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSettingValue>, BusinessRepositoryError> {
    let connection = open_business_connection(auth_db_path)?;
    let rows = read_runtime_setting_rows(&connection)?;
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .map(|definition| {
            internal_runtime_setting_from_definition(definition, rows.get(definition.field), codec)
        })
        .collect()
}

/// Upsert managed runtime settings.
///
/// # Arguments
///
/// * `auth_db_path` - Path to `auth.sqlite`.
/// * `codec` - Deployment secret codec.
/// * `values` - Values keyed by API field name; null clears secret fields.
/// * `secret_pool_updates` - Incremental secret-pool mutations keyed by API field name.
///
/// # Returns
///
/// Updated runtime setting payloads.
pub fn upsert_runtime_settings(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    values: &HashMap<String, Option<String>>,
    secret_pool_updates: &HashMap<String, RuntimeSecretPoolUpdate>,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    upsert_runtime_settings_with_audit(auth_db_path, codec, values, secret_pool_updates, None)
}

/// Upsert managed runtime settings and persist a required audit event atomically.
pub fn upsert_runtime_settings_with_audit(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
    values: &HashMap<String, Option<String>>,
    secret_pool_updates: &HashMap<String, RuntimeSecretPoolUpdate>,
    audit: Option<&SecurityAuditEvent>,
) -> Result<Vec<RuntimeSettingInfo>, BusinessRepositoryError> {
    let mut connection = open_business_connection(auth_db_path.as_ref())?;
    let now = now_seconds();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = read_runtime_setting_rows(&transaction)?;
    let fields = values
        .keys()
        .chain(secret_pool_updates.keys())
        .cloned()
        .collect::<HashSet<_>>();
    {
        let mut statement = transaction.prepare(
            "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )?;
        for field in fields {
            let definition = runtime_definition_by_field(&field)
                .ok_or_else(|| BusinessRepositoryError::UnknownRuntimeSetting(field.clone()))?;
            let current =
                internal_runtime_setting_from_definition(definition, existing.get(&field), codec)?
                    .value;
            let mut value = if let Some(update) = values.get(&field) {
                if definition.is_secret {
                    match update {
                        None => String::new(),
                        Some(raw_value) if raw_value.trim().is_empty() => current,
                        Some(raw_value) => raw_value.trim().to_string(),
                    }
                } else {
                    update
                        .as_deref()
                        .ok_or_else(|| {
                            BusinessRepositoryError::NonSecretRuntimeSettingCannotBeCleared(
                                field.clone(),
                            )
                        })?
                        .to_string()
                }
            } else {
                current
            };
            if let Some(pool_update) = secret_pool_updates.get(&field) {
                value = apply_secret_pool_update(definition, &value, pool_update, codec)?;
            }
            if !definition.is_secret {
                value = normalize_runtime_setting_value(definition, &value)?;
            }
            let stored_value = if definition.is_secret {
                codec.encrypt(&value, &runtime_context(&field))?
            } else {
                value
            };
            statement.execute(params![definition.field, stored_value, now])?;
        }
    }
    if let Some(audit) = audit {
        insert_required_security_audit_event(&transaction, audit)?;
    }
    transaction.commit()?;
    list_runtime_settings(auth_db_path, codec)
}
fn read_runtime_setting_rows(
    connection: &Connection,
) -> Result<HashMap<String, (String, f64)>, BusinessRepositoryError> {
    let mut statement =
        connection.prepare("SELECT key, value, updated_at FROM runtime_settings")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    Ok(collect_rows(rows)?
        .into_iter()
        .map(|(key, value, updated_at)| (key, (value, updated_at)))
        .collect())
}

fn public_runtime_setting_from_definition(
    definition: &RuntimeConfigDefinition,
    row: Option<&(String, f64)>,
    codec: &SecretCodec,
) -> Result<RuntimeSettingInfo, BusinessRepositoryError> {
    let internal = internal_runtime_setting_from_definition(definition, row, codec)?;
    let secret_items = runtime_secret_items(definition, &internal.value, codec)?;
    let has_value = if is_secret_pool(definition) {
        !secret_items.is_empty()
    } else {
        !internal.value.trim().is_empty()
    };
    Ok(RuntimeSettingInfo {
        field: definition.field.to_string(),
        label: definition.label.to_string(),
        description: definition.description.to_string(),
        group: definition.group,
        control: definition.control,
        apply_mode: definition.apply_mode,
        allowed_values: definition
            .allowed_values
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        input_type: definition.input_type.to_string(),
        is_secret: definition.is_secret,
        value: if definition.is_secret {
            String::new()
        } else {
            internal.value
        },
        has_value,
        masked_value: if definition.is_secret && has_value {
            "••••".to_string()
        } else {
            String::new()
        },
        secret_items,
        source: internal.source,
        updated_at: internal.updated_at,
    })
}

fn apply_secret_pool_update(
    definition: &RuntimeConfigDefinition,
    value: &str,
    update: &RuntimeSecretPoolUpdate,
    codec: &SecretCodec,
) -> Result<String, BusinessRepositoryError> {
    if !is_secret_pool(definition) {
        return Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(
            definition.field.to_string(),
        ));
    }
    let mut pool = runtime_pool_from_text(value);
    let mut removals = HashSet::new();
    for reference in &update.remove {
        let item = codec
            .decrypt(reference, &runtime_secret_item_context(definition.field))
            .map_err(|_| {
                BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(
                    definition.field.to_string(),
                )
            })?;
        if item.is_empty() || !pool.iter().any(|candidate| candidate == &item) {
            return Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(
                definition.field.to_string(),
            ));
        }
        removals.insert(item);
    }
    pool.retain(|item| !removals.contains(item));
    for addition in &update.add {
        for item in runtime_pool_from_text(addition) {
            if !pool.iter().any(|candidate| candidate == &item) {
                pool.push(item);
            }
        }
    }
    Ok(pool.join("\n"))
}

fn runtime_secret_items(
    definition: &RuntimeConfigDefinition,
    value: &str,
    codec: &SecretCodec,
) -> Result<Vec<RuntimeSecretItemInfo>, BusinessRepositoryError> {
    if !is_secret_pool(definition) {
        return Ok(Vec::new());
    }
    runtime_pool_from_text(value)
        .into_iter()
        .map(|item| {
            Ok(RuntimeSecretItemInfo {
                reference: codec.encrypt(&item, &runtime_secret_item_context(definition.field))?,
                masked_value: mask_runtime_secret_item(&item),
            })
        })
        .collect()
}

fn runtime_pool_from_text(value: &str) -> Vec<String> {
    let mut pool = Vec::new();
    for part in value.split([',', ';', '\n']) {
        let item = part.trim();
        if !item.is_empty() && !pool.iter().any(|candidate| candidate == item) {
            pool.push(item.to_string());
        }
    }
    pool
}

fn mask_runtime_secret_item(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() <= 5 {
        return "*".repeat(characters.len());
    }
    format!(
        "{}{}",
        characters.iter().take(5).collect::<String>(),
        "*".repeat(characters.len() - 5)
    )
}

fn is_secret_pool(definition: &RuntimeConfigDefinition) -> bool {
    definition.is_secret && definition.field.ends_with("_pool")
}

fn runtime_secret_item_context(field: &str) -> String {
    format!("{}:pool-item-reference", runtime_context(field))
}

fn internal_runtime_setting_from_definition(
    definition: &RuntimeConfigDefinition,
    row: Option<&(String, f64)>,
    codec: &SecretCodec,
) -> Result<RuntimeSettingValue, BusinessRepositoryError> {
    let (stored, source, updated_at) = if let Some((value, updated_at)) = row {
        (value.as_str(), "database".to_string(), Some(*updated_at))
    } else {
        (definition.default_value, "default".to_string(), None)
    };
    let mut value = if definition.is_secret && row.is_some() {
        codec.decrypt(stored, &runtime_context(definition.field))?
    } else {
        stored.to_string()
    };
    if !definition.is_secret {
        value = normalize_runtime_setting_value(definition, &value)?;
    }
    Ok(RuntimeSettingValue {
        field: definition.field.to_string(),
        value,
        source,
        updated_at,
    })
}

fn runtime_definition_by_field(field: &str) -> Option<&'static RuntimeConfigDefinition> {
    RUNTIME_CONFIG_DEFINITIONS
        .iter()
        .find(|definition| definition.field == field)
}

fn runtime_definition_by_key(key: RuntimeSettingKey) -> &'static RuntimeConfigDefinition {
    runtime_definition_by_field(key.as_str())
        .expect("every RuntimeSettingKey must have one registry definition")
}

fn normalize_runtime_setting_value(
    definition: &RuntimeConfigDefinition,
    value: &str,
) -> Result<String, BusinessRepositoryError> {
    let key = RuntimeSettingKey::from_field(definition.field)
        .expect("every registry definition must have one RuntimeSettingKey");
    parse_runtime_setting(key, value).map(ParsedRuntimeSettingValue::into_text)
}

fn normalize_index_provider_routes(value: &str) -> Result<String, BusinessRepositoryError> {
    let routes = serde_json::from_str::<BTreeMap<String, String>>(value).map_err(|_| {
        invalid_runtime_setting(
            "index_provider_routes",
            "value must be a JSON object of catalog and Provider names",
        )
    })?;
    if routes.is_empty() {
        return Err(invalid_runtime_setting(
            "index_provider_routes",
            "at least one catalog route is required",
        ));
    }
    let mut normalized = BTreeMap::new();
    for (catalog, provider) in routes {
        if !is_runtime_name(&catalog) {
            return Err(invalid_runtime_setting(
                "index_provider_routes",
                "catalog stems must use lowercase ASCII names",
            ));
        }
        let provider = rewrite_legacy_provider_runtime_name(&provider);
        if !is_runtime_name(&provider) {
            return Err(invalid_runtime_setting(
                "index_provider_routes",
                "provider names must use lowercase ASCII names",
            ));
        }
        normalized.insert(catalog, provider);
    }
    Ok(serde_json::to_string(&normalized)?)
}

fn normalize_provider_order_configuration(
    field: &str,
    value: &str,
) -> Result<String, BusinessRepositoryError> {
    let mut configuration =
        serde_json::from_str::<ProviderOrderConfiguration>(value).map_err(|_| {
            invalid_runtime_setting(
                field,
                "value must contain only JSON default and catalogs fields",
            )
        })?;
    configuration.default = configuration
        .default
        .into_iter()
        .map(|name| rewrite_legacy_provider_runtime_name(&name))
        .collect();
    configuration.catalogs = configuration
        .catalogs
        .into_iter()
        .map(|(catalog, providers)| {
            (
                catalog,
                providers
                    .into_iter()
                    .map(|name| rewrite_legacy_provider_runtime_name(&name))
                    .collect(),
            )
        })
        .collect();
    validate_provider_order(field, &configuration.default)?;
    for (catalog, providers) in &configuration.catalogs {
        if !is_runtime_name(catalog) {
            return Err(invalid_runtime_setting(
                field,
                "catalog stems must use lowercase ASCII names",
            ));
        }
        validate_provider_order(field, providers)?;
    }
    Ok(serde_json::to_string(&configuration)?)
}

fn rewrite_legacy_provider_runtime_name(name: &str) -> String {
    // Auth migration v8 rewrote stored `cnki` -> `cnki_oversea` once. After domestic
    // registration, bare `cnki` is the NZKPT product name and must not be rewritten.
    match name {
        "zjlib_cnki" => "zjlib".to_string(),
        other => other.to_string(),
    }
}

fn validate_provider_order(
    field: &str,
    providers: &[String],
) -> Result<(), BusinessRepositoryError> {
    let mut seen = BTreeSet::new();
    for provider in providers {
        if !is_runtime_name(provider) {
            return Err(invalid_runtime_setting(
                field,
                "Provider orders must contain lowercase ASCII names",
            ));
        }
        if !seen.insert(provider) {
            return Err(invalid_runtime_setting(
                field,
                "Provider orders must not contain duplicates",
            ));
        }
    }
    Ok(())
}

fn is_runtime_name(value: &str) -> bool {
    (2..=128).contains(&value.len())
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' | b'0'..=b'9' => true,
            b'.' | b'_' | b'-' => index > 0,
            _ => false,
        })
}

fn invalid_runtime_setting(field: &str, detail: &str) -> BusinessRepositoryError {
    BusinessRepositoryError::InvalidRuntimeSetting(format!("Invalid {field}: {detail}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;
    use crate::{migrate_auth_database, SecretCodec};

    #[test]
    fn runtime_settings_ignore_stale_env_keys_and_proxy_pool() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let connection = Connection::open(&auth_db_path).expect("auth database should open");
        connection
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                ("OPENALEX_API_KEY_POOL", "env-key", 1.0_f64),
            )
            .expect("stale env-key row should insert");
        connection
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                ("PROXY_POOL", "proxy", 1.0_f64),
            )
            .expect("stale proxy row should insert");

        let codec = SecretCodec::from_key([8_u8; 32]);
        let settings =
            list_runtime_settings(&auth_db_path, &codec).expect("runtime settings should load");
        let fields = settings
            .iter()
            .map(|setting| setting.field.as_str())
            .collect::<Vec<_>>();

        assert_eq!(settings.len(), 18);
        assert!(fields.contains(&"openalex_api_key_pool"));
        assert!(fields.contains(&"cnki_captcha_token"));
        assert!(fields.contains(&"secure_cookies"));
        assert!(fields.contains(&"trusted_proxy_cidrs"));
        assert!(fields.contains(&"auth_rate_limit_policy"));
        assert!(fields.contains(&"audit_retention_days"));
        assert!(fields.contains(&"delivery_worker_concurrency"));
        assert!(!fields.contains(&"proxy_pool"));
        assert!(settings
            .iter()
            .all(|setting| setting.source == "database" || setting.source == "default"));
        let openalex = settings
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert_eq!(openalex.value, "");
        assert_eq!(openalex.source, "default");
    }

    #[test]
    fn runtime_setting_descriptors_declare_controls_groups_and_apply_modes() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([17_u8; 32]);
        let settings =
            list_runtime_settings(&auth_db_path, &codec).expect("runtime settings should load");
        let expected = [
            (
                "openalex_api_key_pool",
                RuntimeSettingGroup::SourceAccess,
                RuntimeSettingControl::SecretPool,
                RuntimeSettingApplyMode::NextCommand,
                &[][..],
            ),
            (
                "semantic_scholar_api_key_pool",
                RuntimeSettingGroup::SourceAccess,
                RuntimeSettingControl::SecretPool,
                RuntimeSettingApplyMode::NextCommand,
                &[][..],
            ),
            (
                "cnki_captcha_token",
                RuntimeSettingGroup::SourceAccess,
                RuntimeSettingControl::Text,
                RuntimeSettingApplyMode::NextCommand,
                &[][..],
            ),
            (
                "crossref_mailto_pool",
                RuntimeSettingGroup::SourceAccess,
                RuntimeSettingControl::StringList,
                RuntimeSettingApplyMode::NextCommand,
                &[][..],
            ),
            (
                "cors_allowed_origins",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::StringList,
                RuntimeSettingApplyMode::RestartRequired,
                &[][..],
            ),
            (
                "mcp_allowed_hosts",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::StringList,
                RuntimeSettingApplyMode::RestartRequired,
                &[][..],
            ),
            (
                "mcp_allowed_origins",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::StringList,
                RuntimeSettingApplyMode::RestartRequired,
                &[][..],
            ),
            (
                "secure_cookies",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::Boolean,
                RuntimeSettingApplyMode::RestartRequired,
                &["true", "false"][..],
            ),
            (
                "trusted_proxy_cidrs",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::StringList,
                RuntimeSettingApplyMode::RestartRequired,
                &[][..],
            ),
            (
                "auth_rate_limit_policy",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::Text,
                RuntimeSettingApplyMode::RestartRequired,
                &[][..],
            ),
            (
                "audit_retention_days",
                RuntimeSettingGroup::Observability,
                RuntimeSettingControl::Text,
                RuntimeSettingApplyMode::NextRequest,
                &[][..],
            ),
            (
                "delivery_worker_concurrency",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::Text,
                RuntimeSettingApplyMode::RestartRequired,
                &[][..],
            ),
            (
                "ai_allowed_base_urls",
                RuntimeSettingGroup::ServerSecurity,
                RuntimeSettingControl::StringList,
                RuntimeSettingApplyMode::NextRequest,
                &[][..],
            ),
            (
                "index_provider_routes",
                RuntimeSettingGroup::ProviderRouting,
                RuntimeSettingControl::IndexProviderRoutes,
                RuntimeSettingApplyMode::NextCommand,
                &[][..],
            ),
            (
                "article_abstract_provider_orders",
                RuntimeSettingGroup::ProviderRouting,
                RuntimeSettingControl::ProviderOrder,
                RuntimeSettingApplyMode::NextRequest,
                &[][..],
            ),
            (
                "article_fulltext_provider_orders",
                RuntimeSettingGroup::ProviderRouting,
                RuntimeSettingControl::ProviderOrder,
                RuntimeSettingApplyMode::NextRequest,
                &[][..],
            ),
            (
                "log_format",
                RuntimeSettingGroup::Observability,
                RuntimeSettingControl::Select,
                RuntimeSettingApplyMode::RestartRequired,
                &["json", "compact"][..],
            ),
            (
                "log_filter",
                RuntimeSettingGroup::Observability,
                RuntimeSettingControl::Text,
                RuntimeSettingApplyMode::RestartRequired,
                &[][..],
            ),
        ];

        assert_eq!(settings.len(), expected.len());
        for (field, group, control, apply_mode, allowed_values) in expected {
            let setting = settings
                .iter()
                .find(|setting| setting.field == field)
                .expect("declared runtime setting should exist");
            assert_eq!(setting.group, group);
            assert_eq!(setting.control, control);
            assert_eq!(setting.apply_mode, apply_mode);
            assert_eq!(
                setting.allowed_values,
                allowed_values
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn runtime_setting_registry_defaults_are_exhaustive_and_parseable() {
        let fields = RUNTIME_CONFIG_DEFINITIONS
            .iter()
            .map(|definition| definition.field)
            .collect::<BTreeSet<_>>();

        assert_eq!(fields.len(), RUNTIME_CONFIG_DEFINITIONS.len());
        assert_eq!(
            RuntimeSettingKey::ALL.len(),
            RUNTIME_CONFIG_DEFINITIONS.len()
        );
        for key in RuntimeSettingKey::ALL {
            let definition = runtime_definition_by_key(key);
            assert_eq!(definition.field, key.as_str());
            assert!(!definition.label.is_empty());
            assert!(!definition.description.is_empty());
            assert!(!definition.input_type.is_empty());
            let canonical = parse_runtime_setting(key, definition.default_value)
                .expect("every registry default should parse")
                .into_text();
            assert_eq!(canonical, definition.default_value);
        }
    }

    #[test]
    fn audit_retention_setting_accepts_only_managed_day_bounds() {
        for value in ["1", "180", "3650"] {
            let parsed = parse_runtime_setting(RuntimeSettingKey::AuditRetentionDays, value)
                .expect("managed retention bound should parse");
            assert!(matches!(
                parsed,
                ParsedRuntimeSettingValue::UnsignedInteger(_)
            ));
        }
        for value in ["0", "3651", "-1", "not-a-number"] {
            let error = parse_runtime_setting(RuntimeSettingKey::AuditRetentionDays, value)
                .expect_err("out-of-range retention should fail");
            assert!(matches!(
                error,
                BusinessRepositoryError::InvalidRuntimeSetting(_)
            ));
        }
    }

    #[test]
    fn delivery_worker_concurrency_is_bounded_and_loaded_from_one_registry() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");

        assert_eq!(
            load_delivery_worker_concurrency(&auth_db_path)
                .expect("default delivery concurrency should load"),
            DEFAULT_DELIVERY_WORKER_CONCURRENCY
        );
        for value in ["1", "2", "16"] {
            assert!(matches!(
                parse_runtime_setting(RuntimeSettingKey::DeliveryWorkerConcurrency, value),
                Ok(ParsedRuntimeSettingValue::UnsignedInteger(_))
            ));
        }
        for value in ["0", "17", "-1", "not-a-number"] {
            assert!(matches!(
                parse_runtime_setting(RuntimeSettingKey::DeliveryWorkerConcurrency, value),
                Err(BusinessRepositoryError::InvalidRuntimeSetting(_))
            ));
        }
    }

    #[test]
    fn runtime_setting_registry_parses_exact_origins_and_header_lists() {
        let cors = parse_runtime_setting(
            RuntimeSettingKey::CorsAllowedOrigins,
            " ,https://paper.example,http://localhost:3000,http://[::1]:3000,https://paper.example, ",
        )
        .expect("exact CORS origins should parse");
        let mcp = parse_runtime_setting(
            RuntimeSettingKey::McpAllowedOrigins,
            "null,https://paper.example,http://localhost:3000,http://[::1]:3000",
        )
        .expect("exact MCP origins and null should parse");

        assert_eq!(
            cors,
            ParsedRuntimeSettingValue::StringList(vec![
                "https://paper.example".to_string(),
                "http://localhost:3000".to_string(),
                "http://[::1]:3000".to_string(),
                "https://paper.example".to_string(),
            ])
        );
        assert_eq!(
            mcp,
            ParsedRuntimeSettingValue::StringList(vec![
                "null".to_string(),
                "https://paper.example".to_string(),
                "http://localhost:3000".to_string(),
                "http://[::1]:3000".to_string(),
            ])
        );

        for origin in [
            "*",
            "null",
            "paper.example",
            "ftp://paper.example",
            "https://user@paper.example",
            "https://paper.example/",
            "https://paper.example/path",
            "https://paper.example?mode=admin",
            "https://paper.example#admin",
        ] {
            assert!(
                parse_runtime_setting(RuntimeSettingKey::CorsAllowedOrigins, origin).is_err(),
                "CORS origin should be rejected: {origin}"
            );
        }
        assert!(parse_runtime_setting(RuntimeSettingKey::CorsAllowedOrigins, " , ").is_ok());
        assert!(parse_runtime_setting(RuntimeSettingKey::McpAllowedOrigins, "").is_ok());
    }

    #[test]
    fn ai_endpoint_catalog_accepts_only_canonical_https_base_urls() {
        let parsed = parse_runtime_setting(
            RuntimeSettingKey::AiAllowedBaseUrls,
            "https://api.example/v1, https://backup.example:8443/openai/,https://api.example/v1/",
        )
        .expect("valid HTTPS base URLs should parse");

        assert_eq!(
            parsed,
            ParsedRuntimeSettingValue::StringList(vec![
                "https://api.example/v1/".to_string(),
                "https://backup.example:8443/openai/".to_string(),
            ])
        );
        for endpoint in [
            "http://api.example/v1",
            "https://user:secret@api.example/v1",
            "https://@api.example/v1",
            "https://api.example:0/v1",
            "https://api.example/v1?tenant=admin",
            "https://api.example/v1#fragment",
            "not-a-url",
        ] {
            assert!(
                parse_runtime_setting(RuntimeSettingKey::AiAllowedBaseUrls, endpoint).is_err(),
                "unsafe endpoint should be rejected"
            );
        }
    }

    #[test]
    fn auth_network_settings_are_strict_canonical_and_bounded() {
        let networks = parse_runtime_setting(
            RuntimeSettingKey::TrustedProxyCidrs,
            "10.2.3.4/8, 2001:db8::1234/32,127.0.0.1,10.0.0.0/8",
        )
        .expect("trusted proxy networks should parse");
        assert_eq!(
            networks.clone().into_text(),
            "10.0.0.0/8,2001:db8::/32,127.0.0.1/32"
        );
        let ParsedRuntimeSettingValue::TrustedProxyCidrs(networks) = networks else {
            panic!("trusted proxy parser should return typed networks");
        };
        assert!(networks[0].contains("10.9.8.7".parse().expect("IPv4 should parse")));
        assert!(!networks[0].contains("11.0.0.1".parse().expect("IPv4 should parse")));
        assert!(networks[1].contains("2001:db8::beef".parse().expect("IPv6 should parse")));

        let default_policy = parse_runtime_setting(
            RuntimeSettingKey::AuthRateLimitPolicy,
            DEFAULT_AUTH_RATE_LIMIT_POLICY_JSON,
        )
        .expect("default authentication policy should parse");
        assert_eq!(
            default_policy,
            ParsedRuntimeSettingValue::AuthRateLimitPolicy(AuthRateLimitPolicy::default())
        );
        assert_eq!(
            default_policy.into_text(),
            DEFAULT_AUTH_RATE_LIMIT_POLICY_JSON
        );

        for invalid_networks in ["10.0.0.1/33", "2001:db8::/129", "proxy.example/24"] {
            assert!(
                parse_runtime_setting(RuntimeSettingKey::TrustedProxyCidrs, invalid_networks)
                    .is_err()
            );
        }
        for invalid_policy in [
            "{}",
            r#"{"login_ip":{"capacity":0,"refill_tokens":1,"refill_seconds":1},"username":{"capacity":5,"refill_tokens":1,"refill_seconds":60},"register_ip":{"capacity":5,"refill_tokens":1,"refill_seconds":60},"global_login":{"capacity":1000,"refill_tokens":100,"refill_seconds":1},"global_register":{"capacity":250,"refill_tokens":25,"refill_seconds":1},"ip_key_limit":8192,"username_key_limit":4096}"#,
            r#"{"login_ip":{"capacity":30,"refill_tokens":1,"refill_seconds":1},"username":{"capacity":5,"refill_tokens":1,"refill_seconds":60},"register_ip":{"capacity":5,"refill_tokens":1,"refill_seconds":60},"global_login":{"capacity":1000,"refill_tokens":100,"refill_seconds":1},"global_register":{"capacity":250,"refill_tokens":25,"refill_seconds":1},"ip_key_limit":8192,"username_key_limit":4096,"unknown":true}"#,
        ] {
            assert!(
                parse_runtime_setting(RuntimeSettingKey::AuthRateLimitPolicy, invalid_policy)
                    .is_err()
            );
        }
        let mut undersized_global = AuthRateLimitPolicy::default();
        undersized_global.global_login.capacity = undersized_global.login_ip.capacity;
        assert!(parse_runtime_setting(
            RuntimeSettingKey::AuthRateLimitPolicy,
            &serde_json::to_string(&undersized_global).expect("policy should serialize")
        )
        .is_err());
        let mut slower_global = AuthRateLimitPolicy::default();
        slower_global.global_login.refill_tokens = 1;
        slower_global.global_login.refill_seconds = 60;
        assert!(parse_runtime_setting(
            RuntimeSettingKey::AuthRateLimitPolicy,
            &serde_json::to_string(&slower_global).expect("policy should serialize")
        )
        .is_err());
    }

    #[test]
    fn runtime_setting_write_and_startup_use_the_same_registry_parser() {
        let invalid_values = [
            (RuntimeSettingKey::McpAllowedHosts, "localhost,bad\nhost"),
            (RuntimeSettingKey::SecureCookies, "sometimes"),
            (RuntimeSettingKey::CorsAllowedOrigins, "*"),
            (RuntimeSettingKey::TrustedProxyCidrs, "10.0.0.1/99"),
            (RuntimeSettingKey::AuthRateLimitPolicy, "{}"),
            (RuntimeSettingKey::LogFormat, "pretty"),
            (RuntimeSettingKey::LogFilter, "["),
        ];

        for (key, invalid_value) in invalid_values {
            let temp_dir = tempdir().expect("temp dir should be created");
            let auth_db_path = temp_dir.path().join("auth.sqlite");
            migrate_auth_database(&auth_db_path).expect("auth database should migrate");
            let codec = SecretCodec::from_key([49_u8; 32]);
            let values = HashMap::from([
                (key.as_str().to_string(), Some(invalid_value.to_string())),
                (
                    "crossref_mailto_pool".to_string(),
                    Some("admin@example.com".to_string()),
                ),
            ]);

            assert!(parse_runtime_setting(key, invalid_value).is_err());
            upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
                .expect_err("write-time parser should reject the invalid value");
            let connection = Connection::open(&auth_db_path).expect("auth database should open");
            let stored_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM runtime_settings WHERE key IN (?1, 'crossref_mailto_pool')",
                    [key.as_str()],
                    |row| row.get(0),
                )
                .expect("runtime setting rows should be countable");
            assert_eq!(stored_count, 0, "failed update must roll back every field");
            connection
                .execute(
                    "INSERT INTO runtime_settings (key, value, updated_at) VALUES (?1, ?2, 1.0)",
                    (key.as_str(), invalid_value),
                )
                .expect("invalid startup fixture should insert directly");
            load_runtime_settings(&auth_db_path, &codec)
                .expect_err("startup parser should reject the same invalid value");
        }
    }

    #[test]
    fn logging_bootstrap_loader_is_read_only_and_uses_safe_defaults() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let missing_path = temp_dir.path().join("missing-auth.sqlite");
        let missing = load_runtime_logging_settings(&missing_path)
            .expect("missing database should use defaults");
        assert_eq!(missing, RuntimeLoggingSettings::default());
        assert!(!missing_path.exists());

        let empty_path = temp_dir.path().join("empty-auth.sqlite");
        drop(Connection::open(&empty_path).expect("empty database should be created"));
        let empty = load_runtime_logging_settings(&empty_path)
            .expect("database without runtime settings should use defaults");
        assert_eq!(empty, RuntimeLoggingSettings::default());
        let runtime_table_count: i64 = Connection::open(&empty_path)
            .expect("empty database should reopen")
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'runtime_settings'",
                [],
                |row| row.get(0),
            )
            .expect("schema should be inspectable");
        assert_eq!(runtime_table_count, 0);

        let auth_db_path = temp_dir.path().join("configured-auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([37_u8; 32]);
        upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([
                ("log_format".to_string(), Some("compact".to_string())),
                ("log_filter".to_string(), Some("off".to_string())),
            ]),
            &HashMap::new(),
        )
        .expect("logging settings should update");
        assert_eq!(
            load_runtime_logging_settings(&auth_db_path)
                .expect("stored logging settings should load"),
            RuntimeLoggingSettings {
                log_filter: "off".to_string(),
                log_format: "compact".to_string(),
            }
        );
        let invalid = upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("log_format".to_string(), Some("pretty".to_string()))]),
            &HashMap::new(),
        )
        .expect_err("unsupported log format should fail");
        assert!(matches!(
            invalid,
            BusinessRepositoryError::InvalidRuntimeSetting(_)
        ));
        assert_eq!(
            load_runtime_logging_settings(&auth_db_path)
                .expect("failed update should preserve logging settings")
                .log_format,
            "compact"
        );
    }

    #[test]
    fn runtime_provider_routes_and_orders_are_validated_and_normalized() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([31_u8; 32]);
        let values = HashMap::from([
            (
                "index_provider_routes".to_string(),
                Some(
                    "{ \"english_journals\": \"scholarly\", \"chinese_journals\": \"cnki\" }"
                        .to_string(),
                ),
            ),
            (
                "article_abstract_provider_orders".to_string(),
                Some(
                    r#"{
                        "catalogs": {
                            "chinese_journals": ["cnki_oversea", "scholarly"],
                            "disabled_catalog": []
                        },
                        "default": ["scholarly", "cnki_oversea"]
                    }"#
                    .to_string(),
                ),
            ),
        ]);

        let settings = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
            .expect("provider settings should update");
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.field == "index_provider_routes")
                .expect("route setting should exist")
                .value,
            "{\"chinese_journals\":\"cnki\",\"english_journals\":\"scholarly\"}"
        );
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.field == "article_abstract_provider_orders")
                .expect("abstract orders should exist")
                .value,
            "{\"default\":[\"scholarly\",\"cnki_oversea\"],\"catalogs\":{\"chinese_journals\":[\"cnki_oversea\",\"scholarly\"],\"disabled_catalog\":[]}}"
        );

        for invalid in [
            ("index_provider_routes", "{\"chinese_journals\":\"CNKI\"}"),
            (
                "article_abstract_provider_orders",
                "{\"default\":[\"scholarly\",\"scholarly\"],\"catalogs\":{}}",
            ),
            (
                "article_fulltext_provider_orders",
                "{\"default\":[\"zjlib cnki\"],\"catalogs\":{}}",
            ),
        ] {
            let error = upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::from([(invalid.0.to_string(), Some(invalid.1.to_string()))]),
                &HashMap::new(),
            )
            .expect_err("invalid provider setting should fail");
            assert!(matches!(
                error,
                BusinessRepositoryError::InvalidRuntimeSetting(_)
            ));
        }
    }
    #[test]
    fn runtime_settings_reject_proxy_pool_and_normalize_boolean() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([8_u8; 32]);
        let mut values = HashMap::new();
        values.insert("secure_cookies".to_string(), Some("yes".to_string()));

        let settings = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
            .expect("runtime settings should update");
        let secure_cookies = settings
            .iter()
            .find(|setting| setting.field == "secure_cookies")
            .expect("secure cookie setting should exist");

        assert_eq!(secure_cookies.value, "true");
        assert_eq!(secure_cookies.source, "database");

        values.clear();
        values.insert("proxy_pool".to_string(), Some("proxy".to_string()));
        let error = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
            .expect_err("proxy pool should be rejected");

        assert!(matches!(
            error,
            BusinessRepositoryError::UnknownRuntimeSetting(field) if field == "proxy_pool"
        ));
    }

    #[test]
    fn runtime_credentials_are_encrypted_and_use_preserve_replace_clear_updates() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([23_u8; 32]);
        let values = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            Some("key-one,key-two".to_string()),
        )]);

        let public = upsert_runtime_settings(&auth_db_path, &codec, &values, &HashMap::new())
            .expect("secret runtime setting should update");
        let openalex = public
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert_eq!(openalex.value, "");
        assert!(openalex.has_value);
        assert_eq!(openalex.masked_value, "••••");
        assert_eq!(openalex.secret_items.len(), 2);
        assert_eq!(openalex.secret_items[0].masked_value, "key-o**");
        assert_eq!(openalex.secret_items[1].masked_value, "key-t**");
        let raw: String = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted setting should load");
        assert!(raw.starts_with("litradarenc:v1:"));
        assert!(!raw.contains("key-one"));
        let internal = super::load_runtime_settings(&auth_db_path, &codec)
            .expect("trusted settings should decrypt");
        assert_eq!(
            internal
                .iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "key-one,key-two"
        );

        upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), Some(" ".to_string()))]),
            &HashMap::new(),
        )
        .expect("blank secret should preserve");
        assert_eq!(
            super::load_runtime_settings(&auth_db_path, &codec)
                .expect("trusted settings should decrypt")
                .into_iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "key-one,key-two"
        );

        let cleared = upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), None)]),
            &HashMap::new(),
        )
        .expect("null secret should clear");
        let openalex = cleared
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");
        assert!(!openalex.has_value);
        assert!(openalex.masked_value.is_empty());
        assert!(openalex.secret_items.is_empty());
    }

    #[test]
    fn runtime_scalar_secret_is_encrypted_and_publicly_redacted() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([41_u8; 32]);
        let sentinel = "captcha-secret-sentinel";

        let public = upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("cnki_captcha_token".to_string(), Some(sentinel.to_string()))]),
            &HashMap::new(),
        )
        .expect("scalar secret should update");
        let captcha = public
            .iter()
            .find(|setting| setting.field == "cnki_captcha_token")
            .expect("captcha setting should exist");
        assert_eq!(captcha.control, RuntimeSettingControl::Text);
        assert_eq!(captcha.input_type, "password");
        assert_eq!(captcha.value, "");
        assert!(captcha.has_value);
        assert_eq!(captcha.masked_value, "••••");
        assert!(captcha.secret_items.is_empty());
        assert!(!format!("{captcha:?}").contains(sentinel));

        let raw: String = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'cnki_captcha_token'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted scalar secret should load");
        assert!(raw.starts_with("litradarenc:v1:"));
        assert!(!raw.contains(sentinel));
        assert_eq!(
            load_runtime_settings(&auth_db_path, &codec)
                .expect("trusted settings should decrypt")
                .into_iter()
                .find(|setting| setting.field == "cnki_captcha_token")
                .expect("captcha setting should exist")
                .value,
            sentinel
        );

        let cleared = upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("cnki_captcha_token".to_string(), None)]),
            &HashMap::new(),
        )
        .expect("scalar secret should clear");
        let captcha = cleared
            .iter()
            .find(|setting| setting.field == "cnki_captcha_token")
            .expect("captcha setting should exist");
        assert_eq!(captcha.value, "");
        assert!(!captcha.has_value);
        assert_eq!(captcha.masked_value, "");
        assert!(captcha.secret_items.is_empty());
    }

    #[test]
    fn runtime_secret_pool_updates_are_exact_atomic_and_secret_safe() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([29_u8; 32]);
        let initial_values = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            Some("abcde-one,abcde-two,tiny".to_string()),
        )]);
        let initial =
            upsert_runtime_settings(&auth_db_path, &codec, &initial_values, &HashMap::new())
                .expect("initial secret pool should update");
        let openalex = initial
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist");

        assert_eq!(openalex.secret_items.len(), 3);
        assert_eq!(openalex.secret_items[0].masked_value, "abcde****");
        assert_eq!(openalex.secret_items[1].masked_value, "abcde****");
        assert_eq!(openalex.secret_items[2].masked_value, "****");
        assert!(!format!("{openalex:?}").contains(&openalex.secret_items[0].reference));

        let first_reference = openalex.secret_items[0].reference.clone();
        let second_reference = openalex.secret_items[1].reference.clone();
        let pool_updates = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["abcde-three; abcde-two\nnew-key".to_string()],
                remove: vec![first_reference.clone()],
            },
        )]);
        upsert_runtime_settings(&auth_db_path, &codec, &HashMap::new(), &pool_updates)
            .expect("incremental pool update should succeed");

        let internal = load_runtime_settings(&auth_db_path, &codec)
            .expect("updated secret pool should decrypt");
        let updated_value = &internal
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("OpenAlex setting should exist")
            .value;
        assert_eq!(updated_value, "abcde-two\ntiny\nabcde-three\nnew-key");
        let raw: String = Connection::open(&auth_db_path)
            .expect("auth database should open")
            .query_row(
                "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted setting should load");
        assert!(raw.starts_with("litradarenc:v1:"));
        assert!(!raw.contains("abcde-two"));

        let stale_update = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["must-not-commit".to_string()],
                remove: vec![first_reference],
            },
        )]);
        let stale_error =
            upsert_runtime_settings(&auth_db_path, &codec, &HashMap::new(), &stale_update)
                .expect_err("stale item reference should fail");
        assert!(matches!(
            stale_error,
            BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field)
                if field == "openalex_api_key_pool"
        ));
        assert_eq!(
            load_runtime_settings(&auth_db_path, &codec)
                .expect("failed update should roll back")
                .into_iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "abcde-two\ntiny\nabcde-three\nnew-key"
        );

        let tampered_update = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: Vec::new(),
                remove: vec![format!("{second_reference}A")],
            },
        )]);
        assert!(matches!(
            upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::new(),
                &tampered_update,
            ),
            Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field))
                if field == "openalex_api_key_pool"
        ));

        let cross_field_update = HashMap::from([(
            "semantic_scholar_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: Vec::new(),
                remove: vec![second_reference],
            },
        )]);
        assert!(matches!(
            upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::new(),
                &cross_field_update,
            ),
            Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field))
                if field == "semantic_scholar_api_key_pool"
        ));

        let non_secret_update = HashMap::from([(
            "crossref_mailto_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["admin@example.test".to_string()],
                remove: Vec::new(),
            },
        )]);
        assert!(matches!(
            upsert_runtime_settings(
                &auth_db_path,
                &codec,
                &HashMap::new(),
                &non_secret_update,
            ),
            Err(BusinessRepositoryError::InvalidRuntimeSecretPoolUpdate(field))
                if field == "crossref_mailto_pool"
        ));

        let replacement = HashMap::from([(
            "openalex_api_key_pool".to_string(),
            RuntimeSecretPoolUpdate {
                add: vec!["replacement-key".to_string()],
                remove: Vec::new(),
            },
        )]);
        upsert_runtime_settings(
            &auth_db_path,
            &codec,
            &HashMap::from([("openalex_api_key_pool".to_string(), None)]),
            &replacement,
        )
        .expect("clear then add should replace the pool");
        assert_eq!(
            load_runtime_settings(&auth_db_path, &codec)
                .expect("replacement pool should decrypt")
                .into_iter()
                .find(|setting| setting.field == "openalex_api_key_pool")
                .expect("OpenAlex setting should exist")
                .value,
            "replacement-key"
        );
    }

    #[test]
    fn end_state_defaults_prefer_domestic_cnki() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("auth database should migrate");
        let codec = SecretCodec::from_key([19_u8; 32]);
        let settings =
            list_runtime_settings(&auth_db_path, &codec).expect("runtime settings should load");
        let routes = settings
            .iter()
            .find(|setting| setting.field == "index_provider_routes")
            .expect("routes");
        assert!(routes.value.contains("\"chinese_journals\":\"cnki\""));
        assert!(!routes
            .value
            .contains("\"chinese_journals\":\"cnki_oversea\""));
        let abstracts = settings
            .iter()
            .find(|setting| setting.field == "article_abstract_provider_orders")
            .expect("abstracts");
        assert_eq!(
            abstracts.value,
            "{\"default\":[\"scholarly\",\"cnki\"],\"catalogs\":{}}"
        );
        let captcha = settings
            .iter()
            .find(|setting| setting.field == "cnki_captcha_token")
            .expect("captcha");
        assert!(captcha.is_secret);
        assert_eq!(captcha.value, "");
        assert_eq!(captcha.masked_value, "");
        assert!(!captcha.has_value);
    }
}
