//! Runtime provider orchestration for online-only article actions.

use std::sync::Arc;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use litradar_domain::{
    ArticleAccessAction, ArticleAccessContext, ArticleAccessResponse, ArticleFullTextDocument,
    ArticleFullTextResolution, ArticleLocator, ArticleRedirect, ProviderCapabilityKind,
    ProviderOrderConfiguration, UserId,
};
use litradar_provider::conformance::{
    validate_article_locator, validate_article_redirect, validate_full_text_resolution,
};
use litradar_provider::{
    ArticleAbstractProvider, ArticleFullTextProvider, ProviderCapabilities, ProviderDescriptor,
    ProviderError, ProviderErrorKind, ProviderImplementations, ProviderRegistration,
    ProviderRegistry, ProviderRegistryError,
};
use litradar_sources::{
    scholarly_access_registration, CnkiArticleAccessProvider, DomesticCnkiArticleAccessProvider,
    LiveCnkiConfig, LiveCnkiTransport, LiveDomesticCnkiConfig, LiveDomesticCnkiTransport,
    LiveZjlibCnkiConfig, LiveZjlibCnkiTransport, ProviderProxy, ProviderProxySelection,
    ZhejiangLibraryCnkiClient, ZjlibCnkiArticleIdentity, ZjlibCnkiDownloadedPdf, ZjlibCnkiError,
    CNKI_OVERSEA_PROVIDER_NAME, CNKI_PROVIDER_NAME, CNKI_REDIRECT_HOSTS,
    DEFAULT_FULL_TEXT_MAXIMUM_BYTES, DOMESTIC_CNKI_REDIRECT_HOSTS, ZJLIB_PROVIDER_NAME,
};
#[cfg(test)]
use litradar_sources::{FixtureZjlibCnkiMode, FixtureZjlibCnkiTransport};
use serde_json::json;

use crate::response::ApiError;
use crate::state::{ApiState, BlockingTaskError};

const ARTICLE_ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const ARTICLE_ACTION_QUEUE_TIMEOUT: Duration = Duration::from_secs(30);
const ARTICLE_TRANSPORT_TIMEOUT_SECONDS: u64 = 30;
#[cfg(test)]
static FULL_TEXT_FIXTURE_MODE: OnceLock<Mutex<Option<FixtureZjlibCnkiMode>>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
enum ProviderChainFailureKind {
    Miss,
    AuthenticationRequired,
    Retryable,
    BadGateway,
}

#[derive(Debug, Clone, Copy, Default)]
struct ProviderChainFailures {
    has_authentication_requirement: bool,
    has_retryable_failure: bool,
    has_bad_gateway_failure: bool,
}

impl ProviderChainFailures {
    fn record_provider_error(&mut self, provider: &str, action: &str, kind: ProviderErrorKind) {
        let failure_kind = match kind {
            ProviderErrorKind::NotFound => ProviderChainFailureKind::Miss,
            ProviderErrorKind::AuthenticationRequired => {
                ProviderChainFailureKind::AuthenticationRequired
            }
            ProviderErrorKind::TemporarilyUnavailable => ProviderChainFailureKind::Retryable,
            ProviderErrorKind::InvalidResponse | ProviderErrorKind::Internal => {
                ProviderChainFailureKind::BadGateway
            }
        };
        self.record(failure_kind);
        log_fallback(provider, action, error_kind_label(kind));
    }

    fn record_invalid_response(&mut self, provider: &str, action: &str) {
        self.record(ProviderChainFailureKind::BadGateway);
        log_fallback(provider, action, "invalid_response");
    }

    fn record_deadline_expired(&mut self, provider: &str, action: &str) {
        self.record(ProviderChainFailureKind::Retryable);
        log_fallback(provider, action, "deadline_expired");
    }

    fn record_blocking_error(
        &mut self,
        provider: &str,
        action: &str,
        error: BlockingTaskError,
    ) -> bool {
        let (failure_kind, reason, should_continue) = match error {
            BlockingTaskError::Closed => (
                ProviderChainFailureKind::Retryable,
                "executor_closed",
                false,
            ),
            BlockingTaskError::QueueTimedOut => {
                (ProviderChainFailureKind::Retryable, "queue_timeout", true)
            }
            BlockingTaskError::Join => (
                ProviderChainFailureKind::BadGateway,
                "executor_join_failed",
                true,
            ),
        };
        self.record(failure_kind);
        log_fallback(provider, action, reason);
        should_continue
    }

    fn into_api_error(self, action: &str, not_found_detail: &'static str) -> ApiError {
        if self.has_authentication_requirement {
            authentication_required(action)
        } else if self.has_retryable_failure {
            ApiError::article_provider_service_unavailable()
        } else if self.has_bad_gateway_failure {
            ApiError::article_provider_bad_gateway()
        } else {
            ApiError::not_found(not_found_detail)
        }
    }

    fn record(&mut self, kind: ProviderChainFailureKind) {
        match kind {
            ProviderChainFailureKind::Miss => {}
            ProviderChainFailureKind::AuthenticationRequired => {
                self.has_authentication_requirement = true;
            }
            ProviderChainFailureKind::Retryable => {
                self.has_retryable_failure = true;
            }
            ProviderChainFailureKind::BadGateway => {
                self.has_bad_gateway_failure = true;
            }
        }
    }
}

/// Build all request-time providers available to the API process.
///
/// # Arguments
///
/// * `storage_config` - Storage paths used to read authenticated session context.
/// * `secret_codec` - Codec used to read the user's CNKI session.
/// * `provider_proxy_selection` - Startup-validated Provider proxy decisions.
///
/// # Returns
///
/// Validated runtime registry or a deterministic registration failure.
pub(crate) fn build_article_provider_registry(
    storage_config: litradar_storage::StorageConfig,
    secret_codec: litradar_storage::SecretCodec,
    provider_proxy_selection: ProviderProxySelection,
) -> Result<ProviderRegistry, ProviderRegistryError> {
    let captcha_token = load_cnki_captcha_token(&storage_config, &secret_codec);
    let mut registry = ProviderRegistry::default();
    registry.register(scholarly_access_registration()?)?;
    registry.register(live_cnki_oversea_access_registration(
        provider_proxy_selection.for_provider(CNKI_OVERSEA_PROVIDER_NAME),
    )?)?;
    registry.register(live_cnki_access_registration(
        captcha_token,
        provider_proxy_selection.for_provider(CNKI_PROVIDER_NAME),
    )?)?;
    registry.register(zjlib_full_text_registration(
        storage_config,
        secret_codec,
        provider_proxy_selection.for_provider(ZJLIB_PROVIDER_NAME),
    )?)?;
    Ok(registry)
}

