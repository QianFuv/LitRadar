//! Zhejiang Library mediated CNKI login and full-text session client.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Response};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, LOCATION, ORIGIN, REFERER,
    USER_AGENT,
};
use reqwest::redirect::Policy;
use reqwest::Url;
use serde_json::{json, Value};

const WWW_BASE_URL: &str = "https://www.zjlib.cn";
const SHARE_BASE_URL: &str = "https://share.zjlib.cn";
const ZYPROXY_BASE_URL: &str = "https://http-10--18--17--173.elib.zyproxy.zjlib.cn";
const ZYPROXY_LOGIN_HOST: &str = "login.elib.zyproxy.zjlib.cn";
const ENTRY_URL: &str = "https://share.zjlib.cn/entry/area/35594/2120";
const LIBRARY_REFER: &str = "http://10.18.17.173/kns55/";
const WFWFID: &str = "2120";
const BFF_ORG_ID: &str = "1916318653650423810";
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
/// Default maximum size accepted for one request-time full-text document.
pub const DEFAULT_FULL_TEXT_MAXIMUM_BYTES: usize = 32 * 1024 * 1024;
const TOKEN_EXPIRY_SKEW_SECONDS: i64 = 300;
const FULLTEXT_WARM_UP_TTL_SECONDS: i64 = 60 * 60;
const ZYPROXY_LOGIN_ATTEMPTS: usize = 3;
const ZYPROXY_REDIRECT_HOPS: usize = 4;
const ZYPROXY_RETRY_DELAY_MILLIS: u64 = 200;
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";
const DEFAULT_ACCEPT_LANGUAGE: &str = "zh-CN;q=0.9";

/// Errors returned by the Zhejiang Library CNKI client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZjlibCnkiError {
    /// An HTTP request, upstream status, or protocol step failed.
    Request(String),
    /// An upstream response could not be parsed.
    Parse(String),
    /// QR login polling reached its timeout.
    Timeout(String),
}

impl ZjlibCnkiError {
    /// Return whether this error is a QR login timeout.
    ///
    /// # Returns
    ///
    /// True when the error is a timeout.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout(_))
    }
}

impl fmt::Display for ZjlibCnkiError {
    /// Format the client error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message) | Self::Parse(message) | Self::Timeout(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl Error for ZjlibCnkiError {}

fn observe_zjlib_operation<T>(
    endpoint: &'static str,
    attempt: usize,
    operation: impl FnOnce() -> Result<T, ZjlibCnkiError>,
) -> Result<T, ZjlibCnkiError> {
    let started_at = Instant::now();
    let result = operation();
    match &result {
        Ok(_) => tracing::debug!(
            event = "source.request.completed",
            component = "source",
            provider = "zjlib_cnki",
            endpoint,
            attempt,
            outcome = "success",
            http_status = 0,
            has_http_status = false,
            is_retry = attempt > 1,
            will_retry = false,
            duration_ms = elapsed_millis(started_at),
        ),
        Err(error) => tracing::warn!(
            event = "source.request.failed",
            component = "source",
            provider = "zjlib_cnki",
            endpoint,
            attempt,
            outcome = "failure",
            error_kind = zjlib_error_kind(error),
            http_status = 0,
            has_http_status = false,
            is_retry = attempt > 1,
            will_retry = false,
            duration_ms = elapsed_millis(started_at),
        ),
    }
    result
}

fn zjlib_error_kind(error: &ZjlibCnkiError) -> &'static str {
    match error {
        ZjlibCnkiError::Request(_) => "request",
        ZjlibCnkiError::Parse(_) => "parse",
        ZjlibCnkiError::Timeout(_) => "timeout",
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// QR login challenge returned by Zhejiang Library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZjlibCnkiQrLogin {
    /// QR UUID.
    pub uuid: String,
    /// Upstream QR status.
    pub status: String,
    /// QR code URL or payload.
    pub qr_code: String,
}

/// Article metadata required for exact CNKI full-text matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZjlibCnkiArticleIdentity {
    /// Article title.
    pub title: String,
    /// Semicolon-delimited article authors.
    pub authors: String,
    /// Journal title.
    pub journal_title: String,
}

/// One CNKI search result row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZjlibCnkiSearchResult {
    /// Result index in page order.
    pub index: usize,
    /// Result title.
    pub title: String,
    /// Detail page URL.
    pub detail_url: String,
    /// CNKI file name when present.
    pub file_name: Option<String>,
    /// CNKI database name when present.
    pub db_name: Option<String>,
    /// CNKI database code when present.
    pub db_code: Option<String>,
    /// Row-level download URL when present.
    pub download_url: Option<String>,
}

/// CNKI candidate metadata parsed before PDF download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZjlibCnkiArticleCandidate {
    /// Search result that produced this candidate.
    pub result: ZjlibCnkiSearchResult,
    /// Parsed article identity.
    pub identity: ZjlibCnkiArticleIdentity,
    /// Final detail page URL.
    pub detail_url: String,
    /// PDF download URL when present.
    pub pdf_url: Option<String>,
}

/// Downloaded CNKI PDF bytes and response metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZjlibCnkiDownloadedPdf {
    /// Download filename.
    pub filename: String,
    /// Final upstream URL.
    pub final_url: String,
    /// Upstream content type.
    pub content_type: String,
    /// PDF byte count.
    pub byte_count: usize,
    /// PDF content bytes.
    pub content: Vec<u8>,
}

/// JSON-serializable cookie state persisted with a CNKI session.
#[derive(Clone, PartialEq, Eq)]
pub struct ZjlibCnkiCookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Cookie domain.
    pub domain: String,
    /// Cookie path.
    pub path: String,
    /// Whether the cookie is secure-only.
    pub secure: bool,
    /// Optional Unix expiration timestamp.
    pub expires: Option<i64>,
    /// Whether the cookie should be discarded after the session.
    pub discard: bool,
}

impl fmt::Debug for ZjlibCnkiCookie {
    /// Format cookie metadata without exposing its value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ZjlibCnkiCookie")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .finish()
    }
}

impl ZjlibCnkiCookie {
    /// Build a persistent cookie value.
    ///
    /// # Arguments
    ///
    /// * `name` - Cookie name.
    /// * `value` - Cookie value.
    /// * `domain` - Cookie domain.
    ///
    /// # Returns
    ///
    /// Cookie state.
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".to_string(),
            secure: true,
            expires: None,
            discard: false,
        }
    }

    fn from_json(value: &Value) -> Option<Self> {
        let name = value.get("name").and_then(Value::as_str)?.trim();
        let cookie_value = value.get("value").and_then(Value::as_str)?.trim();
        if name.is_empty() {
            return None;
        }
        Some(Self {
            name: name.to_string(),
            value: cookie_value.to_string(),
            domain: value
                .get("domain")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            path: value
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .unwrap_or("/")
                .to_string(),
            secure: value.get("secure").and_then(Value::as_bool).unwrap_or(true),
            expires: value.get("expires").and_then(Value::as_i64),
            discard: value
                .get("discard")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "value": self.value,
            "domain": self.domain,
            "path": self.path,
            "secure": self.secure,
            "expires": self.expires,
            "discard": self.discard,
            "rest": {},
        })
    }

    fn is_unexpired(&self, now: i64) -> bool {
        self.expires.is_none_or(|expires| expires > now)
    }
}

/// Transport boundary used by the Zhejiang Library CNKI client.
pub trait ZjlibCnkiTransport {
    /// Start a QR login challenge.
    ///
    /// # Returns
    ///
    /// QR login challenge data.
    fn start_qr_login(&mut self) -> Result<ZjlibCnkiQrLogin, ZjlibCnkiError>;

    /// Poll QR login until completion or failure.
    ///
    /// # Arguments
    ///
    /// * `uuid` - QR UUID.
    /// * `timeout_seconds` - Maximum polling duration in seconds.
    /// * `interval_seconds` - Delay between status checks.
    ///
    /// # Returns
    ///
    /// Completed BFF user token.
    fn poll_qr_login(
        &mut self,
        uuid: &str,
        timeout_seconds: i64,
        interval_seconds: f64,
    ) -> Result<String, ZjlibCnkiError>;

    /// Add the BFF user token as the browser login cookie.
    ///
    /// # Arguments
    ///
    /// * `token` - BFF user token.
    fn set_login_cookie(&mut self, token: &str);

    /// Prepare Share and zyproxy cookies for full-text access.
    ///
    /// # Arguments
    ///
    /// * `token` - BFF user token.
    ///
    /// # Returns
    ///
    /// Final proxied CNKI URL.
    fn warm_up_fulltext_session(&mut self, token: &str) -> Result<String, ZjlibCnkiError>;

    /// Load persisted cookies into the transport.
    ///
    /// # Arguments
    ///
    /// * `cookies` - Persisted cookie state.
    fn load_cookies(&mut self, cookies: &[ZjlibCnkiCookie]);

    /// Snapshot transport cookies for persistence.
    ///
    /// # Returns
    ///
    /// Persistable cookies.
    fn cookies(&self) -> Vec<ZjlibCnkiCookie>;

    /// Return whether a cookie exists and has not expired.
    ///
    /// # Arguments
    ///
    /// * `name` - Cookie name.
    /// * `now` - Current Unix timestamp.
    ///
    /// # Returns
    ///
    /// True when the named cookie is usable.
    fn has_unexpired_cookie(&self, name: &str, now: i64) -> bool;

    /// Search CNKI through the current full-text session.
    ///
    /// # Arguments
    ///
    /// * `keyword` - Search keyword.
    /// * `limit` - Maximum result count.
    ///
    /// # Returns
    ///
    /// CNKI search results.
    fn search(
        &mut self,
        keyword: &str,
        limit: usize,
    ) -> Result<Vec<ZjlibCnkiSearchResult>, ZjlibCnkiError>;

    /// Inspect a search result detail page.
    ///
    /// # Arguments
    ///
    /// * `result` - Search result to inspect.
    ///
    /// # Returns
    ///
    /// Parsed candidate metadata.
    fn inspect_result_metadata(
        &mut self,
        result: &ZjlibCnkiSearchResult,
    ) -> Result<ZjlibCnkiArticleCandidate, ZjlibCnkiError>;

    /// Download one PDF URL.
    ///
    /// # Arguments
    ///
    /// * `pdf_url` - PDF URL.
    /// * `title` - Optional filename title.
    /// * `referer` - Optional referer URL.
    ///
    /// # Returns
    ///
    /// Downloaded PDF bytes and metadata.
    fn download_pdf(
        &mut self,
        pdf_url: &str,
        title: Option<&str>,
        referer: Option<&str>,
    ) -> Result<ZjlibCnkiDownloadedPdf, ZjlibCnkiError>;
}

/// Zhejiang Library CNKI client using a transport implementation.
#[derive(Clone)]
pub struct ZhejiangLibraryCnkiClient<T> {
    transport: T,
    bff_user_token: Option<String>,
    qr_uuid: Option<String>,
    fulltext_warmed_at: Option<i64>,
    final_zyproxy_url: Option<String>,
}

impl<T> fmt::Debug for ZhejiangLibraryCnkiClient<T> {
    /// Format client state without exposing tokens, cookies, or transport internals.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ZhejiangLibraryCnkiClient")
            .field("session", &"[REDACTED]")
            .field("is_configured", &self.bff_user_token.is_some())
            .field("fulltext_warmed_at", &self.fulltext_warmed_at)
            .finish_non_exhaustive()
    }
}

