//! Managed direct-or-explicit proxy selection for live Provider HTTP clients.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use litradar_domain::RuntimeSettingValue;
use reqwest::blocking::ClientBuilder;
use reqwest::{Proxy, Url};

use crate::providers::built_in_provider_capabilities;

/// A redacted proxy decision for one logical Provider.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProviderProxy {
    url: Option<Arc<str>>,
}

impl ProviderProxy {
    /// Build a deterministic direct-connection decision.
    ///
    /// # Returns
    ///
    /// Proxy decision that disables system proxy discovery.
    pub fn direct() -> Self {
        Self::default()
    }

    /// Build an explicit managed proxy decision.
    ///
    /// # Arguments
    ///
    /// * `url` - Validated HTTP, HTTPS, SOCKS5, or SOCKS5h proxy URL.
    ///
    /// # Returns
    ///
    /// Redacted proxy decision, or a credential-free validation error.
    pub fn explicit(url: impl Into<String>) -> Result<Self, ProviderProxyError> {
        let url = url.into();
        validate_proxy_url(&url)?;
        Proxy::all(&url).map_err(|_| ProviderProxyError::InvalidUrl)?;
        Ok(Self {
            url: Some(Arc::from(url)),
        })
    }

    /// Apply this decision to a blocking reqwest client builder.
    ///
    /// # Arguments
    ///
    /// * `builder` - Client builder whose proxy behavior is not yet finalized.
    ///
    /// # Returns
    ///
    /// Builder with ambient proxy discovery disabled and the managed proxy installed when present.
    pub fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder, ProviderProxyError> {
        let builder = builder.no_proxy();
        match &self.url {
            Some(url) => Proxy::all(url.as_ref())
                .map(|proxy| builder.proxy(proxy))
                .map_err(|_| ProviderProxyError::InvalidUrl),
            None => Ok(builder),
        }
    }

    /// Return whether this decision installs an explicit proxy.
    ///
    /// # Returns
    ///
    /// True for an explicit managed proxy and false for direct connections.
    pub fn is_explicit(&self) -> bool {
        self.url.is_some()
    }

    /// Return the selected URL for secret bootstrap propagation.
    ///
    /// # Returns
    ///
    /// Credential-bearing URL when explicit proxying is enabled.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

impl fmt::Debug for ProviderProxy {
    /// Format the decision without exposing proxy authority or credentials.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderProxy")
            .field(
                "mode",
                if self.is_explicit() {
                    &"explicit"
                } else {
                    &"direct"
                },
            )
            .field("url", &self.is_explicit().then_some("[REDACTED]"))
            .finish()
    }
}

/// Validated global proxy URL plus enabled logical Provider names.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderProxySelection {
    global_proxy: ProviderProxy,
    enabled_providers: BTreeSet<String>,
}

impl ProviderProxySelection {
    /// Build a selection from decrypted runtime-setting values.
    ///
    /// # Arguments
    ///
    /// * `settings` - Trusted runtime settings loaded from the managed registry.
    ///
    /// # Returns
    ///
    /// Validated selection, or a credential-free configuration error.
    pub fn from_runtime_settings(
        settings: &[RuntimeSettingValue],
    ) -> Result<Self, ProviderProxyError> {
        let proxy_url = settings
            .iter()
            .find(|setting| setting.field == "provider_proxy_url")
            .map(|setting| setting.value.as_str())
            .unwrap_or_default();
        let policy = settings
            .iter()
            .find(|setting| setting.field == "provider_proxy_policy")
            .map(|setting| setting.value.as_str())
            .unwrap_or("{}");
        Self::new(proxy_url, policy)
    }