/// Return local action availability without resolving an upstream destination.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `article` - Canonical article locator.
/// * `user_id` - Authenticated user identifier.
/// * `catalog_stem` - Canonical catalog configuration key.
///
/// # Returns
///
/// Provider-neutral action flags and labels.
pub(crate) async fn article_access_response(
    state: &ApiState,
    article: &ArticleLocator,
    user_id: UserId,
    catalog_stem: &str,
) -> Result<ArticleAccessResponse, ApiError> {
    let orders = load_provider_orders(state).await?;
    let abstract_order = provider_order_for_catalog(&orders.abstract_page, catalog_stem);
    let full_text_order = provider_order_for_catalog(&orders.full_text, catalog_stem);
    let has_cnki_session = has_active_cnki_session(state, user_id).await?;
    let abstract_page = action_status(
        state,
        abstract_order,
        ProviderCapabilityKind::ArticleAbstract,
        article,
        "查看摘要页",
        false,
    );
    let has_full_text_without_cnki_login = full_text_order.iter().any(|name| {
        name != ZJLIB_PROVIDER_NAME
            && provider_supports_article(
                state,
                name,
                ProviderCapabilityKind::ArticleFullText,
                article,
            )
    });
    let full_text_requires_login = !has_cnki_session
        && !has_full_text_without_cnki_login
        && full_text_order.iter().any(|name| {
            name == ZJLIB_PROVIDER_NAME
                && provider_supports_article(
                    state,
                    name,
                    ProviderCapabilityKind::ArticleFullText,
                    article,
                )
        });
    let fulltext = action_status(
        state,
        full_text_order,
        ProviderCapabilityKind::ArticleFullText,
        article,
        "获取全文",
        full_text_requires_login,
    );
    Ok(ArticleAccessResponse {
        abstract_page,
        fulltext,
    })
}

/// Resolve an abstract-page redirect through the configured provider chain.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `article` - Canonical article locator.
/// * `user_id` - Authenticated user identifier.
/// * `catalog_stem` - Canonical catalog configuration key.
/// # Returns
///
/// Validated ephemeral redirect or a stable API error.
pub(crate) async fn resolve_article_abstract(
    state: &ApiState,
    article: ArticleLocator,
    user_id: UserId,
    catalog_stem: &str,
) -> Result<ArticleRedirect, ApiError> {
    resolve_article_abstract_until(
        state,
        article,
        user_id,
        catalog_stem,
        Instant::now() + ARTICLE_ACTION_TIMEOUT,
    )
    .await
}

async fn resolve_article_abstract_until(
    state: &ApiState,
    article: ArticleLocator,
    user_id: UserId,
    catalog_stem: &str,
    deadline: Instant,
) -> Result<ArticleRedirect, ApiError> {
    validate_article_locator(&article).map_err(|_| ApiError::internal_server_error())?;
    let orders = load_provider_orders(state).await?;
    let context = ArticleAccessContext {
        user_id: Some(user_id),
        deadline: Some(deadline),
    };
    let mut failures = ProviderChainFailures::default();
    for name in provider_order_for_catalog(&orders.abstract_page, catalog_stem) {
        let Some((provider, allowed_redirect_hosts)) = state
            .article_providers()
            .find(name)
            .and_then(|registration| {
                registration.article_abstract().cloned().map(|provider| {
                    (
                        provider,
                        registration.descriptor().allowed_redirect_hosts.clone(),
                    )
                })
            })
        else {
            continue;
        };
        if !provider.supports_abstract(&article) {
            continue;
        }
        let provider_name = name.to_string();
        let Some(queue_timeout) = article_action_remaining(deadline) else {
            failures.record_deadline_expired(&provider_name, "abstract");
            break;
        };
        let request_article = article.clone();
        let result = state
            .run_upstream_blocking_with_queue_timeout(
                ARTICLE_ACTION_QUEUE_TIMEOUT.min(queue_timeout),
                move || provider.resolve_abstract(&request_article, context),
            )
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if failures.record_blocking_error(&provider_name, "abstract", error)
                    && article_action_remaining(deadline).is_some()
                {
                    continue;
                }
                break;
            }
        };
        if article_action_remaining(deadline).is_none() {
            if let Err(error) = &result {
                failures.record_provider_error(&provider_name, "abstract", error.kind());
            }
            failures.record_deadline_expired(&provider_name, "abstract");
            break;
        }
        match result {
            Ok(redirect)
                if validate_article_redirect(&redirect).is_ok()
                    && is_approved_redirect(&allowed_redirect_hosts, &redirect.location) =>
            {
                return Ok(redirect);
            }
            Ok(_) => failures.record_invalid_response(&provider_name, "abstract"),
            Err(error) => {
                failures.record_provider_error(&provider_name, "abstract", error.kind());
            }
        }
    }
    Err(failures.into_api_error("abstract", "Article abstract action is unavailable"))
}

/// Resolve full text through the configured provider chain.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `article` - Canonical article locator.
/// * `user_id` - Authenticated user identifier.
/// * `catalog_stem` - Canonical catalog configuration key.
///
/// # Returns
///
/// Validated ephemeral redirect or bounded document.
pub(crate) async fn resolve_article_full_text(
    state: &ApiState,
    article: ArticleLocator,
    user_id: UserId,
    catalog_stem: &str,
) -> Result<ArticleFullTextResolution, ApiError> {
    resolve_article_full_text_until(
        state,
        article,
        user_id,
        catalog_stem,
        Instant::now() + ARTICLE_ACTION_TIMEOUT,
    )
    .await
}

async fn resolve_article_full_text_until(
    state: &ApiState,
    article: ArticleLocator,
    user_id: UserId,
    catalog_stem: &str,
    deadline: Instant,
) -> Result<ArticleFullTextResolution, ApiError> {
    validate_article_locator(&article).map_err(|_| ApiError::internal_server_error())?;
    let orders = load_provider_orders(state).await?;
    let context = ArticleAccessContext {
        user_id: Some(user_id),
        deadline: Some(deadline),
    };
    let mut failures = ProviderChainFailures::default();
    for name in provider_order_for_catalog(&orders.full_text, catalog_stem) {
        let Some((provider, allowed_redirect_hosts)) = state
            .article_providers()
            .find(name)
            .and_then(|registration| {
                registration.article_full_text().cloned().map(|provider| {
                    (
                        provider,
                        registration.descriptor().allowed_redirect_hosts.clone(),
                    )
                })
            })
        else {
            continue;
        };
        if !provider.supports_full_text(&article) {
            continue;
        }
        let provider_name = name.to_string();
        let Some(queue_timeout) = article_action_remaining(deadline) else {
            failures.record_deadline_expired(&provider_name, "fulltext");
            break;
        };
        let request_article = article.clone();
        let result = state
            .run_upstream_blocking_with_queue_timeout(
                ARTICLE_ACTION_QUEUE_TIMEOUT.min(queue_timeout),
                move || provider.resolve_full_text(&request_article, context),
            )
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if failures.record_blocking_error(&provider_name, "fulltext", error)
                    && article_action_remaining(deadline).is_some()
                {
                    continue;
                }
                break;
            }
        };
        if article_action_remaining(deadline).is_none() {
            if let Err(error) = &result {
                failures.record_provider_error(&provider_name, "fulltext", error.kind());
            }
            failures.record_deadline_expired(&provider_name, "fulltext");
            break;
        }
        match result {
            Ok(resolution)
                if validate_full_text_resolution(&resolution, DEFAULT_FULL_TEXT_MAXIMUM_BYTES)
                    .is_ok()
                    && full_text_result_is_approved(&allowed_redirect_hosts, &resolution) =>
            {
                return Ok(resolution);
            }
            Ok(_) => failures.record_invalid_response(&provider_name, "fulltext"),
            Err(error) => {
                failures.record_provider_error(&provider_name, "fulltext", error.kind());
            }
        }
    }
    Err(failures.into_api_error("fulltext", "Article full text is unavailable"))
}