impl<T> ZhejiangLibraryCnkiClient<T>
where
    T: ZjlibCnkiTransport,
{
    /// Build a client from a transport.
    ///
    /// # Arguments
    ///
    /// * `transport` - HTTP or fixture transport.
    ///
    /// # Returns
    ///
    /// Client with empty state.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            bff_user_token: None,
            qr_uuid: None,
            fulltext_warmed_at: None,
            final_zyproxy_url: None,
        }
    }

    /// Build a client from a transport and persisted state JSON.
    ///
    /// # Arguments
    ///
    /// * `transport` - HTTP or fixture transport.
    /// * `state_data` - Persisted session state.
    ///
    /// # Returns
    ///
    /// Client initialized from persisted state.
    pub fn from_state_data(transport: T, state_data: &Value) -> Self {
        let mut client = Self::new(transport);
        client.load_state_data(state_data);
        client
    }

    /// Load token, QR UUID, warm-up state, and cookies from JSON state.
    ///
    /// # Arguments
    ///
    /// * `state_data` - Persisted session state.
    pub fn load_state_data(&mut self, state_data: &Value) {
        self.bff_user_token = text_field(state_data, "bff_user_token");
        self.qr_uuid = text_field(state_data, "qr_uuid");
        self.fulltext_warmed_at = state_data.get("fulltext_warmed_at").and_then(Value::as_i64);
        self.final_zyproxy_url = text_field(state_data, "final_zyproxy_url");
        let cookies = state_data
            .get("cookies")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(ZjlibCnkiCookie::from_json)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.transport.load_cookies(&cookies);
    }

    /// Return JSON-serializable session state for server-side persistence.
    ///
    /// # Returns
    ///
    /// Session state containing token, QR UUID, cookies, and timestamps.
    pub fn to_state_data(&self) -> Value {
        json!({
            "bff_user_token": self.bff_user_token,
            "qr_uuid": self.qr_uuid,
            "cookies": self.transport.cookies().iter().map(ZjlibCnkiCookie::to_json).collect::<Vec<_>>(),
            "fulltext_warmed_at": self.fulltext_warmed_at,
            "final_zyproxy_url": self.final_zyproxy_url,
            "saved_at": current_unix_time(),
        })
    }

    /// Start Zhejiang Library QR login.
    ///
    /// # Returns
    ///
    /// QR login challenge data.
    pub fn start_qr_login(&mut self) -> Result<ZjlibCnkiQrLogin, ZjlibCnkiError> {
        let login =
            observe_zjlib_operation("qr_login_start", 1, || self.transport.start_qr_login())?;
        self.qr_uuid = Some(login.uuid.clone());
        Ok(login)
    }

    /// Poll QR login until completion.
    ///
    /// # Arguments
    ///
    /// * `timeout_seconds` - Maximum polling duration in seconds.
    /// * `interval_seconds` - Delay between status checks.
    ///
    /// # Returns
    ///
    /// Completed BFF user token.
    pub fn poll_qr_login(
        &mut self,
        timeout_seconds: i64,
        interval_seconds: f64,
    ) -> Result<String, ZjlibCnkiError> {
        let qr_uuid = self
            .qr_uuid
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ZjlibCnkiError::Request("No QR uuid available. Run start-login first.".to_string())
            })?;
        let token = observe_zjlib_operation("qr_login_poll", 1, || {
            self.transport
                .poll_qr_login(qr_uuid, timeout_seconds, interval_seconds)
        })?;
        self.bff_user_token = Some(token.clone());
        self.transport.set_login_cookie(&token);
        Ok(token)
    }

    /// Prepare Share and zyproxy cookies required for CNKI full-text access.
    ///
    /// # Returns
    ///
    /// Final proxied CNKI entry URL.
    pub fn warm_up_fulltext_session(&mut self) -> Result<String, ZjlibCnkiError> {
        if self.has_fresh_fulltext_session(None) {
            tracing::debug!(
                event = "source.session.reused",
                component = "source",
                provider = "zjlib_cnki",
                endpoint = "fulltext_session",
            );
            return Ok(self
                .final_zyproxy_url
                .clone()
                .unwrap_or_else(|| format!("{ZYPROXY_BASE_URL}/kns55/")));
        }
        let token = self.ensure_logged_in()?;
        self.transport.set_login_cookie(&token);
        let final_url = observe_zjlib_operation("fulltext_session", 1, || {
            self.transport.warm_up_fulltext_session(&token)
        })?;
        self.fulltext_warmed_at = Some(current_unix_time());
        self.final_zyproxy_url = Some(final_url.clone());
        Ok(final_url)
    }

    /// Check whether the full-text session is still fresh.
    ///
    /// # Arguments
    ///
    /// * `now` - Optional Unix timestamp override.
    ///
    /// # Returns
    ///
    /// True when the current full-text cookies can be reused.
    pub fn has_fresh_fulltext_session(&self, now: Option<i64>) -> bool {
        let Some(warmed_at) = self.fulltext_warmed_at else {
            return false;
        };
        let current_time = now.unwrap_or_else(current_unix_time);
        let elapsed_seconds = current_time - warmed_at;
        if !(0..FULLTEXT_WARM_UP_TTL_SECONDS).contains(&elapsed_seconds) {
            return false;
        }
        if self
            .bff_user_token
            .as_deref()
            .and_then(jwt_expiration)
            .is_some_and(|expires_at| expires_at <= current_time + TOKEN_EXPIRY_SKEW_SECONDS)
        {
            return false;
        }
        self.transport
            .has_unexpired_cookie("vpn358_sid", current_time)
    }

    /// Search by title and download only an exact-matching article PDF.
    ///
    /// # Arguments
    ///
    /// * `expected` - Expected article metadata.
    /// * `result_limit` - Maximum search results to inspect.
    ///
    /// # Returns
    ///
    /// Downloaded matching PDF.
    pub fn download_matching_pdf(
        &mut self,
        expected: &ZjlibCnkiArticleIdentity,
        result_limit: usize,
    ) -> Result<ZjlibCnkiDownloadedPdf, ZjlibCnkiError> {
        let started_at = Instant::now();
        let results = observe_zjlib_operation("fulltext_search", 1, || {
            self.transport.search(&expected.title, result_limit)
        })?;
        let mut errors = Vec::new();
        for (candidate_index, result) in results.into_iter().enumerate() {
            let attempt = candidate_index + 1;
            let candidate = observe_zjlib_operation("fulltext_metadata", attempt, || {
                self.transport.inspect_result_metadata(&result)
            })?;
            if !does_article_metadata_match(expected, &candidate.identity) {
                tracing::debug!(
                    event = "source.fallback.activated",
                    component = "source",
                    provider = "zjlib_cnki",
                    endpoint = "fulltext_match",
                    reason = "metadata_mismatch",
                    fallback = "next_candidate",
                    attempt,
                );
                errors.push(format!("{}: metadata mismatch", result.index));
                continue;
            }
            let Some(pdf_url) = candidate.pdf_url.as_deref() else {
                tracing::debug!(
                    event = "source.fallback.activated",
                    component = "source",
                    provider = "zjlib_cnki",
                    endpoint = "fulltext_match",
                    reason = "pdf_link_missing",
                    fallback = "next_candidate",
                    attempt,
                );
                errors.push(format!("{}: PDF link missing", result.index));
                continue;
            };
            let download = observe_zjlib_operation("fulltext_download", attempt, || {
                self.transport.download_pdf(
                    pdf_url,
                    Some(&candidate.identity.title),
                    Some(&candidate.detail_url),
                )
            })?;
            tracing::info!(
                event = "source.fulltext.completed",
                component = "source",
                provider = "zjlib_cnki",
                outcome = "success",
                inspected_candidate_count = attempt,
                byte_count = download.content.len(),
                duration_ms = elapsed_millis(started_at),
            );
            return Ok(download);
        }
        tracing::warn!(
            event = "source.fulltext.failed",
            component = "source",
            provider = "zjlib_cnki",
            outcome = "failure",
            error_kind = "no_exact_match",
            inspected_candidate_count = errors.len(),
            duration_ms = elapsed_millis(started_at),
        );
        let detail = if errors.is_empty() {
            "no search results".to_string()
        } else {
            errors.join(" | ")
        };
        Err(ZjlibCnkiError::Request(format!(
            "No exact CNKI full-text match found: {detail}"
        )))
    }

    fn ensure_logged_in(&mut self) -> Result<String, ZjlibCnkiError> {
        if let Some(token) = self.bff_user_token.clone() {
            if jwt_expiration(&token).is_some_and(|expires_at| {
                expires_at <= current_unix_time() + TOKEN_EXPIRY_SKEW_SECONDS
            }) {
                return Err(ZjlibCnkiError::Request(
                    "bff-user-token is expired or expires soon. Run QR login again.".to_string(),
                ));
            }
            return Ok(token);
        }
        self.poll_qr_login(180, 2.0)
    }
}

/// Live Zhejiang Library CNKI transport configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveZjlibCnkiConfig {
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// Maximum full-text response bytes retained in memory.
    pub maximum_document_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveZjlibCnkiEndpoints {
    www_base_url: Url,
    share_base_url: Url,
    zyproxy_login_base_url: Url,
    zyproxy_base_url: Url,
    entry_url: Url,
    library_refer: Url,
}

impl Default for LiveZjlibCnkiEndpoints {
    fn default() -> Self {
        Self {
            www_base_url: Url::parse(WWW_BASE_URL).expect("WWW base URL should be valid"),
            share_base_url: Url::parse(SHARE_BASE_URL).expect("Share base URL should be valid"),
            zyproxy_login_base_url: Url::parse(&format!("https://{ZYPROXY_LOGIN_HOST}"))
                .expect("zyproxy login base URL should be valid"),
            zyproxy_base_url: Url::parse(ZYPROXY_BASE_URL)
                .expect("zyproxy base URL should be valid"),
            entry_url: Url::parse(ENTRY_URL).expect("Share entry URL should be valid"),
            library_refer: Url::parse(LIBRARY_REFER).expect("library refer URL should be valid"),
        }
    }
}

impl LiveZjlibCnkiEndpoints {
    fn append(base_url: &Url, path: &str) -> String {
        format!("{}{}", base_url.as_str().trim_end_matches('/'), path)
    }

    fn origin(base_url: &Url) -> String {
        let mut origin = base_url.clone();
        origin.set_path("");
        origin.set_query(None);
        origin.set_fragment(None);
        origin.as_str().trim_end_matches('/').to_string()
    }

    #[cfg(test)]
    fn loopback(base_url: &str) -> Result<Self, ZjlibCnkiError> {
        let base_url = Url::parse(base_url).map_err(|_| {
            ZjlibCnkiError::Parse("loopback fixture base URL was invalid.".to_string())
        })?;
        if base_url.scheme() != "http"
            || !matches!(base_url.host_str(), Some("127.0.0.1" | "localhost"))
            || !base_url.username().is_empty()
            || base_url.password().is_some()
        {
            return Err(ZjlibCnkiError::Parse(
                "loopback fixture base URL did not use a local HTTP origin.".to_string(),
            ));
        }
        let endpoint = |path: &str| {
            let mut url = base_url.clone();
            url.set_path(path);
            url.set_query(None);
            url.set_fragment(None);
            url
        };
        Ok(Self {
            www_base_url: endpoint("/www"),
            share_base_url: endpoint("/share"),
            zyproxy_login_base_url: endpoint("/login"),
            zyproxy_base_url: endpoint("/proxy"),
            entry_url: endpoint("/share/entry/area/35594/2120"),
            library_refer: endpoint("/proxy/kns55/"),
        })
    }
}

