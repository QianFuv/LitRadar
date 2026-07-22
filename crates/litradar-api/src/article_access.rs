//! Runtime provider orchestration for online-only article actions.

use std::sync::Arc;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

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
    scholarly_access_registration, CnkiArticleAccessProvider, LiveCnkiConfig, LiveCnkiTransport,
    LiveZjlibCnkiConfig, LiveZjlibCnkiTransport, ZhejiangLibraryCnkiClient,
    ZjlibCnkiArticleIdentity, ZjlibCnkiDownloadedPdf, ZjlibCnkiError, CNKI_PROVIDER_NAME,
    CNKI_REDIRECT_HOSTS, DEFAULT_FULL_TEXT_MAXIMUM_BYTES, ZJLIB_CNKI_PROVIDER_NAME,
};
#[cfg(test)]
use litradar_sources::{FixtureZjlibCnkiMode, FixtureZjlibCnkiTransport};
use serde_json::json;

use crate::response::ApiError;
use crate::state::{ApiState, BlockingTaskError};

const ARTICLE_ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const ZJLIB_FULL_TEXT_TIMEOUT: Duration = Duration::from_secs(120);
#[cfg(test)]
static FULL_TEXT_FIXTURE_MODE: OnceLock<Mutex<Option<FixtureZjlibCnkiMode>>> = OnceLock::new();

/// Build all request-time providers available to the API process.
///
/// # Arguments
///
/// * `storage_config` - Storage paths used to read authenticated session context.
/// * `secret_codec` - Codec used to read the user's CNKI session.
///
/// # Returns
///
/// Validated runtime registry or a deterministic registration failure.
pub(crate) fn build_article_provider_registry(
    storage_config: litradar_storage::StorageConfig,
    secret_codec: litradar_storage::SecretCodec,
) -> Result<ProviderRegistry, ProviderRegistryError> {
    let mut registry = ProviderRegistry::default();
    registry.register(scholarly_access_registration()?)?;
    registry.register(live_cnki_access_registration()?)?;
    registry.register(zjlib_full_text_registration(storage_config, secret_codec)?)?;
    Ok(registry)
}

/// Return local action availability without resolving an upstream destination.
///
/// # Arguments
///
/// * `state` - Shared API state.
/// * `user_id` - Authenticated user identifier.
/// * `catalog_stem` - Canonical catalog configuration key.
///
/// # Returns
///
/// Provider-neutral action flags and labels.
pub(crate) async fn article_access_response(
    state: &ApiState,
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
        "查看摘要页",
        false,
    );
    let has_full_text_without_cnki_login = full_text_order.iter().any(|name| {
        name != ZJLIB_CNKI_PROVIDER_NAME
            && provider_has_capability(state, name, ProviderCapabilityKind::ArticleFullText)
    });
    let full_text_requires_login = !has_cnki_session
        && !has_full_text_without_cnki_login
        && full_text_order.iter().any(|name| {
            name == ZJLIB_CNKI_PROVIDER_NAME
                && provider_has_capability(state, name, ProviderCapabilityKind::ArticleFullText)
        });
    let fulltext = action_status(
        state,
        full_text_order,
        ProviderCapabilityKind::ArticleFullText,
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
    validate_article_locator(&article).map_err(|_| ApiError::internal_server_error())?;
    let orders = load_provider_orders(state).await?;
    let context = ArticleAccessContext {
        user_id: Some(user_id),
    };
    let mut did_require_authentication = false;
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
        let provider_name = name.to_string();
        let request_article = article.clone();
        let result = state
            .run_blocking_with_timeout(ARTICLE_ACTION_TIMEOUT, move || {
                provider.resolve_abstract(&request_article, context)
            })
            .await;
        let result = match result {
            Ok(result) => result,
            Err(BlockingTaskError::TimedOut) => {
                log_fallback(&provider_name, "abstract", "timeout");
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        match result {
            Ok(redirect)
                if validate_article_redirect(&redirect).is_ok()
                    && is_approved_redirect(&allowed_redirect_hosts, &redirect.location) =>
            {
                return Ok(redirect);
            }
            Ok(_) => log_fallback(&provider_name, "abstract", "invalid_response"),
            Err(error) if error.kind() == ProviderErrorKind::AuthenticationRequired => {
                did_require_authentication = true;
                log_fallback(&provider_name, "abstract", "authentication_required");
            }
            Err(error) => log_fallback(&provider_name, "abstract", error_kind_label(error.kind())),
        }
    }
    if did_require_authentication {
        return Err(authentication_required("abstract"));
    }
    Err(ApiError::not_found(
        "Article abstract action is unavailable",
    ))
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
    validate_article_locator(&article).map_err(|_| ApiError::internal_server_error())?;
    let orders = load_provider_orders(state).await?;
    let context = ArticleAccessContext {
        user_id: Some(user_id),
    };
    let mut did_require_authentication = false;
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
        let provider_name = name.to_string();
        let request_article = article.clone();
        let result = state
            .run_blocking_with_timeout(ZJLIB_FULL_TEXT_TIMEOUT, move || {
                provider.resolve_full_text(&request_article, context)
            })
            .await;
        let result = match result {
            Ok(result) => result,
            Err(BlockingTaskError::TimedOut) => {
                log_fallback(&provider_name, "fulltext", "timeout");
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        match result {
            Ok(resolution)
                if validate_full_text_resolution(&resolution, DEFAULT_FULL_TEXT_MAXIMUM_BYTES)
                    .is_ok()
                    && full_text_result_is_approved(&allowed_redirect_hosts, &resolution) =>
            {
                return Ok(resolution);
            }
            Ok(_) => log_fallback(&provider_name, "fulltext", "invalid_response"),
            Err(error) if error.kind() == ProviderErrorKind::AuthenticationRequired => {
                did_require_authentication = true;
                log_fallback(&provider_name, "fulltext", "authentication_required");
            }
            Err(error) => log_fallback(&provider_name, "fulltext", error_kind_label(error.kind())),
        }
    }
    if did_require_authentication {
        return Err(authentication_required("fulltext"));
    }
    Err(ApiError::not_found("Article full text is unavailable"))
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
    label: &str,
    requires_login: bool,
) -> ArticleAccessAction {
    let has_provider = order
        .iter()
        .any(|name| provider_has_capability(state, name, capability));
    let available = has_provider && !requires_login;
    ArticleAccessAction {
        available,
        label: label.to_string(),
        requires_login,
        message: if !has_provider {
            Some("当前未配置可用的在线能力".to_string())
        } else if requires_login {
            Some("请先完成浙江图书馆 CNKI 登录".to_string())
        } else {
            None
        },
    }
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
            litradar_storage::get_cnki_session_data(auth_db_path, &secret_codec, user_id)
                .map(|session| session.is_some_and(|session| session.status == "active"))
        })
        .await?
        .map_err(|_| ApiError::internal_server_error())
}