fn article_action_remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
}

#[derive(Debug, Clone, Default)]
struct ArticleProviderOrders {
    abstract_page: ProviderOrderConfiguration,
    full_text: ProviderOrderConfiguration,
}

async fn load_provider_orders(state: &ApiState) -> Result<ArticleProviderOrders, ApiError> {
    let auth_db_path = state.storage_config().auth_db_path().to_path_buf();
    let secret_codec = state.secret_codec().clone();
    state
        .run_blocking(move || {
            let values = litradar_storage::load_runtime_settings(&auth_db_path, &secret_codec)?;
            let mut orders = ArticleProviderOrders::default();
            for setting in values {
                match setting.field.as_str() {
                    "article_abstract_provider_orders" => {
                        orders.abstract_page =
                            serde_json::from_str(&setting.value).map_err(|_| {
                                litradar_storage::BusinessRepositoryError::InvalidRuntimeSetting(
                                    "Invalid stored article abstract Provider orders".to_string(),
                                )
                            })?;
                    }
                    "article_fulltext_provider_orders" => {
                        orders.full_text = serde_json::from_str(&setting.value).map_err(|_| {
                            litradar_storage::BusinessRepositoryError::InvalidRuntimeSetting(
                                "Invalid stored article full-text Provider orders".to_string(),
                            )
                        })?;
                    }
                    _ => {}
                }
            }
            Ok::<_, litradar_storage::BusinessRepositoryError>(orders)
        })
        .await?
        .map_err(|_| ApiError::internal_server_error())
}

fn provider_order_for_catalog<'configuration>(
    configuration: &'configuration ProviderOrderConfiguration,
    catalog_stem: &str,
) -> &'configuration [String] {
    configuration
        .catalogs
        .get(catalog_stem)
        .unwrap_or(&configuration.default)
}

fn action_status(
    state: &ApiState,
    order: &[String],
    capability: ProviderCapabilityKind,
    article: &ArticleLocator,
    label: &str,
    requires_login: bool,
) -> ArticleAccessAction {
    let has_configured_provider = order
        .iter()
        .any(|name| provider_has_capability(state, name, capability));
    let has_provider = order
        .iter()
        .any(|name| provider_supports_article(state, name, capability, article));
    let available = has_provider && !requires_login;
    ArticleAccessAction {
        available,
        label: label.to_string(),
        requires_login,
        message: if !has_configured_provider {
            Some("当前未配置可用的在线能力".to_string())
        } else if !has_provider {
            Some("当前文章缺少可用于在线解析的信息".to_string())
        } else if requires_login {
            Some("请先完成浙江图书馆 CNKI 登录".to_string())
        } else {
            None
        },
    }
}

fn provider_supports_article(
    state: &ApiState,
    name: &str,
    capability: ProviderCapabilityKind,
    article: &ArticleLocator,
) -> bool {
    state
        .article_providers()
        .find(name)
        .is_some_and(|registration| match capability {
            ProviderCapabilityKind::ArticleAbstract => registration
                .article_abstract()
                .is_some_and(|provider| provider.supports_abstract(article)),
            ProviderCapabilityKind::ArticleFullText => registration
                .article_full_text()
                .is_some_and(|provider| provider.supports_full_text(article)),
            ProviderCapabilityKind::IndexContent => false,
        })
}

fn provider_has_capability(
    state: &ApiState,
    name: &str,
    capability: ProviderCapabilityKind,
) -> bool {
    state
        .article_providers()
        .find(name)
        .is_some_and(|registration| registration.descriptor().capabilities.contains(capability))
}

async fn has_active_cnki_session(state: &ApiState, user_id: UserId) -> Result<bool, ApiError> {
    let auth_db_path = state.storage_config().auth_db_path().to_path_buf();
    let secret_codec = state.secret_codec().clone();
    state
        .run_blocking(move || {
            litradar_storage::cnki::get_active_cnki_session_data(
                auth_db_path,
                &secret_codec,
                user_id,
            )
            .map(|session| session.is_some())
        })
        .await?
        .map_err(|_| ApiError::internal_server_error())
}

fn zjlib_full_text_registration(
    storage_config: litradar_storage::StorageConfig,
    secret_codec: litradar_storage::SecretCodec,
    provider_proxy: ProviderProxy,
) -> Result<ProviderRegistration, ProviderRegistryError> {
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: ZJLIB_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                article_full_text: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: Vec::new(),
        },
        ProviderImplementations {
            article_full_text: Some(Arc::new(ZjlibCnkiFullTextProvider {
                storage_config,
                secret_codec,
                provider_proxy,
            })),
            ..ProviderImplementations::default()
        },
    )
}

struct ZjlibCnkiFullTextProvider {
    storage_config: litradar_storage::StorageConfig,
    secret_codec: litradar_storage::SecretCodec,
    provider_proxy: ProviderProxy,
}

struct LiveCnkiAccessProvider {
    config: LiveCnkiConfig,
    provider_proxy: ProviderProxy,
}

impl LiveCnkiAccessProvider {
    fn resolve(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        let transport = LiveCnkiTransport::new_with_proxy_and_deadline(
            self.config.clone(),
            self.provider_proxy.clone(),
            context.deadline,
        )
        .map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::TemporarilyUnavailable,
                "CNKI transport is unavailable",
            )
        })?;
        CnkiArticleAccessProvider::new(transport).resolve_abstract(article, context)
    }
}

impl ArticleAbstractProvider for LiveCnkiAccessProvider {
    fn supports_abstract(&self, article: &ArticleLocator) -> bool {
        CnkiArticleAccessProvider::<LiveCnkiTransport>::supports_article(article)
    }

    fn resolve_abstract(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        self.resolve(article, context)
    }
}

fn live_cnki_oversea_access_registration(
    provider_proxy: ProviderProxy,
) -> Result<ProviderRegistration, ProviderRegistryError> {
    let provider = Arc::new(LiveCnkiAccessProvider {
        config: LiveCnkiConfig {
            timeout_seconds: ARTICLE_TRANSPORT_TIMEOUT_SECONDS,
        },
        provider_proxy,
    });
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_OVERSEA_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                article_abstract: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: CNKI_REDIRECT_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
        ProviderImplementations {
            article_abstract: Some(provider),
            ..ProviderImplementations::default()
        },
    )
}

struct LiveDomesticCnkiAccessProvider {
    config: LiveDomesticCnkiConfig,
    provider_proxy: ProviderProxy,
}

impl LiveDomesticCnkiAccessProvider {
    fn resolve(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        let transport = LiveDomesticCnkiTransport::new_with_proxy_and_deadline(
            self.config.clone(),
            self.provider_proxy.clone(),
            context.deadline,
        )
        .map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::TemporarilyUnavailable,
                "domestic CNKI transport is unavailable",
            )
        })?;
        DomesticCnkiArticleAccessProvider::new(transport).resolve_abstract(article, context)
    }
}