impl Default for LiveZjlibCnkiConfig {
    /// Build default live transport configuration.
    ///
    /// # Returns
    ///
    /// Default live configuration.
    fn default() -> Self {
        Self {
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            maximum_document_bytes: DEFAULT_FULL_TEXT_MAXIMUM_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZyproxyHost {
    Login,
    Proxy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ZyproxyEndpoint {
    host: ZyproxyHost,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ZyproxyRedirectAction {
    Follow {
        next_url: Url,
        current_endpoint: ZyproxyEndpoint,
    },
    Retry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ZyproxyEntryOutcome {
    Ready(String),
    Retry,
}

fn retry_zyproxy_login<Attempt, Pause>(
    mut attempt: Attempt,
    mut pause: Pause,
) -> Result<String, ZjlibCnkiError>
where
    Attempt: FnMut() -> Result<ZyproxyEntryOutcome, ZjlibCnkiError>,
    Pause: FnMut(Duration),
{
    for attempt_index in 0..ZYPROXY_LOGIN_ATTEMPTS {
        let attempt_number = attempt_index + 1;
        let started_at = Instant::now();
        match attempt() {
            Ok(ZyproxyEntryOutcome::Ready(final_url)) => return Ok(final_url),
            Ok(ZyproxyEntryOutcome::Retry) if attempt_number < ZYPROXY_LOGIN_ATTEMPTS => {
                let delay_ms = ZYPROXY_RETRY_DELAY_MILLIS * attempt_number as u64;
                tracing::warn!(
                    event = "source.request.failed",
                    component = "source",
                    provider = "zjlib_cnki",
                    endpoint = "zyproxy_login",
                    attempt = attempt_number,
                    outcome = "failure",
                    error_kind = "session_not_ready",
                    http_status = 0,
                    has_http_status = false,
                    is_retry = attempt_index > 0,
                    will_retry = true,
                    retry_delay_ms = delay_ms,
                    duration_ms = elapsed_millis(started_at),
                );
                pause(Duration::from_millis(delay_ms));
            }
            Ok(ZyproxyEntryOutcome::Retry) => {
                tracing::warn!(
                    event = "source.request.failed",
                    component = "source",
                    provider = "zjlib_cnki",
                    endpoint = "zyproxy_login",
                    attempt = attempt_number,
                    outcome = "failure",
                    error_kind = "session_not_ready",
                    http_status = 0,
                    has_http_status = false,
                    is_retry = attempt_index > 0,
                    will_retry = false,
                    duration_ms = elapsed_millis(started_at),
                );
                return Err(ZjlibCnkiError::Request(format!(
                    "zyproxy session was not accepted after {ZYPROXY_LOGIN_ATTEMPTS} login attempts."
                )));
            }
            Err(error) => {
                tracing::warn!(
                    event = "source.request.failed",
                    component = "source",
                    provider = "zjlib_cnki",
                    endpoint = "zyproxy_login",
                    attempt = attempt_number,
                    outcome = "failure",
                    error_kind = zjlib_error_kind(&error),
                    http_status = 0,
                    has_http_status = false,
                    is_retry = attempt_index > 0,
                    will_retry = false,
                    duration_ms = elapsed_millis(started_at),
                );
                return Err(error);
            }
        }
    }
    Err(ZjlibCnkiError::Request(
        "zyproxy login attempt budget was exhausted.".to_string(),
    ))
}

#[cfg(test)]
fn zyproxy_endpoint(url: &Url) -> Result<ZyproxyEndpoint, ZjlibCnkiError> {
    zyproxy_endpoint_for(url, &LiveZjlibCnkiEndpoints::default())
}

fn zyproxy_endpoint_for(
    url: &Url,
    endpoints: &LiveZjlibCnkiEndpoints,
) -> Result<ZyproxyEndpoint, ZjlibCnkiError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ZjlibCnkiError::Parse(
            "zyproxy redirect used an unexpected origin.".to_string(),
        ));
    }
    let (host, base_url) = if has_endpoint_origin_and_path(url, &endpoints.zyproxy_login_base_url) {
        (ZyproxyHost::Login, &endpoints.zyproxy_login_base_url)
    } else if has_endpoint_origin_and_path(url, &endpoints.zyproxy_base_url) {
        (ZyproxyHost::Proxy, &endpoints.zyproxy_base_url)
    } else {
        return Err(ZjlibCnkiError::Parse(
            "zyproxy redirect used an unexpected endpoint.".to_string(),
        ));
    };
    let base_path = base_url.path().trim_end_matches('/');
    let path = url
        .path()
        .strip_prefix(base_path)
        .unwrap_or(url.path())
        .trim_end_matches('/');
    Ok(ZyproxyEndpoint {
        host,
        path: if path.is_empty() { "/" } else { path }.to_ascii_lowercase(),
    })
}

fn has_endpoint_origin_and_path(url: &Url, base_url: &Url) -> bool {
    if url.scheme() != base_url.scheme()
        || url.host_str() != base_url.host_str()
        || url.port_or_known_default() != base_url.port_or_known_default()
    {
        return false;
    }
    let base_path = base_url.path().trim_end_matches('/');
    base_path.is_empty()
        || url.path() == base_path
        || url
            .path()
            .strip_prefix(base_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
fn zyproxy_redirect_action(
    history: &[ZyproxyEndpoint],
    response_url: &Url,
    location: Option<&str>,
    redirect_hops: usize,
) -> Result<ZyproxyRedirectAction, ZjlibCnkiError> {
    zyproxy_redirect_action_for(
        history,
        response_url,
        location,
        redirect_hops,
        &LiveZjlibCnkiEndpoints::default(),
    )
}

fn zyproxy_redirect_action_for(
    history: &[ZyproxyEndpoint],
    response_url: &Url,
    location: Option<&str>,
    redirect_hops: usize,
    endpoints: &LiveZjlibCnkiEndpoints,
) -> Result<ZyproxyRedirectAction, ZjlibCnkiError> {
    if redirect_hops >= ZYPROXY_REDIRECT_HOPS {
        return Err(ZjlibCnkiError::Request(format!(
            "zyproxy login exceeded {ZYPROXY_REDIRECT_HOPS} redirect hops."
        )));
    }
    let location = location.ok_or_else(|| {
        ZjlibCnkiError::Parse("zyproxy redirect did not contain a valid Location.".to_string())
    })?;
    let next_url = response_url
        .join(location)
        .map_err(|_| ZjlibCnkiError::Parse("zyproxy redirect Location was invalid.".to_string()))?;
    let current_endpoint = zyproxy_endpoint_for(response_url, endpoints)?;
    let next_endpoint = zyproxy_endpoint_for(&next_url, endpoints)?;
    if current_endpoint == next_endpoint {
        return Err(ZjlibCnkiError::Request(
            "zyproxy returned an unexpected self-redirect.".to_string(),
        ));
    }
    if history.iter().any(|endpoint| endpoint == &next_endpoint) {
        if history.last() == Some(&next_endpoint)
            && is_expected_zyproxy_loop_edge(&current_endpoint, &next_endpoint)
        {
            return Ok(ZyproxyRedirectAction::Retry);
        }
        return Err(ZjlibCnkiError::Request(
            "zyproxy returned an unexpected redirect cycle.".to_string(),
        ));
    }
    Ok(ZyproxyRedirectAction::Follow {
        next_url,
        current_endpoint,
    })
}

fn is_expected_zyproxy_loop_edge(current: &ZyproxyEndpoint, next: &ZyproxyEndpoint) -> bool {
    (is_zyproxy_login_index(current) && is_zyproxy_proxy_entry(next))
        || (is_zyproxy_proxy_entry(current) && is_zyproxy_login_index(next))
}

fn is_zyproxy_login_index(endpoint: &ZyproxyEndpoint) -> bool {
    endpoint.host == ZyproxyHost::Login && endpoint.path == "/index.php"
}

fn is_zyproxy_proxy_entry(endpoint: &ZyproxyEndpoint) -> bool {
    endpoint.host == ZyproxyHost::Proxy && endpoint.path == "/kns55"
}

#[cfg(test)]
fn validate_zyproxy_success(url: &Url, has_session_cookie: bool) -> Result<String, ZjlibCnkiError> {
    validate_zyproxy_success_for(url, has_session_cookie, &LiveZjlibCnkiEndpoints::default())
}

fn validate_zyproxy_success_for(
    url: &Url,
    has_session_cookie: bool,
    endpoints: &LiveZjlibCnkiEndpoints,
) -> Result<String, ZjlibCnkiError> {
    if zyproxy_endpoint_for(url, endpoints)?.host != ZyproxyHost::Proxy {
        return Err(ZjlibCnkiError::Parse(
            "zyproxy login ended outside the proxy host.".to_string(),
        ));
    }
    if !has_session_cookie {
        return Err(ZjlibCnkiError::Parse(
            "zyproxy login did not set vpn358_sid.".to_string(),
        ));
    }
    Ok(url.to_string())
}

/// Blocking HTTP transport for live Zhejiang Library CNKI login.
#[derive(Clone)]
pub struct LiveZjlibCnkiTransport {
    redirect_client: Client,
    no_redirect_client: Client,
    cookie_jar: Arc<Jar>,
    endpoints: LiveZjlibCnkiEndpoints,
    maximum_document_bytes: usize,
    last_brief_url: Option<String>,
}

impl fmt::Debug for LiveZjlibCnkiTransport {
    /// Format live transport state without exposing its cookie jar.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveZjlibCnkiTransport")
            .field("session", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl LiveZjlibCnkiTransport {
    /// Build a live transport.
    ///
    /// # Arguments
    ///
    /// * `config` - Live transport configuration.
    ///
    /// # Returns
    ///
    /// Live transport.
    pub fn new(config: LiveZjlibCnkiConfig) -> Result<Self, ZjlibCnkiError> {
        Self::new_with_endpoints(config, LiveZjlibCnkiEndpoints::default())
    }

    fn new_with_endpoints(
        config: LiveZjlibCnkiConfig,
        endpoints: LiveZjlibCnkiEndpoints,
    ) -> Result<Self, ZjlibCnkiError> {
        let cookie_jar = Arc::new(Jar::default());
        let timeout = Duration::from_secs(config.timeout_seconds.max(1));
        let redirect_client = Client::builder()
            .timeout(timeout)
            .cookie_provider(cookie_jar.clone())
            .redirect(Policy::limited(10))
            .build()
            .map_err(request_error)?;
        let no_redirect_client = Client::builder()
            .timeout(timeout)
            .cookie_provider(cookie_jar.clone())
            .redirect(Policy::none())
            .build()
            .map_err(request_error)?;
        Ok(Self {
            redirect_client,
            no_redirect_client,
            cookie_jar,
            endpoints,
            maximum_document_bytes: config.maximum_document_bytes.max(1),
            last_brief_url: None,
        })
    }

    #[cfg(test)]
    fn new_for_loopback(
        config: LiveZjlibCnkiConfig,
        base_url: &str,
    ) -> Result<Self, ZjlibCnkiError> {
        Self::new_with_endpoints(config, LiveZjlibCnkiEndpoints::loopback(base_url)?)
    }

    fn build_share_sso_url(&mut self, token: &str) -> Result<String, ZjlibCnkiError> {
        let response = self
            .no_redirect_client
            .get(LiveZjlibCnkiEndpoints::append(
                &self.endpoints.www_base_url,
                "/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl",
            ))
            .query(&[("referURL", self.endpoints.entry_url.as_str())])
            .headers(www_headers(&self.endpoints.www_base_url, Some(token)))
            .send()
            .map_err(request_error)?;
        let payload = json_payload(response, "build Share SSO URL")?;
        let sso_url = payload
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let parsed_sso_url = Url::parse(&sso_url).map_err(|_| {
            ZjlibCnkiError::Parse("Share SSO URL response was invalid.".to_string())
        })?;
        if !has_endpoint_origin_and_path(&parsed_sso_url, &self.endpoints.share_base_url) {
            return Err(ZjlibCnkiError::Parse(
                "Share SSO URL response did not contain a share.zjlib.cn URL.".to_string(),
            ));
        }
        Ok(sso_url)
    }

    fn enter_share(&mut self, sso_url: &str) -> Result<(), ZjlibCnkiError> {
        let response = self
            .no_redirect_client
            .get(sso_url)
            .headers(html_headers(Some(self.endpoints.www_base_url.as_str())))
            .send()
            .map_err(request_error)?;
        let response = raise_for_status(response, "enter Share protocolAuth")?;
        let response_url = response.url().to_string();
        let text = response.text().map_err(request_error)?;
        if let Some((sync_url, data)) = extract_share_cookie_sync(&text) {
            let response = self
                .redirect_client
                .post(sync_url)
                .form(&data)
                .headers(html_headers(Some(&response_url)))
                .header(
                    ORIGIN,
                    LiveZjlibCnkiEndpoints::origin(&self.endpoints.share_base_url),
                )
                .send()
                .map_err(request_error)?;
            raise_for_status(response, "sync Share login cookies")?;
        }
        let response = self
            .redirect_client
            .get(self.endpoints.entry_url.clone())
            .headers(html_headers(Some(sso_url)))
            .send()
            .map_err(request_error)?;
        raise_for_status(response, "open Share entry")?;
        let response = self
            .redirect_client
            .get(LiveZjlibCnkiEndpoints::append(
                &self.endpoints.share_base_url,
                "/engine2/header/user-info",
            ))
            .query(&[("t", current_millis().to_string())])
            .headers(ajax_headers(Some(self.endpoints.entry_url.as_str())))
            .send()
            .map_err(request_error)?;
        raise_for_status(response, "load Share user info")?;
        Ok(())
    }

    fn get_zyproxy_login_url(&mut self) -> Result<String, ZjlibCnkiError> {
        let response = self
            .no_redirect_client
            .get(LiveZjlibCnkiEndpoints::append(
                &self.endpoints.share_base_url,
                "/sso/api/auth/library/vpn358",
            ))
            .query(&[
                ("wfwfid", WFWFID),
                ("refer", self.endpoints.library_refer.as_str()),
            ])
            .headers(html_headers(Some(self.endpoints.entry_url.as_str())))
            .send()
            .map_err(request_error)?;
        let response = raise_for_status(response, "get zyproxy login URL")?;
        let response_url = response.url().to_string();
        if let Some(location) = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
        {
            return join_url(&response_url, location);
        }
        let text = response.text().map_err(request_error)?;
        let login_url = extract_window_location(&text, &response_url)?;
        let parsed_login_url = Url::parse(&login_url).map_err(|_| {
            ZjlibCnkiError::Parse("Share library auth returned an invalid login URL.".to_string())
        })?;
        if zyproxy_endpoint_for(&parsed_login_url, &self.endpoints)?.host != ZyproxyHost::Login {
            return Err(ZjlibCnkiError::Parse(
                "Share library auth did not return login.elib redirect.".to_string(),
            ));
        }
        Ok(login_url)
    }

    fn enter_zyproxy(&mut self, login_url: &str) -> Result<ZyproxyEntryOutcome, ZjlibCnkiError> {
        let mut current_url = Url::parse(login_url)
            .map_err(|_| ZjlibCnkiError::Parse("zyproxy login URL was invalid.".to_string()))?;
        zyproxy_endpoint_for(&current_url, &self.endpoints)?;
        let mut history = Vec::new();
        let mut redirect_hops = 0;
        let mut referer = self.endpoints.share_base_url.to_string();
        loop {
            let response = self
                .no_redirect_client
                .get(current_url)
                .headers(html_headers(Some(&referer)))
                .send()
                .map_err(request_error)?;
            let response_url = response.url().clone();
            if response.status().is_success() {
                let has_session_cookie =
                    self.has_unexpired_cookie("vpn358_sid", current_unix_time());
                return validate_zyproxy_success_for(
                    &response_url,
                    has_session_cookie,
                    &self.endpoints,
                )
                .map(ZyproxyEntryOutcome::Ready);
            }
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok());
                match zyproxy_redirect_action_for(
                    &history,
                    &response_url,
                    location,
                    redirect_hops,
                    &self.endpoints,
                )? {
                    ZyproxyRedirectAction::Follow {
                        next_url,
                        current_endpoint,
                    } => {
                        history.push(current_endpoint);
                        redirect_hops += 1;
                        referer = response_url.to_string();
                        current_url = next_url;
                    }
                    ZyproxyRedirectAction::Retry => return Ok(ZyproxyEntryOutcome::Retry),
                }
                continue;
            }
            return Err(ZjlibCnkiError::Request(format!(
                "enter zyproxy failed with HTTP {}.",
                response.status().as_u16()
            )));
        }
    }

    fn post_form_text(
        &mut self,
        url: &str,
        form: &[(String, String)],
        headers: HeaderMap,
        action: &str,
    ) -> Result<String, ZjlibCnkiError> {
        let response = self
            .redirect_client
            .post(url)
            .headers(headers)
            .form(form)
            .send()
            .map_err(request_error)?;
        let response = raise_for_status(response, action)?;
        response.text().map_err(request_error)
    }
}

impl Default for LiveZjlibCnkiTransport {
    /// Build a live transport with default configuration.
    ///
    /// # Returns
    ///
    /// Live transport.
    fn default() -> Self {
        Self::new(LiveZjlibCnkiConfig::default())
            .expect("default Zhejiang Library CNKI transport should build")
    }
}

impl ZjlibCnkiTransport for LiveZjlibCnkiTransport {
    /// Start a live QR login challenge.
    fn start_qr_login(&mut self) -> Result<ZjlibCnkiQrLogin, ZjlibCnkiError> {
        let response = self
            .no_redirect_client
            .get(LiveZjlibCnkiEndpoints::append(
                &self.endpoints.www_base_url,
                "/bff-api/reader-sso-service/portal-pc-api/login/zfb-qr",
            ))
            .headers(www_headers(&self.endpoints.www_base_url, None))
            .send()
            .map_err(request_error)?;
        let payload = json_payload(response, "start QR login")?;
        let data = payload_data(&payload, "start QR login")?;
        let uuid = data
            .get("uuid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let qr_code = data
            .get("qrCode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let status = data
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if uuid.is_empty() || qr_code.is_empty() {
            return Err(ZjlibCnkiError::Parse(
                "QR login response did not contain uuid/qrCode.".to_string(),
            ));
        }
        Ok(ZjlibCnkiQrLogin {
            uuid,
            status,
            qr_code,
        })
    }

    /// Poll a live QR login challenge.
    fn poll_qr_login(
        &mut self,
        uuid: &str,
        timeout_seconds: i64,
        interval_seconds: f64,
    ) -> Result<String, ZjlibCnkiError> {
        let timeout = Duration::from_secs(timeout_seconds.max(1) as u64);
        let interval = Duration::from_secs_f64(interval_seconds.clamp(0.1, 10.0));
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let response = self
                .no_redirect_client
                .get(LiveZjlibCnkiEndpoints::append(
                    &self.endpoints.www_base_url,
                    "/bff-api/reader-sso-service/portal-pc-api/qr/status",
                ))
                .query(&[("uuid", uuid)])
                .headers(www_headers(&self.endpoints.www_base_url, None))
                .send()
                .map_err(request_error)?;
            let payload = json_payload(response, "poll QR login")?;
            let data = payload_data(&payload, "poll QR login")?;
            let status = data
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if status == "COMPLETE" {
                let token = data
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if token.is_empty() {
                    return Err(ZjlibCnkiError::Parse(
                        "QR login completed but did not return token.".to_string(),
                    ));
                }
                return Ok(token);
            }
            if matches!(
                status,
                "EXPIRED" | "CANCEL" | "CANCELED" | "FAIL" | "FAILED"
            ) {
                return Err(ZjlibCnkiError::Request(format!(
                    "QR login ended with status {status}."
                )));
            }
            thread::sleep(interval);
        }
        Err(ZjlibCnkiError::Timeout(format!(
            "Timed out waiting for QR scan after {timeout_seconds} seconds."
        )))
    }

    /// Add the BFF user token as the browser login cookie.
    fn set_login_cookie(&mut self, token: &str) {
        let secure = (self.endpoints.www_base_url.scheme() == "https").then_some("; Secure");
        self.cookie_jar.add_cookie_str(
            &format!("userToken={token}; Path=/{}", secure.unwrap_or_default()),
            &self.endpoints.www_base_url,
        );
    }

    /// Prepare live Share and zyproxy cookies.
    fn warm_up_fulltext_session(&mut self, token: &str) -> Result<String, ZjlibCnkiError> {
        let sso_url = self.build_share_sso_url(token)?;
        self.enter_share(&sso_url)?;
        retry_zyproxy_login(
            || {
                let login_url = self.get_zyproxy_login_url()?;
                self.enter_zyproxy(&login_url)
            },
            thread::sleep,
        )
    }

    /// Load persisted cookies into the live cookie jar.
    fn load_cookies(&mut self, cookies: &[ZjlibCnkiCookie]) {
        for cookie in cookies {
            if cookie.name.trim().is_empty() {
                continue;
            }
            let Some(url) = cookie_url(cookie, &self.endpoints) else {
                continue;
            };
            self.cookie_jar.add_cookie_str(&cookie_string(cookie), &url);
        }
    }

    /// Snapshot live cookies from known Zhejiang Library domains.
    fn cookies(&self) -> Vec<ZjlibCnkiCookie> {
        let mut cookies = BTreeMap::new();
        for url in known_cookie_urls(&self.endpoints) {
            let Some(header_value) = self.cookie_jar.cookies(&url) else {
                continue;
            };
            let Ok(header) = header_value.to_str() else {
                continue;
            };
            let domain = url.host_str().unwrap_or_default().to_string();
            for (name, value) in cookie_header_pairs(header) {
                cookies.insert(
                    name.clone(),
                    ZjlibCnkiCookie::new(name, value, domain.clone()),
                );
            }
        }
        cookies.into_values().collect()
    }

    /// Check whether a live cookie exists.
    fn has_unexpired_cookie(&self, name: &str, _now: i64) -> bool {
        self.cookies().iter().any(|cookie| cookie.name == name)
    }

    /// Search live CNKI through zyproxy.
    fn search(
        &mut self,
        keyword: &str,
        limit: usize,
    ) -> Result<Vec<ZjlibCnkiSearchResult>, ZjlibCnkiError> {
        let result_url = LiveZjlibCnkiEndpoints::append(
            &self.endpoints.zyproxy_base_url,
            "/kns55/brief/result.aspx",
        );
        let handler_url = LiveZjlibCnkiEndpoints::append(
            &self.endpoints.zyproxy_base_url,
            "/kns55/request/SearchHandler.ashx",
        );
        let brief_url = LiveZjlibCnkiEndpoints::append(
            &self.endpoints.zyproxy_base_url,
            "/kns55/brief/brief.aspx",
        );
        let proxy_entry =
            LiveZjlibCnkiEndpoints::append(&self.endpoints.zyproxy_base_url, "/kns55/");
        let post_headers = cnki_form_headers(
            &proxy_entry,
            &LiveZjlibCnkiEndpoints::origin(&self.endpoints.zyproxy_base_url),
        );
        let result_fields = search_result_form_fields(keyword);
        self.post_form_text(
            &result_url,
            &result_fields,
            post_headers.clone(),
            "post CNKI result.aspx",
        )?;
        let mut handler_headers = post_headers;
        handler_headers.insert(
            REFERER,
            HeaderValue::from_str(&result_url)
                .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?,
        );
        handler_headers.insert(
            "X-Requested-With",
            HeaderValue::from_static("XMLHttpRequest"),
        );
        let handler_fields = search_handler_form_fields(keyword);
        self.post_form_text(
            &handler_url,
            &handler_fields,
            handler_headers,
            "post CNKI SearchHandler",
        )?;
        let brief_query = vec![
            ("pagename", "ASP.brief_result_aspx".to_string()),
            ("dbPrefix", "SCDB".to_string()),
            ("dbCatalog", "中国学术文献网络出版总库".to_string()),
            ("ConfigFile", "SCDB.xml".to_string()),
            ("research", "off".to_string()),
            ("t", current_millis().to_string()),
        ];
        let response = self
            .redirect_client
            .get(&brief_url)
            .query(&brief_query)
            .headers(html_headers(Some(&result_url)))
            .send()
            .map_err(request_error)?;
        let response = raise_for_status(response, "get CNKI brief results")?;
        let final_url = response.url().to_string();
        let text = response.text().map_err(request_error)?;
        self.last_brief_url = Some(final_url.clone());
        Ok(parse_search_results(&text, &final_url)
            .into_iter()
            .take(limit)
            .collect())
    }

    /// Inspect a live CNKI result detail page.
    fn inspect_result_metadata(
        &mut self,
        result: &ZjlibCnkiSearchResult,
    ) -> Result<ZjlibCnkiArticleCandidate, ZjlibCnkiError> {
        let referer = self.last_brief_url.clone().unwrap_or_else(|| {
            LiveZjlibCnkiEndpoints::append(&self.endpoints.zyproxy_base_url, "/kns55/")
        });
        let response = self
            .redirect_client
            .get(&result.detail_url)
            .headers(html_headers(Some(&referer)))
            .send()
            .map_err(request_error)?;
        let response = raise_for_status(response, "open CNKI detail")?;
        let detail_url = response.url().to_string();
        let text = response.text().map_err(request_error)?;
        let identity = extract_article_identity(&text, &result.title);
        let pdf_url = extract_pdf_download_url(&text, &detail_url);
        Ok(ZjlibCnkiArticleCandidate {
            result: result.clone(),
            identity,
            detail_url,
            pdf_url,
        })
    }

    /// Download a live CNKI PDF.
    fn download_pdf(
        &mut self,
        pdf_url: &str,
        title: Option<&str>,
        referer: Option<&str>,
    ) -> Result<ZjlibCnkiDownloadedPdf, ZjlibCnkiError> {
        let default_referer =
            LiveZjlibCnkiEndpoints::append(&self.endpoints.zyproxy_base_url, "/kns55/");
        let response = self
            .redirect_client
            .get(pdf_url)
            .headers(html_headers(Some(referer.unwrap_or(&default_referer))))
            .send()
            .map_err(request_error)?;
        let response = raise_for_status(response, "download PDF")?;
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/pdf")
            .to_string();
        if response.content_length().is_some_and(|length| {
            length > u64::try_from(self.maximum_document_bytes).unwrap_or(u64::MAX)
        }) {
            return Err(ZjlibCnkiError::Request(
                "Download endpoint exceeded the configured document size limit.".to_string(),
            ));
        }
        let mut content = Vec::new();
        response
            .take(self.maximum_document_bytes as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        if content.len() > self.maximum_document_bytes {
            return Err(ZjlibCnkiError::Request(
                "Download endpoint exceeded the configured document size limit.".to_string(),
            ));
        }
        if !content_type.to_ascii_lowercase().contains("pdf") && !content.starts_with(b"%PDF") {
            return Err(ZjlibCnkiError::Request(format!(
                "Download endpoint did not return PDF (content-type={content_type:?}, url={}).",
                redact_url(&final_url)
            )));
        }
        let resolved_title = title
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| title_from_pdf_url(&final_url))
            .unwrap_or_else(|| "cnki".to_string());
        let filename_stem = safe_filename(&resolved_title);
        let filename = format!("{filename_stem}.pdf");
        Ok(ZjlibCnkiDownloadedPdf {
            filename,
            final_url,
            content_type,
            byte_count: content.len(),
            content,
        })
    }
}

/// Deterministic fixture behavior for Zhejiang Library CNKI tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureZjlibCnkiMode {
    /// Start, poll, and warm-up all succeed.
    Success,
    /// QR start fails.
    StartFailure,
    /// QR polling times out.
    PollTimeout,
    /// QR polling fails with a non-timeout login status.
    PollFailure,
    /// Full-text warm-up fails after login completion.
    WarmupFailure,
    /// Full-text search returns only mismatching metadata.
    FulltextMismatch,
    /// Full-text PDF download fails after metadata matches.
    FulltextFailure,
}

impl FixtureZjlibCnkiMode {
    /// Parse a fixture mode string.
    ///
    /// # Arguments
    ///
    /// * `value` - Fixture mode value.
    ///
    /// # Returns
    ///
    /// Parsed fixture mode.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "success" | "live_success" => Some(Self::Success),
            "start_failure" => Some(Self::StartFailure),
            "timeout" | "poll_timeout" => Some(Self::PollTimeout),
            "poll_failure" => Some(Self::PollFailure),
            "warmup_failure" => Some(Self::WarmupFailure),
            "fulltext_mismatch" => Some(Self::FulltextMismatch),
            "fulltext_failure" => Some(Self::FulltextFailure),
            _ => None,
        }
    }
}

/// Deterministic Zhejiang Library CNKI transport for route and client tests.
#[derive(Clone)]
pub struct FixtureZjlibCnkiTransport {
    mode: FixtureZjlibCnkiMode,
    cookies: Vec<ZjlibCnkiCookie>,
}

impl fmt::Debug for FixtureZjlibCnkiTransport {
    /// Format fixture transport state without exposing cookie values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixtureZjlibCnkiTransport")
            .field("mode", &self.mode)
            .field("cookie_count", &self.cookies.len())
            .field("cookies", &"[REDACTED]")
            .finish()
    }
}

impl FixtureZjlibCnkiTransport {
    /// Build a fixture transport.
    ///
    /// # Arguments
    ///
    /// * `mode` - Fixture behavior.
    ///
    /// # Returns
    ///
    /// Fixture transport.
    pub fn new(mode: FixtureZjlibCnkiMode) -> Self {
        Self {
            mode,
            cookies: Vec::new(),
        }
    }
}

impl ZjlibCnkiTransport for FixtureZjlibCnkiTransport {
    /// Start a fixture QR login challenge.
    fn start_qr_login(&mut self) -> Result<ZjlibCnkiQrLogin, ZjlibCnkiError> {
        if self.mode == FixtureZjlibCnkiMode::StartFailure {
            return Err(ZjlibCnkiError::Request(
                "fixture QR login start failed".to_string(),
            ));
        }
        Ok(ZjlibCnkiQrLogin {
            uuid: "qr-rust-live-fixture".to_string(),
            status: "WAITING_SCAN".to_string(),
            qr_code: "https://qr.test/qr-rust-live-fixture.png".to_string(),
        })
    }

    /// Poll a fixture QR login challenge.
    fn poll_qr_login(
        &mut self,
        _uuid: &str,
        timeout_seconds: i64,
        _interval_seconds: f64,
    ) -> Result<String, ZjlibCnkiError> {
        match self.mode {
            FixtureZjlibCnkiMode::PollTimeout => Err(ZjlibCnkiError::Timeout(format!(
                "Timed out waiting for QR scan after {timeout_seconds} seconds."
            ))),
            FixtureZjlibCnkiMode::PollFailure => Err(ZjlibCnkiError::Request(
                "QR login ended with status FAILED.".to_string(),
            )),
            _ => Ok(build_unsigned_jwt(current_unix_time() + 3600)),
        }
    }

    /// Add the BFF user token as a fixture cookie.
    fn set_login_cookie(&mut self, token: &str) {
        upsert_fixture_cookie(
            &mut self.cookies,
            ZjlibCnkiCookie::new("userToken", token, "www.zjlib.cn"),
        );
    }

    /// Prepare fixture Share and zyproxy cookies.
    fn warm_up_fulltext_session(&mut self, _token: &str) -> Result<String, ZjlibCnkiError> {
        if self.mode == FixtureZjlibCnkiMode::WarmupFailure {
            return Err(ZjlibCnkiError::Request("Share warm-up failed".to_string()));
        }
        upsert_fixture_cookie(
            &mut self.cookies,
            ZjlibCnkiCookie::new(
                "vpn358_sid",
                "SECRET_VPN_VALUE",
                "http-10--18--17--173.elib.zyproxy.zjlib.cn",
            ),
        );
        Ok(format!("{ZYPROXY_BASE_URL}/kns55/"))
    }

    /// Load fixture cookies.
    fn load_cookies(&mut self, cookies: &[ZjlibCnkiCookie]) {
        self.cookies = cookies.to_vec();
    }

    /// Return fixture cookies.
    fn cookies(&self) -> Vec<ZjlibCnkiCookie> {
        let mut cookies = self.cookies.clone();
        cookies.sort_by(|left, right| left.name.cmp(&right.name));
        cookies
    }

    /// Check whether a fixture cookie exists and has not expired.
    fn has_unexpired_cookie(&self, name: &str, now: i64) -> bool {
        self.cookies
            .iter()
            .any(|cookie| cookie.name == name && cookie.is_unexpired(now))
    }

    /// Search fixture CNKI results.
    fn search(
        &mut self,
        keyword: &str,
        limit: usize,
    ) -> Result<Vec<ZjlibCnkiSearchResult>, ZjlibCnkiError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![ZjlibCnkiSearchResult {
            index: 1,
            title: keyword.to_string(),
            detail_url: "https://fixture.cnki.test/detail".to_string(),
            file_name: Some("fixture".to_string()),
            db_name: Some("CJFDLAST2026".to_string()),
            db_code: Some("CJFD".to_string()),
            download_url: Some("https://fixture.cnki.test/download.aspx?dflag=pdfdown".to_string()),
        }])
    }