    /// Build a selection from one global URL and canonical policy JSON.
    ///
    /// # Arguments
    ///
    /// * `proxy_url` - Decrypted proxy URL or an empty string.
    /// * `policy` - JSON object mapping Provider names to booleans.
    ///
    /// # Returns
    ///
    /// Validated selection, or a credential-free configuration error.
    pub fn new(proxy_url: &str, policy: &str) -> Result<Self, ProviderProxyError> {
        let policy = serde_json::from_str::<BTreeMap<String, bool>>(policy)
            .map_err(|_| ProviderProxyError::InvalidPolicy)?;
        let known_providers = built_in_provider_capabilities()
            .into_iter()
            .map(|provider| provider.name)
            .collect::<BTreeSet<_>>();
        for provider in policy.keys() {
            if !known_providers.contains(provider) {
                return Err(ProviderProxyError::UnknownProvider(provider.clone()));
            }
        }
        let enabled_providers = policy
            .into_iter()
            .filter_map(|(provider, is_enabled)| is_enabled.then_some(provider))
            .collect::<BTreeSet<_>>();
        let global_proxy = if proxy_url.trim().is_empty() {
            ProviderProxy::direct()
        } else {
            ProviderProxy::explicit(proxy_url.trim())?
        };
        if !enabled_providers.is_empty() && !global_proxy.is_explicit() {
            return Err(ProviderProxyError::MissingUrl);
        }
        Ok(Self {
            global_proxy,
            enabled_providers,
        })
    }

    /// Return the proxy decision for one logical Provider.
    ///
    /// # Arguments
    ///
    /// * `provider_name` - Stable logical Provider name.
    ///
    /// # Returns
    ///
    /// Explicit proxy for enabled Providers and a deterministic direct decision otherwise.
    pub fn for_provider(&self, provider_name: &str) -> ProviderProxy {
        if self.enabled_providers.contains(provider_name) {
            self.global_proxy.clone()
        } else {
            ProviderProxy::direct()
        }
    }

    /// Return the secret bootstrap URL selected for one Provider.
    ///
    /// # Arguments
    ///
    /// * `provider_name` - Stable logical Provider name.
    ///
    /// # Returns
    ///
    /// Cloned credential-bearing URL only when the Provider is enabled.
    pub fn proxy_url_for_provider(&self, provider_name: &str) -> Option<String> {
        self.for_provider(provider_name).url().map(str::to_string)
    }
}

impl Default for ProviderProxySelection {
    /// Build the managed-direct default selection.
    fn default() -> Self {
        Self {
            global_proxy: ProviderProxy::direct(),
            enabled_providers: BTreeSet::new(),
        }
    }
}

impl fmt::Debug for ProviderProxySelection {
    /// Format enabled names without exposing the configured URL.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderProxySelection")
            .field("has_global_proxy", &self.global_proxy.is_explicit())
            .field("enabled_providers", &self.enabled_providers)
            .field(
                "proxy_url",
                &self.global_proxy.is_explicit().then_some("[REDACTED]"),
            )
            .finish()
    }
}

/// Managed Provider proxy configuration error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderProxyError {
    /// The proxy URL failed fixed syntactic validation.
    InvalidUrl,
    /// The Provider policy was not a boolean JSON object.
    InvalidPolicy,
    /// The policy named a Provider absent from the built-in catalog.
    UnknownProvider(String),
    /// At least one Provider was enabled without a global URL.
    MissingUrl,
}

impl fmt::Display for ProviderProxyError {
    /// Format a credential-free configuration error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => formatter.write_str("Invalid Provider proxy URL"),
            Self::InvalidPolicy => formatter.write_str("Invalid Provider proxy policy"),
            Self::UnknownProvider(_) => formatter.write_str("Unknown Provider proxy policy name"),
            Self::MissingUrl => formatter
                .write_str("Provider proxy URL is required when a Provider proxy is enabled"),
        }
    }
}

impl Error for ProviderProxyError {}

fn validate_proxy_url(value: &str) -> Result<(), ProviderProxyError> {
    let url = Url::parse(value).map_err(|_| ProviderProxyError::InvalidUrl)?;
    let has_userinfo = !url.username().is_empty() || url.password().is_some();
    let has_complete_userinfo =
        !url.username().is_empty() && url.password().is_some_and(|password| !password.is_empty());
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        || url.host_str().is_none()
        || has_userinfo != has_complete_userinfo
        || url.port() == Some(0)
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
        || url.cannot_be_a_base()
    {
        return Err(ProviderProxyError::InvalidUrl);
    }
    Ok(())
}