impl ArticleAbstractProvider for LiveDomesticCnkiAccessProvider {
    fn supports_abstract(&self, article: &ArticleLocator) -> bool {
        DomesticCnkiArticleAccessProvider::<LiveDomesticCnkiTransport>::supports_article(article)
    }

    fn resolve_abstract(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        self.resolve(article, context)
    }
}

fn load_cnki_captcha_token(
    storage_config: &litradar_storage::StorageConfig,
    secret_codec: &litradar_storage::SecretCodec,
) -> Option<String> {
    litradar_storage::load_runtime_settings(storage_config.auth_db_path(), secret_codec)
        .ok()
        .and_then(|settings| {
            settings
                .into_iter()
                .find(|setting| setting.field == "cnki_captcha_token")
                .map(|setting| setting.value)
        })
        .filter(|value| !value.trim().is_empty())
}

fn live_cnki_access_registration(
    captcha_token: Option<String>,
    provider_proxy: ProviderProxy,
) -> Result<ProviderRegistration, ProviderRegistryError> {
    let provider = Arc::new(LiveDomesticCnkiAccessProvider {
        config: LiveDomesticCnkiConfig {
            timeout_seconds: ARTICLE_TRANSPORT_TIMEOUT_SECONDS,
            captcha_token,
        },
        provider_proxy,
    });
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_PROVIDER_NAME.to_string(),
            capabilities: ProviderCapabilities {
                article_abstract: true,
                ..ProviderCapabilities::default()
            },
            allowed_redirect_hosts: DOMESTIC_CNKI_REDIRECT_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
        ProviderImplementations {
            article_abstract: Some(provider),
            ..ProviderImplementations::default()
        },
    )
}

impl ArticleFullTextProvider for ZjlibCnkiFullTextProvider {
    fn supports_full_text(&self, article: &ArticleLocator) -> bool {
        !article.title.trim().is_empty()
            && !article.journal_title.trim().is_empty()
            && article
                .authors
                .iter()
                .any(|author| !author.trim().is_empty())
    }

    fn resolve_full_text(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleFullTextResolution, ProviderError> {
        let user_id = context.user_id.ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::AuthenticationRequired,
                "authenticated CNKI session required",
            )
        })?;
        let session = litradar_storage::cnki::get_active_cnki_session_data(
            self.storage_config.auth_db_path(),
            &self.secret_codec,
            user_id,
        )
        .map_err(|_| ProviderError::new(ProviderErrorKind::Internal, "CNKI session unavailable"))?
        .ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::AuthenticationRequired,
                "active CNKI session required",
            )
        })?;
        let expected = ZjlibCnkiArticleIdentity {
            title: article.title.clone(),
            authors: article.authors.join("; "),
            journal_title: article.journal_title.clone(),
        };
        let downloaded = download_zjlib_full_text(
            expected,
            session.session_data,
            self.provider_proxy.clone(),
            context.deadline,
        )
        .map_err(map_zjlib_provider_error)?;
        Ok(ArticleFullTextResolution::Document(
            ArticleFullTextDocument {
                content_type: downloaded.content_type.to_ascii_lowercase(),
                filename: Some(downloaded.filename),
                bytes: downloaded.content,
            },
        ))
    }
}

fn download_zjlib_full_text(
    expected: ZjlibCnkiArticleIdentity,
    session_data: serde_json::Value,
    provider_proxy: ProviderProxy,
    deadline: Option<Instant>,
) -> Result<ZjlibCnkiDownloadedPdf, ZjlibCnkiError> {
    #[cfg(test)]
    if let Some(mode) = full_text_fixture_mode()
        .lock()
        .expect("full-text fixture mode lock should not be poisoned")
        .clone()
    {
        ensure_zjlib_deadline(deadline)?;
        let mut client = ZhejiangLibraryCnkiClient::from_state_data(
            FixtureZjlibCnkiTransport::new(mode),
            &session_data,
        );
        client.warm_up_fulltext_session()?;
        ensure_zjlib_deadline(deadline)?;
        return client.download_matching_pdf(&expected, 10);
    }
    let transport = LiveZjlibCnkiTransport::new_with_proxy_and_deadline(
        LiveZjlibCnkiConfig {
            timeout_seconds: ARTICLE_TRANSPORT_TIMEOUT_SECONDS,
            ..LiveZjlibCnkiConfig::default()
        },
        provider_proxy,
        deadline,
    )?;
    let mut client = ZhejiangLibraryCnkiClient::from_state_data(transport, &session_data);
    client.warm_up_fulltext_session()?;
    client.download_matching_pdf(&expected, 10)
}

#[cfg(test)]
fn ensure_zjlib_deadline(deadline: Option<Instant>) -> Result<(), ZjlibCnkiError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(ZjlibCnkiError::Request(
            "article access deadline expired".to_string(),
        ));
    }
    Ok(())
}

fn map_zjlib_provider_error(error: ZjlibCnkiError) -> ProviderError {
    let message = error.to_string();
    let kind = if message.contains("No exact CNKI full-text match") {
        ProviderErrorKind::NotFound
    } else if message.contains("Run QR login") || message.contains("token") {
        ProviderErrorKind::AuthenticationRequired
    } else {
        ProviderErrorKind::TemporarilyUnavailable
    };
    ProviderError::new(kind, "Zhejiang Library CNKI full-text resolution failed")
}

fn full_text_result_is_approved(
    allowed_redirect_hosts: &[String],
    resolution: &ArticleFullTextResolution,
) -> bool {
    match resolution {
        ArticleFullTextResolution::Redirect(redirect) => {
            is_approved_redirect(allowed_redirect_hosts, &redirect.location)
        }
        ArticleFullTextResolution::Document(document) => document.content_type == "application/pdf",
    }
}

fn is_approved_redirect(allowed_hosts: &[String], location: &str) -> bool {
    let Some(remainder) = location.strip_prefix("https://") else {
        return false;
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, port)| {
            if port.bytes().all(|byte| byte.is_ascii_digit()) {
                host
            } else {
                authority
            }
        })
        .to_ascii_lowercase();
    allowed_hosts
        .iter()
        .any(|allowed_host| allowed_host == &host)
}

fn authentication_required(action: &str) -> ApiError {
    ApiError::json_detail(
        StatusCode::PRECONDITION_REQUIRED,
        json!({
            "code": "article_access_authentication_required",
            "action": action,
            "message": "Complete the configured provider login before retrying this action."
        }),
    )
}

fn error_kind_label(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::NotFound => "not_found",
        ProviderErrorKind::AuthenticationRequired => "authentication_required",
        ProviderErrorKind::TemporarilyUnavailable => "temporarily_unavailable",
        ProviderErrorKind::InvalidResponse => "invalid_response",
        ProviderErrorKind::Internal => "internal",
    }
}

fn log_fallback(provider: &str, action: &str, reason: &str) {
    tracing::debug!(
        event = "article.access.fallback",
        component = "article_access",
        provider,
        action,
        reason,
    );
}