    /// Inspect fixture result metadata.
    fn inspect_result_metadata(
        &mut self,
        result: &ZjlibCnkiSearchResult,
    ) -> Result<ZjlibCnkiArticleCandidate, ZjlibCnkiError> {
        let identity = if self.mode == FixtureZjlibCnkiMode::FulltextMismatch {
            ZjlibCnkiArticleIdentity {
                title: "Mismatched CNKI Article".to_string(),
                authors: "Different Author".to_string(),
                journal_title: "Different Journal".to_string(),
            }
        } else {
            ZjlibCnkiArticleIdentity {
                title: result.title.clone(),
                authors: "Ada Lovelace; Grace Hopper".to_string(),
                journal_title: "Fixture CNKI Journal".to_string(),
            }
        };
        Ok(ZjlibCnkiArticleCandidate {
            result: result.clone(),
            identity,
            detail_url: result.detail_url.clone(),
            pdf_url: result.download_url.clone(),
        })
    }

    /// Download a fixture PDF.
    fn download_pdf(
        &mut self,
        pdf_url: &str,
        title: Option<&str>,
        _referer: Option<&str>,
    ) -> Result<ZjlibCnkiDownloadedPdf, ZjlibCnkiError> {
        if self.mode == FixtureZjlibCnkiMode::FulltextFailure {
            return Err(ZjlibCnkiError::Request(
                "fixture PDF download failed".to_string(),
            ));
        }
        let content = b"%PDF-1.4\n% fixture cnki pdf\n".to_vec();
        Ok(ZjlibCnkiDownloadedPdf {
            filename: format!(
                "{}.pdf",
                safe_filename(title.unwrap_or("Fixture CNKI Article"))
            ),
            final_url: pdf_url.to_string(),
            content_type: "application/pdf".to_string(),
            byte_count: content.len(),
            content,
        })
    }
}