fn zjlib_full_text_registration(
    storage_config: litradar_storage::StorageConfig,
    secret_codec: litradar_storage::SecretCodec,
) -> Result<ProviderRegistration, ProviderRegistryError> {
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: ZJLIB_CNKI_PROVIDER_NAME.to_string(),
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
            })),
            ..ProviderImplementations::default()
        },
    )
}

struct ZjlibCnkiFullTextProvider {
    storage_config: litradar_storage::StorageConfig,
    secret_codec: litradar_storage::SecretCodec,
}

struct LiveCnkiAccessProvider {
    config: LiveCnkiConfig,
}

impl LiveCnkiAccessProvider {
    fn resolve(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        let transport = LiveCnkiTransport::new(self.config.clone()).map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::TemporarilyUnavailable,
                "CNKI transport is unavailable",
            )
        })?;
        CnkiArticleAccessProvider::new(transport).resolve_abstract(article, context)
    }
}

impl ArticleAbstractProvider for LiveCnkiAccessProvider {
    fn resolve_abstract(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        self.resolve(article, context)
    }
}

fn live_cnki_access_registration() -> Result<ProviderRegistration, ProviderRegistryError> {
    let provider = Arc::new(LiveCnkiAccessProvider {
        config: LiveCnkiConfig {
            timeout_seconds: ARTICLE_ACTION_TIMEOUT.as_secs(),
        },
    });
    ProviderRegistration::try_new(
        ProviderDescriptor {
            name: CNKI_PROVIDER_NAME.to_string(),
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

impl ArticleFullTextProvider for ZjlibCnkiFullTextProvider {
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
        let session = litradar_storage::get_cnki_session_data(
            self.storage_config.auth_db_path(),
            &self.secret_codec,
            user_id,
        )
        .map_err(|_| ProviderError::new(ProviderErrorKind::Internal, "CNKI session unavailable"))?
        .filter(|session| session.status == "active")
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
        let downloaded = download_zjlib_full_text(expected, session.session_data)
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
) -> Result<ZjlibCnkiDownloadedPdf, ZjlibCnkiError> {
    #[cfg(test)]
    if let Some(mode) = full_text_fixture_mode()
        .lock()
        .expect("full-text fixture mode lock should not be poisoned")
        .clone()
    {
        let mut client = ZhejiangLibraryCnkiClient::from_state_data(
            FixtureZjlibCnkiTransport::new(mode),
            &session_data,
        );
        client.warm_up_fulltext_session()?;
        return client.download_matching_pdf(&expected, 10);
    }
    let transport = LiveZjlibCnkiTransport::new(LiveZjlibCnkiConfig::default())?;
    let mut client = ZhejiangLibraryCnkiClient::from_state_data(transport, &session_data);
    client.warm_up_fulltext_session()?;
    client.download_matching_pdf(&expected, 10)
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

    use litradar_domain::ArticleId;
    use tempfile::{tempdir, TempDir};

    use super::*;

    enum RedirectFixtureOutcome {
        Error(ProviderErrorKind),
        Redirect(&'static str),
    }

    struct RedirectFixtureProvider {
        outcome: RedirectFixtureOutcome,
    }

    impl ArticleAbstractProvider for RedirectFixtureProvider {
        fn resolve_abstract(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleRedirect, ProviderError> {
            match self.outcome {
                RedirectFixtureOutcome::Error(kind) => {
                    Err(ProviderError::new(kind, "fixture provider failure"))
                }
                RedirectFixtureOutcome::Redirect(location) => Ok(ArticleRedirect {
                    location: location.to_string(),
                }),
            }
        }
    }

    struct AuthenticationRequiredFullTextProvider;

    impl ArticleFullTextProvider for AuthenticationRequiredFullTextProvider {
        fn resolve_full_text(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleFullTextResolution, ProviderError> {
            Err(ProviderError::new(
                ProviderErrorKind::AuthenticationRequired,
                "fixture authentication required",
            ))
        }
    }

    struct PdfFullTextProvider;

    impl ArticleFullTextProvider for PdfFullTextProvider {
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

    fn abstract_registration(name: &str, outcome: RedirectFixtureOutcome) -> ProviderRegistration {
        ProviderRegistration::try_new(
            ProviderDescriptor {
                name: name.to_string(),
                capabilities: ProviderCapabilities {
                    article_abstract: true,
                    ..ProviderCapabilities::default()
                },
                allowed_redirect_hosts: vec!["oversea.cnki.net".to_string()],
            },
            ProviderImplementations {
                article_abstract: Some(Arc::new(RedirectFixtureProvider { outcome })),
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
            default: vec!["scholarly".to_string(), "cnki".to_string()],
            catalogs: std::collections::BTreeMap::from([
                ("disabled".to_string(), Vec::new()),
                ("reverse".to_string(), vec!["cnki".to_string()]),
            ]),
        };
        assert_eq!(
            provider_order_for_catalog(&configuration, "inherited"),
            ["scholarly", "cnki"]
        );
        assert_eq!(
            provider_order_for_catalog(&configuration, "reverse"),
            ["cnki"]
        );
        assert!(provider_order_for_catalog(&configuration, "disabled").is_empty());
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
                "cnki",
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
                        "default": ["scholarly", "cnki"],
                        "catalogs": {"reverse": ["cnki", "scholarly"], "disabled": []}
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
            RedirectFixtureOutcome::Error(ProviderErrorKind::AuthenticationRequired),
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
                        "https://oversea.cnki.net/kcms/detail/fixture",
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
                "https://oversea.cnki.net/kcms/detail/fixture"
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
    async fn full_text_resolution_falls_back_after_authentication_requirement() {
        let mut registry = ProviderRegistry::default();
        registry
            .register(full_text_registration(
                ZJLIB_CNKI_PROVIDER_NAME,
                Arc::new(AuthenticationRequiredFullTextProvider),
            ))
            .expect("authenticated provider should register");
        registry
            .register(full_text_registration(
                "fixture",
                Arc::new(PdfFullTextProvider),
            ))
            .expect("fallback provider should register");
        let (_directory, state) = test_state(registry, Some("zjlib_cnki,fixture"));

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
                ZJLIB_CNKI_PROVIDER_NAME,
                Arc::new(AuthenticationRequiredFullTextProvider),
            ))
            .expect("authenticated provider should register");
        registry
            .register(full_text_registration(
                "fixture",
                Arc::new(PdfFullTextProvider),
            ))
            .expect("fallback provider should register");
        let (_directory, state) = test_state(registry, Some("zjlib_cnki,fixture"));

        let response = article_access_response(&state, UserId(1), "fixture")
            .await
            .expect("local action status should resolve");

        assert!(response.fulltext.available);
        assert!(!response.fulltext.requires_login);
        assert!(response.fulltext.message.is_none());
    }
}