#[cfg(test)]
fn full_text_fixture_mode() -> &'static Mutex<Option<FixtureZjlibCnkiMode>> {
    FULL_TEXT_FIXTURE_MODE.get_or_init(|| Mutex::new(None))
}

/// Set the deterministic Zhejiang Library full-text provider mode for route tests.
///
/// # Arguments
///
/// * `mode` - Optional fixture behavior.
#[cfg(test)]
pub(crate) fn set_full_text_fixture_mode(mode: Option<FixtureZjlibCnkiMode>) {
    *full_text_fixture_mode()
        .lock()
        .expect("full-text fixture mode lock should not be poisoned") = mode;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::http::header::RETRY_AFTER;
    use axum::response::IntoResponse;
    use litradar_domain::ArticleId;
    use rusqlite::Connection;
    use tempfile::{tempdir, TempDir};

    use super::*;

    const RAW_PROVIDER_SENTINEL: &str = "raw-provider-sentinel-must-stay-private";

    enum RedirectFixtureOutcome {
        Error(ProviderErrorKind),
        Redirect(&'static str),
    }

    struct RedirectFixtureProvider {
        outcome: RedirectFixtureOutcome,
        is_supported: bool,
    }

    impl ArticleAbstractProvider for RedirectFixtureProvider {
        fn supports_abstract(&self, _article: &ArticleLocator) -> bool {
            self.is_supported
        }

        fn resolve_abstract(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleRedirect, ProviderError> {
            assert!(self.is_supported, "unsupported provider must be skipped");
            match self.outcome {
                RedirectFixtureOutcome::Error(kind) => {
                    Err(ProviderError::new(kind, RAW_PROVIDER_SENTINEL))
                }
                RedirectFixtureOutcome::Redirect(location) => Ok(ArticleRedirect {
                    location: location.to_string(),
                }),
            }
        }
    }

    struct AuthenticationRequiredFullTextProvider {
        is_supported: bool,
    }

    impl ArticleFullTextProvider for AuthenticationRequiredFullTextProvider {
        fn supports_full_text(&self, _article: &ArticleLocator) -> bool {
            self.is_supported
        }

        fn resolve_full_text(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleFullTextResolution, ProviderError> {
            assert!(self.is_supported, "unsupported provider must be skipped");
            Err(ProviderError::new(
                ProviderErrorKind::AuthenticationRequired,
                "fixture authentication required",
            ))
        }
    }

    struct PdfFullTextProvider;

    impl ArticleFullTextProvider for PdfFullTextProvider {
        fn supports_full_text(&self, _article: &ArticleLocator) -> bool {
            true
        }

        fn resolve_full_text(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleFullTextResolution, ProviderError> {
            Ok(ArticleFullTextResolution::Document(
                ArticleFullTextDocument {
                    content_type: "application/pdf".to_string(),
                    filename: Some("fixture.pdf".to_string()),
                    bytes: b"%PDF-fixture".to_vec(),
                },
            ))
        }
    }

    #[derive(Clone, Copy)]
    enum DeadlineFixtureOutcome {
        Error(ProviderErrorKind),
        Redirect(&'static str),
    }

    struct DeadlineFixtureProvider {
        delay: Duration,
        call_count: Arc<AtomicUsize>,
        observed_deadlines: Arc<Mutex<Vec<Option<Instant>>>>,
        outcome: DeadlineFixtureOutcome,
    }

    impl ArticleAbstractProvider for DeadlineFixtureProvider {
        fn supports_abstract(&self, _article: &ArticleLocator) -> bool {
            true
        }

        fn resolve_abstract(
            &self,
            _article: &ArticleLocator,
            context: ArticleAccessContext,
        ) -> Result<ArticleRedirect, ProviderError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.observed_deadlines
                .lock()
                .expect("deadline observations should lock")
                .push(context.deadline);
            std::thread::sleep(self.delay);
            match self.outcome {
                DeadlineFixtureOutcome::Error(kind) => {
                    Err(ProviderError::new(kind, "deadline fixture failure"))
                }
                DeadlineFixtureOutcome::Redirect(location) => Ok(ArticleRedirect {
                    location: location.to_string(),
                }),
            }
        }
    }

    fn article_locator() -> ArticleLocator {
        ArticleLocator {
            article_id: ArticleId(1),
            catalog_id: "fixture-journal".to_string(),
            journal_title: "Fixture Journal".to_string(),
            journal_issns: vec!["1234-5679".to_string()],
            title: "Fixture Article".to_string(),
            publication_year: Some(2026),
            date: Some("2026-07-18".to_string()),
            authors: vec!["Ada Lovelace".to_string()],
            volume: Some("1".to_string()),
            issue_number: Some("2".to_string()),
            start_page: Some("1".to_string()),
            end_page: Some("8".to_string()),
            doi: Some("10.1000/fixture".to_string()),
            pmid: None,
        }
    }

    #[test]
    fn api_article_provider_ignores_index_only_captcha_environment() {
        let output = Command::new(
            std::env::current_exe().expect("current API test executable should resolve"),
        )
        .arg("--exact")
        .arg("article_access::tests::api_captcha_environment_helper")
        .arg("--ignored")
        .arg("--nocapture")
        .env(
            "LITRADAR_CNKI_CAPTCHA_TOKEN",
            "api-must-ignore-index-captcha-sentinel",
        )
        .output()
        .expect("captcha environment helper should run");

        assert!(
            output.status.success(),
            "captcha environment helper failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "private helper process for API captcha environment isolation"]
    fn api_captcha_environment_helper() {
        let directory = tempdir().expect("captcha helper directory should be created");
        let storage_config = litradar_storage::StorageConfig::from_project_root(directory.path());
        fs::create_dir_all(
            storage_config
                .auth_db_path()
                .parent()
                .expect("auth database should have a parent"),
        )
        .expect("auth database parent should be created");
        litradar_storage::initialize_auth_database(storage_config.auth_db_path())
            .expect("auth database should initialize");
        let secret_codec = litradar_storage::SecretCodec::from_key([71_u8; 32]);

        assert_eq!(
            load_cnki_captcha_token(&storage_config, &secret_codec),
            None,
            "serve must not consume the index-only captcha environment"
        );
        let database_token = "api-database-captcha-sentinel";
        litradar_storage::upsert_runtime_settings(
            storage_config.auth_db_path(),
            &secret_codec,
            &HashMap::from([(
                "cnki_captcha_token".to_string(),
                Some(database_token.to_string()),
            )]),
            &HashMap::new(),
        )
        .expect("database captcha token should update");
        assert_eq!(
            load_cnki_captcha_token(&storage_config, &secret_codec).as_deref(),
            Some(database_token),
            "serve should retain the encrypted database token path"
        );
    }

    fn abstract_registration(name: &str, outcome: RedirectFixtureOutcome) -> ProviderRegistration {
        abstract_registration_with_support(name, outcome, true)
    }

    fn abstract_registration_with_support(
        name: &str,
        outcome: RedirectFixtureOutcome,
        is_supported: bool,
    ) -> ProviderRegistration {
        ProviderRegistration::try_new(
            ProviderDescriptor {
                name: name.to_string(),
                capabilities: ProviderCapabilities {
                    article_abstract: true,
                    ..ProviderCapabilities::default()
                },
                allowed_redirect_hosts: vec![
                    "oversea.cnki.net".to_string(),
                    "navi.cnki.net".to_string(),
                    "kns.cnki.net".to_string(),
                    "www.cnki.net".to_string(),
                ],
            },
            ProviderImplementations {
                article_abstract: Some(Arc::new(RedirectFixtureProvider {
                    outcome,
                    is_supported,
                })),
                ..ProviderImplementations::default()
            },
        )
        .expect("abstract fixture registration should be valid")
    }

    fn full_text_registration(
        name: &str,
        provider: Arc<dyn ArticleFullTextProvider>,
    ) -> ProviderRegistration {
        ProviderRegistration::try_new(
            ProviderDescriptor {
                name: name.to_string(),
                capabilities: ProviderCapabilities {
                    article_full_text: true,
                    ..ProviderCapabilities::default()
                },
                allowed_redirect_hosts: Vec::new(),
            },
            ProviderImplementations {
                article_full_text: Some(provider),
                ..ProviderImplementations::default()
            },
        )
        .expect("full-text fixture registration should be valid")
    }

    fn deadline_abstract_registration(
        name: &str,
        provider: DeadlineFixtureProvider,
    ) -> ProviderRegistration {
        ProviderRegistration::try_new(
            ProviderDescriptor {
                name: name.to_string(),
                capabilities: ProviderCapabilities {
                    article_abstract: true,
                    ..ProviderCapabilities::default()
                },
                allowed_redirect_hosts: vec!["doi.org".to_string()],
            },
            ProviderImplementations {
                article_abstract: Some(Arc::new(provider)),
                ..ProviderImplementations::default()
            },
        )
        .expect("deadline abstract fixture registration should be valid")
    }

    fn test_state(
        registry: ProviderRegistry,
        full_text_order: Option<&str>,
    ) -> (TempDir, ApiState) {
        let directory = tempdir().expect("test directory should be created");
        let storage_config = litradar_storage::StorageConfig::from_project_root(directory.path());
        fs::create_dir_all(
            storage_config
                .auth_db_path()
                .parent()
                .expect("auth database should have a parent"),
        )
        .expect("auth database parent should be created");
        litradar_storage::initialize_auth_database(storage_config.auth_db_path())
            .expect("auth database should initialize");
        let secret_codec = litradar_storage::SecretCodec::from_key([42_u8; 32]);
        if let Some(order) = full_text_order {
            let providers = order
                .split(',')
                .map(str::trim)
                .filter(|provider| !provider.is_empty())
                .collect::<Vec<_>>();
            litradar_storage::upsert_runtime_settings(
                storage_config.auth_db_path(),
                &secret_codec,
                &HashMap::from([(
                    "article_fulltext_provider_orders".to_string(),
                    Some(json!({"default": providers, "catalogs": {}}).to_string()),
                )]),
                &HashMap::new(),
            )
            .expect("full-text order should update");
        }
        let state =
            ApiState::new(storage_config, secret_codec, false).with_article_providers(registry);
        (directory, state)
    }

    fn set_abstract_order(state: &ApiState, providers: &[&str]) {
        litradar_storage::upsert_runtime_settings(
            state.storage_config().auth_db_path(),
            state.secret_codec(),
            &HashMap::from([(
                "article_abstract_provider_orders".to_string(),
                Some(json!({"default": providers, "catalogs": {}}).to_string()),
            )]),
            &HashMap::new(),
        )
        .expect("abstract Provider order should update");
    }

    #[test]
    fn redirect_allowlist_rejects_userinfo_http_and_unregistered_hosts() {
        let allowed_hosts = vec!["doi.org".to_string()];
        assert!(is_approved_redirect(
            &allowed_hosts,
            "https://doi.org/10.1000/article"
        ));
        assert!(!is_approved_redirect(
            &allowed_hosts,
            "http://doi.org/10.1000/article"
        ));
        assert!(!is_approved_redirect(
            &allowed_hosts,
            "https://user@doi.org/10.1000/article"
        ));
        assert!(!is_approved_redirect(
            &allowed_hosts,
            "https://example.test/article"
        ));
    }

    #[test]
    fn provider_order_selection_distinguishes_inherit_override_and_disable() {
        let configuration = ProviderOrderConfiguration {
            default: vec!["scholarly".to_string(), "cnki_oversea".to_string()],
            catalogs: std::collections::BTreeMap::from([
                ("disabled".to_string(), Vec::new()),
                ("reverse".to_string(), vec!["cnki_oversea".to_string()]),
            ]),
        };
        assert_eq!(
            provider_order_for_catalog(&configuration, "inherited"),
            ["scholarly", "cnki_oversea"]
        );
        assert_eq!(
            provider_order_for_catalog(&configuration, "reverse"),
            ["cnki_oversea"]
        );
        assert!(provider_order_for_catalog(&configuration, "disabled").is_empty());
    }

    #[test]
    fn provider_failure_summary_preserves_terminal_precedence() {
        let cases = [
            (Vec::new(), StatusCode::NOT_FOUND, false),
            (
                vec![ProviderErrorKind::NotFound],
                StatusCode::NOT_FOUND,
                false,
            ),
            (
                vec![ProviderErrorKind::Internal],
                StatusCode::BAD_GATEWAY,
                false,
            ),
            (
                vec![ProviderErrorKind::InvalidResponse],
                StatusCode::BAD_GATEWAY,
                false,
            ),
            (
                vec![
                    ProviderErrorKind::Internal,
                    ProviderErrorKind::TemporarilyUnavailable,
                ],
                StatusCode::SERVICE_UNAVAILABLE,
                true,
            ),
            (
                vec![
                    ProviderErrorKind::Internal,
                    ProviderErrorKind::TemporarilyUnavailable,
                    ProviderErrorKind::AuthenticationRequired,
                ],
                StatusCode::PRECONDITION_REQUIRED,
                false,
            ),
        ];

        for (kinds, expected_status, has_retry_after) in cases {
            let mut failures = ProviderChainFailures::default();
            for kind in kinds {
                failures.record_provider_error("fixture", "abstract", kind);
            }
            let response = failures
                .into_api_error("abstract", "Article abstract action is unavailable")
                .into_response();

            assert_eq!(response.status(), expected_status);
            assert_eq!(
                response.headers().contains_key(RETRY_AFTER),
                has_retry_after
            );
            if has_retry_after {
                assert_eq!(
                    response
                        .headers()
                        .get(RETRY_AFTER)
                        .expect("retryable failure should include Retry-After"),
                    "5"
                );
            }
        }

        for (error, expected_status, should_continue) in [
            (
                BlockingTaskError::QueueTimedOut,
                StatusCode::SERVICE_UNAVAILABLE,
                true,
            ),
            (
                BlockingTaskError::Closed,
                StatusCode::SERVICE_UNAVAILABLE,
                false,
            ),
            (BlockingTaskError::Join, StatusCode::BAD_GATEWAY, true),
        ] {
            let mut failures = ProviderChainFailures::default();
            assert_eq!(
                failures.record_blocking_error("fixture", "abstract", error),
                should_continue
            );
            let response = failures
                .into_api_error("abstract", "Article abstract action is unavailable")
                .into_response();
            assert_eq!(response.status(), expected_status);
        }
    }

    #[tokio::test]
    async fn fallback_providers_observe_one_shared_deadline() {
        let observed_deadlines = Arc::new(Mutex::new(Vec::new()));
        let first_calls = Arc::new(AtomicUsize::new(0));
        let second_calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ProviderRegistry::default();
        registry
            .register(deadline_abstract_registration(
                "first",
                DeadlineFixtureProvider {
                    delay: Duration::ZERO,
                    call_count: Arc::clone(&first_calls),
                    observed_deadlines: Arc::clone(&observed_deadlines),
                    outcome: DeadlineFixtureOutcome::Error(ProviderErrorKind::NotFound),
                },
            ))
            .expect("first deadline fixture should register");
        registry
            .register(deadline_abstract_registration(
                "second",
                DeadlineFixtureProvider {
                    delay: Duration::ZERO,
                    call_count: Arc::clone(&second_calls),
                    observed_deadlines: Arc::clone(&observed_deadlines),
                    outcome: DeadlineFixtureOutcome::Redirect(
                        "https://doi.org/10.1000/deadline-fallback",
                    ),
                },
            ))
            .expect("second deadline fixture should register");
        let (_directory, state) = test_state(registry, None);
        set_abstract_order(&state, &["first", "second"]);
        let deadline = Instant::now() + Duration::from_secs(1);

        let redirect = resolve_article_abstract_until(
            &state,
            article_locator(),
            UserId(1),
            "fixture",
            deadline,
        )
        .await
        .expect("fallback should resolve inside the shared deadline");

        assert_eq!(
            redirect.location,
            "https://doi.org/10.1000/deadline-fallback"
        );
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *observed_deadlines
                .lock()
                .expect("deadline observations should lock"),
            [Some(deadline), Some(deadline)]
        );
    }

    #[tokio::test]
    async fn expired_deadline_waits_for_started_work_and_skips_later_provider() {
        let observed_deadlines = Arc::new(Mutex::new(Vec::new()));
        let first_calls = Arc::new(AtomicUsize::new(0));
        let second_calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ProviderRegistry::default();
        registry
            .register(deadline_abstract_registration(
                "slow",
                DeadlineFixtureProvider {
                    delay: Duration::from_millis(350),
                    call_count: Arc::clone(&first_calls),
                    observed_deadlines: Arc::clone(&observed_deadlines),
                    outcome: DeadlineFixtureOutcome::Error(
                        ProviderErrorKind::TemporarilyUnavailable,
                    ),
                },
            ))
            .expect("slow deadline fixture should register");
        registry
            .register(deadline_abstract_registration(
                "later",
                DeadlineFixtureProvider {
                    delay: Duration::ZERO,
                    call_count: Arc::clone(&second_calls),
                    observed_deadlines: Arc::clone(&observed_deadlines),
                    outcome: DeadlineFixtureOutcome::Redirect(
                        "https://doi.org/10.1000/must-not-run",
                    ),
                },
            ))
            .expect("later deadline fixture should register");
        let (_directory, state) = test_state(registry, None);
        set_abstract_order(&state, &["slow", "later"]);
        let started_at = Instant::now();
        let deadline = started_at + Duration::from_millis(250);

        let error = resolve_article_abstract_until(
            &state,
            article_locator(),
            UserId(1),
            "fixture",
            deadline,
        )
        .await
        .expect_err("expired chain should return a retryable failure");
        let elapsed = started_at.elapsed();
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(RETRY_AFTER)
                .expect("deadline failure should include Retry-After"),
            "5"
        );
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
        assert!(elapsed >= Duration::from_millis(300));
        assert!(elapsed < Duration::from_secs(2));
        assert_eq!(
            *observed_deadlines
                .lock()
                .expect("deadline observations should lock"),
            [Some(deadline)]
        );
    }

    #[tokio::test]
    async fn abstract_resolution_uses_catalog_override_and_explicit_disable() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(abstract_registration(
                "scholarly",
                RedirectFixtureOutcome::Redirect("https://oversea.cnki.net/kcms/detail/scholarly"),
            ))
            .expect("Scholarly fixture should register");
        registry
            .register(abstract_registration(
                "cnki_oversea",
                RedirectFixtureOutcome::Redirect("https://oversea.cnki.net/kcms/detail/cnki"),
            ))
            .expect("CNKI fixture should register");
        let (_directory, state) = test_state(registry, None);
        litradar_storage::upsert_runtime_settings(
            state.storage_config().auth_db_path(),
            state.secret_codec(),
            &HashMap::from([(
                "article_abstract_provider_orders".to_string(),
                Some(
                    json!({
                        "default": ["scholarly", "cnki_oversea"],
                        "catalogs": {"reverse": ["cnki_oversea", "scholarly"], "disabled": []}
                    })
                    .to_string(),
                ),
            )]),
            &HashMap::new(),
        )
        .expect("abstract Provider orders should update");

        let redirect = resolve_article_abstract(&state, article_locator(), UserId(1), "reverse")
            .await
            .expect("catalog override should resolve");
        assert_eq!(
            redirect.location,
            "https://oversea.cnki.net/kcms/detail/cnki"
        );

        let error = resolve_article_abstract(&state, article_locator(), UserId(1), "disabled")
            .await
            .expect_err("empty catalog override should disable abstract access");
        match error {
            ApiError::Http { status, .. } => assert_eq!(status, StatusCode::NOT_FOUND),
            ApiError::JsonDetail { .. }
            | ApiError::TooManyRequests { .. }
            | ApiError::Unexpected { .. } => panic!("expected not-found HTTP error"),
        }
    }

    #[tokio::test]
    async fn redirect_resolution_falls_back_after_authentication_and_invalid_results() {
        for first_outcome in [
            RedirectFixtureOutcome::Error(ProviderErrorKind::NotFound),
            RedirectFixtureOutcome::Error(ProviderErrorKind::AuthenticationRequired),
            RedirectFixtureOutcome::Error(ProviderErrorKind::TemporarilyUnavailable),
            RedirectFixtureOutcome::Error(ProviderErrorKind::InvalidResponse),
            RedirectFixtureOutcome::Error(ProviderErrorKind::Internal),
            RedirectFixtureOutcome::Redirect("https://example.test/unsafe"),
        ] {
            let mut registry = ProviderRegistry::default();
            registry
                .register(abstract_registration("scholarly", first_outcome))
                .expect("first provider should register");
            registry
                .register(abstract_registration(
                    "cnki",
                    RedirectFixtureOutcome::Redirect(
                        "https://navi.cnki.net/knavi/journals/detail?pcode=CJFD&pykm=fixture",
                    ),
                ))
                .expect("fallback provider should register");
            let (_directory, state) = test_state(registry, None);

            let redirect =
                resolve_article_abstract(&state, article_locator(), UserId(1), "fixture")
                    .await
                    .expect("fallback provider should resolve");

            assert_eq!(
                redirect.location,
                "https://navi.cnki.net/knavi/journals/detail?pcode=CJFD&pykm=fixture"
            );
        }
    }

    #[tokio::test]
    async fn redirect_resolution_reports_unavailable_without_a_capable_provider() {
        let (_directory, state) = test_state(ProviderRegistry::default(), None);

        let error = resolve_article_abstract(&state, article_locator(), UserId(1), "fixture")
            .await
            .expect_err("missing providers should fail");

        match error {
            ApiError::Http { status, .. } => assert_eq!(status, StatusCode::NOT_FOUND),
            ApiError::JsonDetail { .. }
            | ApiError::TooManyRequests { .. }
            | ApiError::Unexpected { .. } => panic!("expected not-found HTTP error"),
        }
    }

    #[tokio::test]
    async fn scholarly_status_requires_an_external_identifier() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(scholarly_access_registration().expect("Scholarly should build"))
            .expect("Scholarly should register");
        let (_directory, state) = test_state(registry, None);
        set_abstract_order(&state, &["scholarly"]);
        let mut article = article_locator();
        article.doi = None;
        article.pmid = None;

        let response = article_access_response(&state, &article, UserId(1), "fixture")
            .await
            .expect("local action status should resolve");
        let error = resolve_article_abstract(&state, article, UserId(1), "fixture")
            .await
            .expect_err("unsupported Scholarly article should not resolve");

        assert!(!response.abstract_page.available);
        assert!(!response.abstract_page.requires_login);
        assert_eq!(
            response.abstract_page.message.as_deref(),
            Some("当前文章缺少可用于在线解析的信息")
        );
        match error {
            ApiError::Http { status, .. } => assert_eq!(status, StatusCode::NOT_FOUND),
            ApiError::JsonDetail { .. }
            | ApiError::TooManyRequests { .. }
            | ApiError::Unexpected { .. } => panic!("expected not-found HTTP error"),
        }
    }

    #[tokio::test]
    async fn locator_filter_skips_an_inapplicable_provider_before_fallback() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(abstract_registration_with_support(
                "scholarly",
                RedirectFixtureOutcome::Redirect("https://oversea.cnki.net/unsupported"),
                false,
            ))
            .expect("unsupported fixture should register");
        registry
            .register(abstract_registration(
                "cnki_oversea",
                RedirectFixtureOutcome::Redirect("https://oversea.cnki.net/supported"),
            ))
            .expect("fallback fixture should register");
        let (_directory, state) = test_state(registry, None);
        set_abstract_order(&state, &["scholarly", "cnki_oversea"]);
        let article = article_locator();

        let response = article_access_response(&state, &article, UserId(1), "fixture")
            .await
            .expect("local action status should resolve");
        let redirect = resolve_article_abstract(&state, article, UserId(1), "fixture")
            .await
            .expect("supported fallback should resolve");

        assert!(response.abstract_page.available);
        assert_eq!(redirect.location, "https://oversea.cnki.net/supported");
    }

    #[tokio::test]
    async fn full_text_resolution_falls_back_after_authentication_requirement() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(full_text_registration(
                ZJLIB_PROVIDER_NAME,
                Arc::new(AuthenticationRequiredFullTextProvider { is_supported: true }),
            ))
            .expect("authenticated provider should register");
        registry
            .register(full_text_registration(
                "fixture",
                Arc::new(PdfFullTextProvider),
            ))
            .expect("fallback provider should register");
        let (_directory, state) = test_state(registry, Some("zjlib,fixture"));

        let resolution = resolve_article_full_text(&state, article_locator(), UserId(1), "fixture")
            .await
            .expect("full-text fallback should resolve");

        match resolution {
            ArticleFullTextResolution::Document(document) => {
                assert_eq!(document.content_type, "application/pdf");
                assert_eq!(document.bytes, b"%PDF-fixture");
            }
            ArticleFullTextResolution::Redirect(_) => panic!("expected PDF document"),
        }
    }

    #[tokio::test]
    async fn access_status_keeps_full_text_available_for_a_login_free_fallback() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(full_text_registration(
                ZJLIB_PROVIDER_NAME,
                Arc::new(AuthenticationRequiredFullTextProvider { is_supported: true }),
            ))
            .expect("authenticated provider should register");
        registry
            .register(full_text_registration(
                "fixture",
                Arc::new(PdfFullTextProvider),
            ))
            .expect("fallback provider should register");
        let (_directory, state) = test_state(registry, Some("zjlib,fixture"));

        let article = article_locator();
        let response = article_access_response(&state, &article, UserId(1), "fixture")
            .await
            .expect("local action status should resolve");

        assert!(response.fulltext.available);
        assert!(!response.fulltext.requires_login);
        assert!(response.fulltext.message.is_none());
    }

    #[tokio::test]
    async fn expired_cnki_session_requires_login_for_article_access() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(full_text_registration(
                ZJLIB_PROVIDER_NAME,
                Arc::new(AuthenticationRequiredFullTextProvider { is_supported: true }),
            ))
            .expect("authenticated provider should register");
        let (_directory, state) = test_state(registry, Some("zjlib"));
        let user_id = UserId(1);
        Connection::open(state.storage_config().auth_db_path())
            .expect("auth database should open")
            .execute(
                "INSERT INTO users (id, username, password_hash, salt, is_admin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 0)",
                (user_id.value(), "expired-cnki-user", "hash", "salt"),
            )
            .expect("user fixture should insert");
        litradar_storage::upsert_cnki_session(
            state.storage_config().auth_db_path(),
            state.secret_codec(),
            user_id,
            &json!({"bff_user_token": "header.eyJleHAiOjF9.signature"}),
            &litradar_domain::CnkiStatus::Active,
            None,
        )
        .expect("expired session fixture should store");

        let response = article_access_response(&state, &article_locator(), user_id, "fixture")
            .await
            .expect("local action status should resolve");

        assert!(!response.fulltext.available);
        assert!(response.fulltext.requires_login);
    }

    #[tokio::test]
    async fn access_status_requires_login_only_for_applicable_full_text_providers() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(full_text_registration(
                ZJLIB_PROVIDER_NAME,
                Arc::new(AuthenticationRequiredFullTextProvider {
                    is_supported: false,
                }),
            ))
            .expect("unsupported authenticated provider should register");
        let (_directory, state) = test_state(registry, Some("zjlib"));
        let article = article_locator();

        let response = article_access_response(&state, &article, UserId(1), "fixture")
            .await
            .expect("local action status should resolve");
        let error = resolve_article_full_text(&state, article, UserId(1), "fixture")
            .await
            .expect_err("unsupported full-text Provider should be skipped");

        assert!(!response.fulltext.available);
        assert!(!response.fulltext.requires_login);
        assert_eq!(
            response.fulltext.message.as_deref(),
            Some("当前文章缺少可用于在线解析的信息")
        );
        match error {
            ApiError::Http { status, .. } => assert_eq!(status, StatusCode::NOT_FOUND),
            ApiError::JsonDetail { .. }
            | ApiError::TooManyRequests { .. }
            | ApiError::Unexpected { .. } => panic!("expected not-found HTTP error"),
        }
    }
}