fn www_headers(base_url: &Url, token: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static(DEFAULT_ACCEPT_LANGUAGE),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    if let Ok(referer) = HeaderValue::from_str(base_url.as_str().trim_end_matches('/')) {
        headers.insert(REFERER, referer);
    }
    headers.insert("bff-org-id", HeaderValue::from_static(BFF_ORG_ID));
    if let Some(token) = token {
        if let Ok(value) = HeaderValue::from_str(token) {
            headers.insert("bff-user-token", value);
        }
    }
    headers
}

fn html_headers(referer: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static(DEFAULT_ACCEPT_LANGUAGE),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    );
    if let Some(referer) = referer.and_then(|value| HeaderValue::from_str(value).ok()) {
        headers.insert(REFERER, referer);
    }
    headers
}

fn ajax_headers(referer: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static(DEFAULT_ACCEPT_LANGUAGE),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(
        "X-Requested-With",
        HeaderValue::from_static("XMLHttpRequest"),
    );
    if let Some(referer) = referer.and_then(|value| HeaderValue::from_str(value).ok()) {
        headers.insert(REFERER, referer);
    }
    headers
}

fn cnki_form_headers(referer: &str, origin: &str) -> HeaderMap {
    let mut headers = html_headers(Some(referer));
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    if let Ok(origin) = HeaderValue::from_str(origin) {
        headers.insert(ORIGIN, origin);
    }
    headers
}

fn search_result_form_fields(keyword: &str) -> Vec<(String, String)> {
    let db_value = "中国学术期刊网络出版总库,中国博士学位论文全文数据库,中国优秀硕士学位论文全文数据库,中国重要会议论文全文数据库,中国重要报纸全文数据库,中国年鉴网络出版总库";
    vec![
        ("dbPrefix", "SCDB"),
        ("db_opt", "中国学术文献网络出版总库"),
        ("db_value", db_value),
        ("hidTabChange", ""),
        ("hidDivIDS", ""),
        ("txt_i", "1"),
        ("txt_c", "7"),
        ("{key}_logical", "and"),
        ("txt_1_sel", "主题"),
        ("txt_1_value1", keyword),
        ("txt_1_freq1", ""),
        ("txt_1_relation", "#CNKI_AND"),
        ("txt_1_value2", "输入检索词"),
        ("txt_1_freq2", ""),
        ("txt_1_special1", "="),
        ("txt_extension", "xls"),
        ("tmpexpertvalue", ""),
        ("expertValue", ""),
        ("cjfdcode", ""),
        ("currentid", "txt_1_value1"),
        ("action", "scdbsearch"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value.to_string()))
    .collect()
}

fn search_handler_form_fields(keyword: &str) -> Vec<(String, String)> {
    let db_value = "中国学术期刊网络出版总库,中国博士学位论文全文数据库,中国优秀硕士学位论文全文数据库,中国重要会议论文全文数据库,中国重要报纸全文数据库,中国年鉴网络出版总库";
    let mut fields = vec![
        ("action", ""),
        ("NaviCode", "*"),
        ("PageName", "ASP.brief_result_aspx"),
        ("DbPrefix", "SCDB"),
        ("DbCatalog", "中国学术文献网络出版总库"),
        ("ConfigFile", "SCDB.xml"),
        ("db_opt", "中国学术文献网络出版总库"),
        ("db_value", db_value),
        ("txt_1_sel", "主题"),
        ("txt_1_value1", keyword),
        ("txt_1_relation", "#CNKI_AND"),
        ("txt_1_special1", "="),
        ("txt_1_extension", "xls"),
        ("his", "0"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value.to_string()))
    .collect::<Vec<_>>();
    fields.push(("__".to_string(), current_millis().to_string()));
    fields
}

fn reqwest_error_message(error: reqwest::Error) -> String {
    error.without_url().to_string()
}

fn request_error(error: reqwest::Error) -> ZjlibCnkiError {
    ZjlibCnkiError::Request(reqwest_error_message(error))
}

fn json_payload(response: Response, action: &str) -> Result<Value, ZjlibCnkiError> {
    let response = raise_for_status(response, action)?;
    let payload = response.json::<Value>().map_err(|error| {
        ZjlibCnkiError::Parse(format!(
            "{action} returned non-JSON response: {}",
            reqwest_error_message(error)
        ))
    })?;
    if payload.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(ZjlibCnkiError::Request(format!(
            "{action} failed: {}",
            payload
                .get("desc")
                .or_else(|| payload.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown upstream error")
        )));
    }
    if !payload.is_object() {
        return Err(ZjlibCnkiError::Parse(format!(
            "{action} returned non-object JSON response."
        )));
    }
    Ok(payload)
}

fn payload_data<'a>(payload: &'a Value, action: &str) -> Result<&'a Value, ZjlibCnkiError> {
    let data = payload.get("data").ok_or_else(|| {
        ZjlibCnkiError::Parse(format!("{action} response did not contain object data."))
    })?;
    if !data.is_object() {
        return Err(ZjlibCnkiError::Parse(format!(
            "{action} response did not contain object data."
        )));
    }
    Ok(data)
}

fn raise_for_status(response: Response, action: &str) -> Result<Response, ZjlibCnkiError> {
    if response.status().as_u16() >= 400 {
        return Err(ZjlibCnkiError::Request(format!(
            "{action} failed with HTTP {}: {}",
            response.status().as_u16(),
            redact_url(response.url().as_str())
        )));
    }
    Ok(response)
}

fn extract_share_cookie_sync(text: &str) -> Option<(String, BTreeMap<String, String>)> {
    let sign = extract_js_var(text, "sign")?;
    let url = extract_js_var(text, "url")?;
    if !text.contains("sso-login/cookie/sync") {
        return None;
    }
    let domain_url =
        extract_js_var(text, "domainUrl").unwrap_or_else(|| SHARE_BASE_URL.to_string());
    let portal_context_path =
        extract_js_var(text, "portalContextPath").unwrap_or_else(|| "/entry".to_string());
    let normalized_domain = if domain_url.starts_with("//") {
        format!("https:{domain_url}")
    } else {
        domain_url
    };
    let sync_url = format!(
        "{}{}/sso-login/cookie/sync",
        normalized_domain.trim_end_matches('/'),
        portal_context_path
    );
    Some((
        sync_url,
        BTreeMap::from([("sign".to_string(), sign), ("url".to_string(), url)]),
    ))
}

fn extract_js_var(text: &str, name: &str) -> Option<String> {
    let marker = format!("var {name}");
    let start = text.find(&marker)?;
    let after_marker = &text[start + marker.len()..];
    let equals = after_marker.find('=')?;
    let after_equals = after_marker[equals + 1..].trim_start();
    let quote = after_equals.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let raw = after_equals[quote.len_utf8()..]
        .split(quote)
        .next()
        .unwrap_or_default();
    Some(decode_js_string(raw))
}

fn extract_window_location(text: &str, base_url: &str) -> Result<String, ZjlibCnkiError> {
    for marker in ["window.location.href", "location.href", "window.location"] {
        let Some(start) = text.find(marker) else {
            continue;
        };
        let after_marker = &text[start + marker.len()..];
        let Some(equals) = after_marker.find('=') else {
            continue;
        };
        let after_equals = after_marker[equals + 1..].trim_start();
        let Some(quote) = after_equals.chars().next() else {
            continue;
        };
        if quote != '\'' && quote != '"' {
            continue;
        }
        let raw = after_equals[quote.len_utf8()..]
            .split(quote)
            .next()
            .unwrap_or_default();
        return join_url(base_url, &decode_js_string(raw));
    }
    Err(ZjlibCnkiError::Parse(
        "Could not find JavaScript window.location redirect.".to_string(),
    ))
}

fn decode_js_string(value: &str) -> String {
    let mut decoded = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let Some(escaped) = chars.next() else {
            decoded.push('\\');
            break;
        };
        match escaped {
            '/' => decoded.push('/'),
            '"' => decoded.push('"'),
            '\'' => decoded.push('\''),
            '\\' => decoded.push('\\'),
            'b' => decoded.push('\u{0008}'),
            'f' => decoded.push('\u{000c}'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            'u' => {
                let digits = chars.by_ref().take(4).collect::<String>();
                if digits.len() == 4 {
                    if let Ok(codepoint) = u32::from_str_radix(&digits, 16) {
                        if let Some(decoded_character) = char::from_u32(codepoint) {
                            decoded.push(decoded_character);
                            continue;
                        }
                    }
                }
                decoded.push_str("\\u");
                decoded.push_str(&digits);
            }
            _ => {
                decoded.push('\\');
                decoded.push(escaped);
            }
        }
    }
    decoded
}

fn join_url(base_url: &str, reference: &str) -> Result<String, ZjlibCnkiError> {
    let base = Url::parse(base_url).map_err(|error| ZjlibCnkiError::Parse(error.to_string()))?;
    let joined = base
        .join(reference)
        .map_err(|error| ZjlibCnkiError::Parse(error.to_string()))?;
    Ok(joined.to_string())
}

fn parse_search_results(text: &str, base_url: &str) -> Vec<ZjlibCnkiSearchResult> {
    let lowered = text.to_ascii_lowercase();
    let mut seen = Vec::new();
    let mut results = Vec::new();
    for anchor in anchor_links(text) {
        if !anchor
            .href
            .to_ascii_lowercase()
            .contains("/kns55/detail/detail.aspx")
        {
            continue;
        }
        let Ok(detail_url) = join_url(base_url, &decode_html(&anchor.href)) else {
            continue;
        };
        if seen.iter().any(|value| value == &detail_url) {
            continue;
        }
        seen.push(detail_url.clone());
        let row_start = lowered[..anchor.start].rfind("<tr").unwrap_or(anchor.start);
        let row_end = lowered[anchor.end..]
            .find("</tr>")
            .map(|index| anchor.end + index + "</tr>".len())
            .unwrap_or(anchor.end);
        let row = &text[row_start..row_end];
        let query = query_dict(&detail_url);
        let title = extract_anchor_title(&anchor.body)
            .or_else(|| query.get("FileName").cloned())
            .unwrap_or_else(|| format!("result-{}", results.len() + 1));
        results.push(ZjlibCnkiSearchResult {
            index: results.len() + 1,
            title,
            detail_url,
            file_name: query.get("FileName").cloned(),
            db_name: query.get("DbName").cloned(),
            db_code: query.get("DbCode").cloned(),
            download_url: extract_result_download_url(row, base_url),
        });
    }
    results
}

fn extract_result_download_url(row: &str, base_url: &str) -> Option<String> {
    anchor_links(row).into_iter().find_map(|anchor| {
        anchor
            .href
            .to_ascii_lowercase()
            .contains("download.aspx")
            .then(|| join_url(base_url, &decode_html(&anchor.href)).ok())
            .flatten()
    })
}

fn extract_pdf_download_url(text: &str, base_url: &str) -> Option<String> {
    anchor_links(text).into_iter().find_map(|anchor| {
        let href = decode_html(&anchor.href);
        let href_lower = href.to_ascii_lowercase();
        if !href_lower.contains("download.aspx") {
            return None;
        }
        let visible = strip_tags(&anchor.body);
        (href_lower.contains("dflag=pdfdown") || visible.to_ascii_uppercase().contains("PDF"))
            .then(|| join_url(base_url, &href).ok())
            .flatten()
    })
}

fn extract_article_identity(text: &str, fallback_title: &str) -> ZjlibCnkiArticleIdentity {
    let title = meta_content(text, "citation_title")
        .or_else(|| first_tag_text(text, "h1"))
        .or_else(|| first_tag_text(text, "h2"))
        .or_else(|| title_text(text))
        .unwrap_or_else(|| fallback_title.to_string());
    let authors = meta_content_list(text, "citation_author");
    let author_text = if authors.is_empty() {
        author_block_text(text)
            .or_else(|| cnki_label_authors(text, "作者"))
            .or_else(|| row_value(text, "作者"))
            .unwrap_or_default()
    } else {
        authors.join("; ")
    };
    let journal_title = meta_content(text, "citation_journal_title")
        .or_else(|| cnki_label_span_text(text, "文献出处", "jname"))
        .or_else(|| cnki_label_first_anchor(text, "文献出处"))
        .or_else(|| cnki_label_first_anchor(text, "刊名"))
        .or_else(|| cnki_label_first_anchor(text, "来源"))
        .or_else(|| row_value(text, "刊名"))
        .or_else(|| row_value(text, "来源"))
        .unwrap_or_default();
    ZjlibCnkiArticleIdentity {
        title,
        authors: author_text,
        journal_title,
    }
}

fn does_article_metadata_match(
    expected: &ZjlibCnkiArticleIdentity,
    actual: &ZjlibCnkiArticleIdentity,
) -> bool {
    titles_match(&expected.title, &actual.title)
        && authors_match(&expected.authors, &actual.authors)
        && titles_match(&expected.journal_title, &actual.journal_title)
}

fn titles_match(expected: &str, actual: &str) -> bool {
    let expected = normalize_exact_text(expected);
    let actual = normalize_exact_text(actual);
    !expected.is_empty() && expected == actual
}

fn authors_match(expected: &str, actual: &str) -> bool {
    let expected = split_author_names(expected);
    let actual = split_author_names(actual);
    !expected.is_empty() && expected == actual
}

fn split_author_names(value: &str) -> Vec<String> {
    let normalized = decode_html(value);
    let names = normalized
        .split([';', '；', ',', '，', '、'])
        .map(normalize_exact_text)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if names.is_empty() {
        let name = normalize_exact_text(&normalized);
        if name.is_empty() {
            Vec::new()
        } else {
            vec![name]
        }
    } else {
        names
    }
}

fn normalize_exact_text(value: &str) -> String {
    decode_html(value)
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn redact_url(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return url.to_string();
    };
    let sensitive = [
        "token",
        "bff-user-token",
        "userid",
        "username",
        "md5",
        "sign",
        "mhEnc",
        "enc",
        "sid",
        "uid",
        "filename",
        "filetitle",
        "title",
        "doi",
        "path",
        "query",
        "keyword",
        "dk",
    ];
    let pairs = parsed
        .query_pairs()
        .map(|(key, value)| {
            if sensitive.iter().any(|item| item.eq_ignore_ascii_case(&key)) {
                (key.to_string(), "<redacted>".to_string())
            } else {
                (key.to_string(), value.to_string())
            }
        })
        .collect::<Vec<_>>();
    let mut output = parsed;
    output.query_pairs_mut().clear().extend_pairs(pairs);
    output.to_string()
}

fn known_cookie_urls(endpoints: &LiveZjlibCnkiEndpoints) -> Vec<Url> {
    [
        endpoints.www_base_url.clone(),
        endpoints.share_base_url.clone(),
        endpoints.zyproxy_login_base_url.clone(),
        endpoints.zyproxy_base_url.clone(),
    ]
    .into_iter()
    .collect()
}

fn cookie_header_pairs(header: &str) -> Vec<(String, String)> {
    header
        .split(';')
        .filter_map(|part| {
            let trimmed = part.trim();
            let (name, value) = trimmed.split_once('=')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

#[derive(Debug)]
struct AnchorLink {
    href: String,
    body: String,
    start: usize,
    end: usize,
}

fn anchor_links(text: &str) -> Vec<AnchorLink> {
    let lowered = text.to_ascii_lowercase();
    let mut cursor = 0;
    let mut links = Vec::new();
    while let Some(start) = lowered[cursor..].find("<a").map(|index| cursor + index) {
        let Some(tag_end) = lowered[start..].find('>').map(|index| start + index + 1) else {
            break;
        };
        let tag = &text[start..tag_end];
        let Some(href) = attr_value(tag, "href") else {
            cursor = tag_end;
            continue;
        };
        let Some(close) = lowered[tag_end..].find("</a>").map(|index| tag_end + index) else {
            cursor = tag_end;
            continue;
        };
        links.push(AnchorLink {
            href,
            body: text[tag_end..close].to_string(),
            start,
            end: close + "</a>".len(),
        });
        cursor = close + "</a>".len();
    }
    links
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lowered = tag.to_ascii_lowercase();
    let marker = format!("{name}=");
    let start = lowered.find(&marker)? + marker.len();
    let rest = tag[start..].trim_start();
    let first = rest.chars().next()?;
    if first == '"' || first == '\'' {
        return rest[first.len_utf8()..]
            .split(first)
            .next()
            .map(|value| decode_html(value).trim().to_string())
            .filter(|value| !value.trim().is_empty());
    }
    rest.split_whitespace()
        .next()
        .map(|value| decode_html(value).trim().to_string())
        .filter(|value| !value.trim().is_empty())
}

fn extract_anchor_title(body: &str) -> Option<String> {
    if let Some(marker_start) = body.find("ReplaceJiankuohao('") {
        let start = marker_start + "ReplaceJiankuohao('".len();
        if let Some(end) = body[start..].find("')") {
            return clean_text(&strip_tags(&body[start..start + end]));
        }
    }
    clean_text(&strip_tags(body))
}

fn query_dict(url: &str) -> BTreeMap<String, String> {
    Url::parse(url)
        .ok()
        .map(|url| {
            url.query_pairs()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn meta_content(text: &str, name: &str) -> Option<String> {
    meta_content_list(text, name).into_iter().next()
}

fn meta_content_list(text: &str, name: &str) -> Vec<String> {
    let lowered = text.to_ascii_lowercase();
    let mut cursor = 0;
    let mut values = Vec::new();
    while let Some(start) = lowered[cursor..].find("<meta").map(|index| cursor + index) {
        let Some(end) = lowered[start..].find('>').map(|index| start + index + 1) else {
            break;
        };
        let tag = &text[start..end];
        if attr_value(tag, "name").is_some_and(|value| value.eq_ignore_ascii_case(name)) {
            if let Some(content) = attr_value(tag, "content").and_then(|value| clean_text(&value)) {
                values.push(content);
            }
        }
        cursor = end;
    }
    values
}

fn first_tag_text(text: &str, tag_name: &str) -> Option<String> {
    let lowered = text.to_ascii_lowercase();
    let marker = format!("<{tag_name}");
    let start = lowered.find(&marker)?;
    let open_end = lowered[start..].find('>').map(|index| start + index + 1)?;
    let close_marker = format!("</{tag_name}>");
    let close = lowered[open_end..]
        .find(&close_marker)
        .map(|index| open_end + index)?;
    clean_text(&strip_tags(&text[open_end..close]))
}

fn title_text(text: &str) -> Option<String> {
    let title = first_tag_text(text, "title")?;
    let mut output = title.as_str();
    for suffix in [" - 中国知网", " - CNKI", " - 中国学术期刊网络出版总库"] {
        if let Some(stripped) = output.strip_suffix(suffix) {
            output = stripped.trim();
        }
    }
    clean_text(output)
}

fn author_block_text(text: &str) -> Option<String> {
    let lowered = text.to_ascii_lowercase();
    let id_start = lowered.find("id=\"authorpart\"")?;
    let block_start = lowered[..id_start].rfind("<h3").unwrap_or(id_start);
    let block_end = lowered[id_start..]
        .find("</h3>")
        .map(|index| id_start + index + "</h3>".len())?;
    let block = &text[block_start..block_end];
    let names = span_texts(block);
    if names.is_empty() {
        clean_text(&strip_tags(block))
    } else {
        Some(names.join("; "))
    }
}

fn span_texts(text: &str) -> Vec<String> {
    let lowered = text.to_ascii_lowercase();
    let mut cursor = 0;
    let mut output = Vec::new();
    while let Some(start) = lowered[cursor..].find("<span").map(|index| cursor + index) {
        let Some(open_end) = lowered[start..].find('>').map(|index| start + index + 1) else {
            break;
        };
        let Some(close) = lowered[open_end..]
            .find("</span>")
            .map(|index| open_end + index)
        else {
            break;
        };
        if let Some(text) = clean_text(&strip_tags(&text[open_end..close])) {
            output.push(text);
        }
        cursor = close + "</span>".len();
    }
    output
}

fn cnki_label_authors(text: &str, label: &str) -> Option<String> {
    let block = cnki_label_block(text, label)?;
    let names = anchor_links(&block)
        .into_iter()
        .filter_map(|anchor| clean_text(&strip_tags(&anchor.body)))
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("; "))
}

fn cnki_label_first_anchor(text: &str, label: &str) -> Option<String> {
    let block = cnki_label_block(text, label)?;
    anchor_links(&block)
        .into_iter()
        .find_map(|anchor| clean_text(&strip_tags(&anchor.body)))
        .or_else(|| clean_text(&strip_tags(&block)))
}

fn cnki_label_span_text(text: &str, label: &str, span_id: &str) -> Option<String> {
    let block = cnki_label_block(text, label)?;
    let lowered = block.to_ascii_lowercase();
    let marker = format!("id=\"{}\"", span_id.to_ascii_lowercase());
    let id_start = lowered.find(&marker)?;
    let span_start = lowered[..id_start].rfind("<span").unwrap_or(id_start);
    let open_end = lowered[span_start..]
        .find('>')
        .map(|index| span_start + index + 1)?;
    let close = lowered[open_end..]
        .find("</span>")
        .map(|index| open_end + index)?;
    clean_text(&strip_tags(&block[open_end..close]))
}

fn cnki_label_block(text: &str, label: &str) -> Option<String> {
    let marker = format!("【{label}】");
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = ["</p>", "</li>", "</div>"]
        .into_iter()
        .filter_map(|marker| rest.find(marker))
        .min()
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn row_value(text: &str, label: &str) -> Option<String> {
    let plain = strip_tags(text);
    let marker = format!("{label}:");
    let alt_marker = format!("{label}：");
    let index = plain.find(&marker).or_else(|| plain.find(&alt_marker))?;
    let start = index
        + if plain[index..].starts_with(&marker) {
            marker.len()
        } else {
            alt_marker.len()
        };
    clean_text(
        plain[start..]
            .split(['\n', '\r'])
            .next()
            .unwrap_or_default(),
    )
}

fn strip_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => {
                inside_tag = true;
                output.push(' ');
            }
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    decode_html(&output)
}

fn clean_text(value: &str) -> Option<String> {
    let text = decode_html(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!text.trim().is_empty()).then(|| text.trim().to_string())
}

fn decode_html(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';') else {
            output.push('&');
            rest = &rest['&'.len_utf8()..];
            continue;
        };
        let entity = &rest['&'.len_utf8()..end];
        if let Some(decoded) = decode_html_entity(entity) {
            output.push_str(&decoded);
            rest = &rest[end + ';'.len_utf8()..];
            continue;
        }
        output.push('&');
        rest = &rest['&'.len_utf8()..];
    }
    output.push_str(rest);
    output
}

fn decode_html_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => Some("&".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        "quot" => Some("\"".to_string()),
        "apos" => Some("'".to_string()),
        _ => {
            let codepoint = if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok()?
            } else if let Some(decimal) = entity.strip_prefix('#') {
                decimal.parse::<u32>().ok()?
            } else {
                return None;
            };
            char::from_u32(codepoint).map(|character| character.to_string())
        }
    }
}

fn safe_filename(value: &str) -> String {
    let text = strip_tags(value);
    let mut output = String::new();
    let mut last_was_space = false;
    for character in text.chars() {
        let replacement = if "\\/:*?\"<>|".contains(character) {
            '_'
        } else {
            character
        };
        if replacement.is_whitespace() {
            if !last_was_space {
                output.push(' ');
            }
            last_was_space = true;
        } else {
            output.push(replacement);
            last_was_space = false;
        }
        if output.chars().count() >= 120 {
            break;
        }
    }
    let trimmed = output.trim_matches([' ', '.']);
    if trimmed.is_empty() {
        "cnki".to_string()
    } else {
        trimmed.to_string()
    }
}

fn title_from_pdf_url(url: &str) -> Option<String> {
    let query = query_dict(url);
    query
        .get("filetitle")
        .or_else(|| query.get("filename"))
        .and_then(|value| clean_text(value))
}

fn cookie_url(cookie: &ZjlibCnkiCookie, endpoints: &LiveZjlibCnkiEndpoints) -> Option<Url> {
    let host = cookie.domain.trim().trim_start_matches('.');
    if host.is_empty() {
        return Some(endpoints.www_base_url.clone());
    }
    if let Some(url) = known_cookie_urls(endpoints)
        .into_iter()
        .find(|url| url.host_str() == Some(host))
    {
        return Some(url);
    }
    Url::parse(&format!("https://{host}/")).ok()
}

fn cookie_string(cookie: &ZjlibCnkiCookie) -> String {
    let mut parts = vec![
        format!("{}={}", cookie.name, cookie.value),
        format!("Path={}", cookie.path),
    ];
    if !cookie.domain.trim().is_empty() {
        parts.push(format!("Domain={}", cookie.domain));
    }
    if cookie.secure {
        parts.push("Secure".to_string());
    }
    parts.join("; ")
}

fn upsert_fixture_cookie(cookies: &mut Vec<ZjlibCnkiCookie>, cookie: ZjlibCnkiCookie) {
    if let Some(existing) = cookies.iter_mut().find(|value| value.name == cookie.name) {
        *existing = cookie;
    } else {
        cookies.push(cookie);
    }
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn current_unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs() as i64
}

fn current_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_millis() as i64
}

fn jwt_expiration(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = decode_base64_url(payload)?;
    let value = serde_json::from_slice::<Value>(&bytes).ok()?;
    value.get("exp").and_then(Value::as_i64)
}

fn build_unsigned_jwt(expires_at: i64) -> String {
    format!(
        "{}.{}.",
        encode_base64_url(br#"{"alg":"none"}"#),
        encode_base64_url(format!(r#"{{"exp":{expires_at}}}"#).as_bytes()),
    )
}

fn encode_base64_url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let second = bytes.get(index + 1).copied().unwrap_or(0);
        let third = bytes.get(index + 2).copied().unwrap_or(0);
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if index + 1 < bytes.len() {
            encoded.push(ALPHABET[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        }
        if index + 2 < bytes.len() {
            encoded.push(ALPHABET[(third & 0b0011_1111) as usize] as char);
        }
        index += 3;
    }
    encoded
}

fn decode_base64_url(value: &str) -> Option<Vec<u8>> {
    let mut bit_buffer = 0_u32;
    let mut bit_count = 0_u8;
    let mut output = Vec::new();
    for byte in value.bytes().filter(|byte| *byte != b'=') {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;
        bit_buffer = (bit_buffer << 6) | digit;
        bit_count += 6;
        while bit_count >= 8 {
            bit_count -= 8;
            output.push(((bit_buffer >> bit_count) & 0xff) as u8);
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    mod live_transport;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    use reqwest::blocking::Client;
    use reqwest::redirect::Policy;
    use reqwest::Url;

    use crate::scholarly::test_support::CapturedLogs;

    use super::{
        extract_anchor_title, extract_pdf_download_url, extract_share_cookie_sync,
        extract_window_location, request_error, retry_zyproxy_login, search_handler_form_fields,
        search_result_form_fields, validate_zyproxy_success, zyproxy_endpoint,
        zyproxy_redirect_action, FixtureZjlibCnkiMode, FixtureZjlibCnkiTransport,
        ZhejiangLibraryCnkiClient, ZjlibCnkiArticleIdentity, ZjlibCnkiCookie, ZjlibCnkiError,
        ZyproxyEntryOutcome, ZyproxyRedirectAction, ZYPROXY_REDIRECT_HOPS,
    };

    #[test]
    fn fulltext_events_keep_context_and_omit_article_material() {
        let sentinel = "fulltext-title-author-cookie-url-sentinel";
        let mut client = warmed_fixture_client(FixtureZjlibCnkiMode::FulltextMismatch);
        let expected = ZjlibCnkiArticleIdentity {
            title: sentinel.to_string(),
            authors: sentinel.to_string(),
            journal_title: sentinel.to_string(),
        };
        let logs = CapturedLogs::default();

        let error = tracing::subscriber::with_default(logs.subscriber(), || {
            let span = tracing::info_span!(
                "index.worker",
                run_id = "run-fulltext-correlation",
                worker_id = 6,
            );
            span.in_scope(|| client.download_matching_pdf(&expected, 10))
        })
        .expect_err("mismatching fixture should fail");

        assert!(matches!(error, ZjlibCnkiError::Request(_)));
        let events = logs.events();
        let failed = events
            .iter()
            .find(|event| event["event"] == "source.fulltext.failed")
            .expect("fulltext failure should be logged");
        assert_eq!(failed["provider"], "zjlib_cnki");
        assert_eq!(failed["error_kind"], "no_exact_match");
        assert_eq!(failed["span"]["run_id"], "run-fulltext-correlation");
        assert_eq!(failed["span"]["worker_id"], 6);
        assert!(events
            .iter()
            .any(|event| event["event"] == "source.fallback.activated"));
        assert!(!logs.text().contains(sentinel));
    }

    #[test]
    fn zyproxy_redirects_follow_normal_transition_and_retry_expected_loop() {
        let login_url =
            Url::parse("https://login.elib.zyproxy.zjlib.cn/index.php?enc=secret&username=user")
                .expect("login URL should parse");
        let proxy_url = Url::parse("https://http-10--18--17--173.elib.zyproxy.zjlib.cn/kns55/")
            .expect("proxy URL should parse");

        let transition = zyproxy_redirect_action(&[], &login_url, Some(proxy_url.as_str()), 0)
            .expect("normal transition should be followed");
        assert!(matches!(
            transition,
            ZyproxyRedirectAction::Follow { next_url, .. } if next_url == proxy_url
        ));

        let history = vec![zyproxy_endpoint(&login_url).expect("login endpoint should validate")];
        let loop_action =
            zyproxy_redirect_action(&history, &proxy_url, Some(login_url.as_str()), 1)
                .expect("expected two-node loop should be retryable");
        assert_eq!(loop_action, ZyproxyRedirectAction::Retry);
    }

    #[test]
    fn zyproxy_redirects_reject_unsafe_or_malformed_transitions() {
        let login_url = Url::parse("https://login.elib.zyproxy.zjlib.cn/index.php")
            .expect("login URL should parse");
        let proxy_url = Url::parse("https://http-10--18--17--173.elib.zyproxy.zjlib.cn/kns55/")
            .expect("proxy URL should parse");

        assert!(zyproxy_redirect_action(&[], &login_url, None, 0).is_err());
        assert!(zyproxy_redirect_action(
            &[],
            &login_url,
            Some("http://http-10--18--17--173.elib.zyproxy.zjlib.cn/kns55/"),
            0,
        )
        .is_err());
        assert!(zyproxy_redirect_action(
            &[],
            &login_url,
            Some("https://attacker.example/kns55/"),
            0,
        )
        .is_err());
        assert!(zyproxy_redirect_action(
            &[],
            &login_url,
            Some("https://login.elib.zyproxy.zjlib.cn:444/index.php"),
            0,
        )
        .is_err());
        assert!(zyproxy_redirect_action(
            &[],
            &login_url,
            Some("https://user@login.elib.zyproxy.zjlib.cn/index.php"),
            0,
        )
        .is_err());
        assert!(zyproxy_redirect_action(&[], &login_url, Some("http://["), 0).is_err());
        assert!(zyproxy_redirect_action(
            &[],
            &login_url,
            Some(proxy_url.as_str()),
            ZYPROXY_REDIRECT_HOPS,
        )
        .is_err());
        assert!(zyproxy_redirect_action(&[], &login_url, Some(login_url.as_str()), 0).is_err());

        let unexpected_login_url = Url::parse("https://login.elib.zyproxy.zjlib.cn/unexpected")
            .expect("unexpected login URL should parse");
        let history = vec![zyproxy_endpoint(&proxy_url).expect("proxy endpoint should validate")];
        assert!(zyproxy_redirect_action(
            &history,
            &unexpected_login_url,
            Some(proxy_url.as_str()),
            1,
        )
        .is_err());
    }

    #[test]
    fn zyproxy_retry_recovers_once_and_caps_persistent_loops() {
        let final_url = "https://http-10--18--17--173.elib.zyproxy.zjlib.cn/kns55/".to_string();
        let mut transient_attempts = 0;
        let mut transient_delays = Vec::new();
        let recovered = retry_zyproxy_login(
            || {
                transient_attempts += 1;
                if transient_attempts == 1 {
                    Ok(ZyproxyEntryOutcome::Retry)
                } else {
                    Ok(ZyproxyEntryOutcome::Ready(final_url.clone()))
                }
            },
            |delay| transient_delays.push(delay),
        )
        .expect("one transient loop should recover");

        assert_eq!(recovered, final_url);
        assert_eq!(transient_attempts, 2);
        assert_eq!(transient_delays, vec![Duration::from_millis(200)]);

        let mut persistent_attempts = 0;
        let mut persistent_delays = Vec::new();
        let persistent_error = retry_zyproxy_login(
            || {
                persistent_attempts += 1;
                Ok(ZyproxyEntryOutcome::Retry)
            },
            |delay| persistent_delays.push(delay),
        )
        .expect_err("persistent loops should exhaust the retry budget");

        assert_eq!(persistent_attempts, 3);
        assert_eq!(
            persistent_delays,
            vec![Duration::from_millis(200), Duration::from_millis(400)]
        );
        assert!(persistent_error
            .to_string()
            .contains("after 3 login attempts"));
    }

    #[test]
    fn zyproxy_success_requires_proxy_origin_and_session_cookie() {
        let login_url = Url::parse("https://login.elib.zyproxy.zjlib.cn/index.php")
            .expect("login URL should parse");
        let proxy_url = Url::parse("https://http-10--18--17--173.elib.zyproxy.zjlib.cn/kns55/")
            .expect("proxy URL should parse");

        assert_eq!(
            validate_zyproxy_success(&proxy_url, true)
                .expect("proxy success with cookie should validate"),
            proxy_url.to_string()
        );
        assert!(validate_zyproxy_success(&proxy_url, false).is_err());
        assert!(validate_zyproxy_success(&login_url, true).is_err());
    }

    #[test]
    fn reqwest_errors_strip_sensitive_urls_before_display() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test request should connect");
            let mut request = [0_u8; 2048];
            let request_length = stream.read(&mut request).expect("test request should read");
            assert!(request_length > 0);
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("test response should write");
        });
        let client = Client::builder()
            .redirect(Policy::limited(0))
            .build()
            .expect("test client should build");
        let error = client
            .get(format!(
                "http://{address}/start?enc=sensitive-value&username=sensitive-user"
            ))
            .send()
            .expect_err("redirect policy should reject the redirect");

        assert!(error
            .url()
            .expect("redirect error should retain its URL before sanitization")
            .as_str()
            .contains("sensitive-value"));
        let sanitized = request_error(error).to_string();
        assert!(!sanitized.contains("http://"));
        assert!(!sanitized.contains("enc="));
        assert!(!sanitized.contains("sensitive-value"));
        assert!(!sanitized.contains("username="));
        assert!(!sanitized.contains("sensitive-user"));
        server.join().expect("test server should finish");
    }

    #[test]
    fn session_debug_redacts_cookie_and_client_credentials() {
        let cookie = ZjlibCnkiCookie::new("userToken", "cookie-secret", ".elib.zyproxy.zjlib.cn");
        let cookie_debug = format!("{cookie:?}");
        assert!(cookie_debug.contains("[REDACTED]"));
        assert!(!cookie_debug.contains("cookie-secret"));

        let mut client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(
            FixtureZjlibCnkiMode::Success,
        ));
        client.start_qr_login().expect("fixture start should work");
        let token = client
            .poll_qr_login(1, 0.1)
            .expect("fixture poll should complete");
        let client_debug = format!("{client:?}");

        assert!(client_debug.contains("[REDACTED]"));
        assert!(!client_debug.contains(&token));
        assert!(!client_debug.contains("qr-rust-live-fixture"));
    }

    #[test]
    fn fixture_client_persists_safe_login_and_warmup_state() {
        let mut client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(
            FixtureZjlibCnkiMode::Success,
        ));

        let login = client.start_qr_login().expect("fixture start should work");
        let token = client
            .poll_qr_login(1, 0.1)
            .expect("fixture poll should complete");
        let final_url = client
            .warm_up_fulltext_session()
            .expect("fixture warm-up should complete");
        let state_data = client.to_state_data();

        assert_eq!(login.uuid, "qr-rust-live-fixture");
        assert!(token.contains('.'));
        assert!(final_url.contains("elib.zyproxy.zjlib.cn"));
        assert_eq!(state_data["qr_uuid"], "qr-rust-live-fixture");
        assert_eq!(state_data["cookies"][0]["name"], "userToken");
        assert_eq!(state_data["cookies"][1]["name"], "vpn358_sid");
        assert_eq!(state_data["final_zyproxy_url"], final_url);
        assert!(client.has_fresh_fulltext_session(None));
    }

    #[test]
    fn fixture_client_restores_state_for_warm_session_reuse() {
        let mut client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(
            FixtureZjlibCnkiMode::Success,
        ));
        client.start_qr_login().expect("fixture start should work");
        client
            .poll_qr_login(1, 0.1)
            .expect("fixture poll should complete");
        client
            .warm_up_fulltext_session()
            .expect("fixture warm-up should complete");
        let state_data = client.to_state_data();
        let restored = ZhejiangLibraryCnkiClient::from_state_data(
            FixtureZjlibCnkiTransport::new(FixtureZjlibCnkiMode::Success),
            &state_data,
        );

        assert!(restored.has_fresh_fulltext_session(None));
        assert_eq!(restored.to_state_data()["cookies"], state_data["cookies"]);
    }

    #[test]
    fn fixture_client_reports_start_timeout_poll_and_warmup_failures() {
        let mut start_client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(
            FixtureZjlibCnkiMode::StartFailure,
        ));
        let start_error = start_client
            .start_qr_login()
            .expect_err("start failure fixture should fail");

        let mut timeout_client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(
            FixtureZjlibCnkiMode::PollTimeout,
        ));
        timeout_client
            .start_qr_login()
            .expect("timeout fixture start should work");
        let timeout_error = timeout_client
            .poll_qr_login(1, 0.1)
            .expect_err("timeout fixture should time out");

        let mut warmup_client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(
            FixtureZjlibCnkiMode::WarmupFailure,
        ));
        warmup_client
            .start_qr_login()
            .expect("warmup fixture start should work");
        warmup_client
            .poll_qr_login(1, 0.1)
            .expect("warmup fixture poll should work");
        let warmup_error = warmup_client
            .warm_up_fulltext_session()
            .expect_err("warmup fixture should fail");

        assert!(matches!(start_error, ZjlibCnkiError::Request(_)));
        assert!(timeout_error.is_timeout());
        assert!(matches!(warmup_error, ZjlibCnkiError::Request(_)));
    }

    #[test]
    fn fixture_client_downloads_exact_matching_fulltext_pdf() {
        let mut client = warmed_fixture_client(FixtureZjlibCnkiMode::Success);
        let expected = fixture_identity();

        let downloaded = client
            .download_matching_pdf(&expected, 10)
            .expect("matching fixture PDF should download");

        assert_eq!(downloaded.filename, "Fixture CNKI Article.pdf");
        assert_eq!(downloaded.content_type, "application/pdf");
        assert!(downloaded.content.starts_with(b"%PDF"));
        assert_eq!(downloaded.byte_count, downloaded.content.len());
    }

    #[test]
    fn fixture_client_reports_fulltext_mismatch_and_download_failure() {
        let expected = fixture_identity();
        let mut mismatch_client = warmed_fixture_client(FixtureZjlibCnkiMode::FulltextMismatch);
        let mismatch_error = mismatch_client
            .download_matching_pdf(&expected, 10)
            .expect_err("mismatching fixture should fail");

        let mut failure_client = warmed_fixture_client(FixtureZjlibCnkiMode::FulltextFailure);
        let failure_error = failure_client
            .download_matching_pdf(&expected, 10)
            .expect_err("download failure fixture should fail");

        assert!(mismatch_error
            .to_string()
            .contains("No exact CNKI full-text match found"));
        assert!(failure_error
            .to_string()
            .contains("fixture PDF download failed"));
    }

    #[test]
    fn helper_parsers_cover_share_sync_and_window_redirects() {
        let sync = extract_share_cookie_sync(
            r#"
            <script>
            var sign = "abc";
            var url = "https:\/\/share.zjlib.cn\/entry";
            var domainUrl = "https://share.zjlib.cn";
            var portalContextPath = "/entry";
            </script>
            <form action="/entry/sso-login/cookie/sync"></form>
            "#,
        )
        .expect("sync payload should parse");
        let location = extract_window_location(
            r#"<script>window.location.href = "/login?sid=abc";</script>"#,
            "https://login.elib.zyproxy.zjlib.cn/start",
        )
        .expect("window location should parse");
        let unicode_location = extract_window_location(
            r#"<script>window.location.href = "https:\/\/login.elib.zyproxy.zjlib.cn\/index.php?r=site%2Fenclogin\u0026enc=abc\u0026username=user\u0026pre=http%3A%2F%2F10.18.17.173%2Fkns55%2F";</script>"#,
            "https://share.zjlib.cn/entry/area/35594/2120",
        )
        .expect("unicode-escaped window location should parse");
        let pdf_url = extract_pdf_download_url(
            r##"<a target="_blank" href="&#xA; /kcms/download.aspx?filename=abc&amp;tablename=CJFDLAST2025&amp;dflag=pdfdown&#xA; "><b>PDF下载</b></a>"##,
            "https://http-10--18--17--173.elib.zyproxy.zjlib.cn/kcms/detail/detail.aspx?FileName=abc",
        )
        .expect("numeric-escaped PDF URL should parse");
        let result_fields = search_result_form_fields("lstm");
        let handler_fields = search_handler_form_fields("lstm");
        let highlighted_title = extract_anchor_title(
            "document.write(ReplaceChar1(ReplaceChar(ReplaceJiankuohao('基于<font class=Mark>LSTM</font>的股票预测'))));",
        )
        .expect("highlighted result title should parse");

        assert_eq!(sync.0, "https://share.zjlib.cn/entry/sso-login/cookie/sync");
        assert_eq!(sync.1["sign"], "abc");
        assert_eq!(
            location,
            "https://login.elib.zyproxy.zjlib.cn/login?sid=abc"
        );
        assert_eq!(
            unicode_location,
            "https://login.elib.zyproxy.zjlib.cn/index.php?r=site%2Fenclogin&enc=abc&username=user&pre=http%3A%2F%2F10.18.17.173%2Fkns55%2F"
        );
        assert_eq!(
            pdf_url,
            "https://http-10--18--17--173.elib.zyproxy.zjlib.cn/kcms/download.aspx?filename=abc&tablename=CJFDLAST2025&dflag=pdfdown"
        );
        assert!(result_fields.contains(&("txt_1_sel".to_string(), "主题".to_string())));
        assert!(handler_fields.contains(&("txt_1_sel".to_string(), "主题".to_string())));
        assert_eq!(highlighted_title, "基于 LSTM 的股票预测");
    }

    fn warmed_fixture_client(
        mode: FixtureZjlibCnkiMode,
    ) -> ZhejiangLibraryCnkiClient<FixtureZjlibCnkiTransport> {
        let mut client = ZhejiangLibraryCnkiClient::new(FixtureZjlibCnkiTransport::new(mode));
        client.start_qr_login().expect("fixture start should work");
        client
            .poll_qr_login(1, 0.1)
            .expect("fixture poll should complete");
        client
            .warm_up_fulltext_session()
            .expect("fixture warm-up should complete");
        client
    }

    fn fixture_identity() -> ZjlibCnkiArticleIdentity {
        ZjlibCnkiArticleIdentity {
            title: "Fixture CNKI Article".to_string(),
            authors: "Ada Lovelace; Grace Hopper".to_string(),
            journal_title: "Fixture CNKI Journal".to_string(),
        }
    }
}
