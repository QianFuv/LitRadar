//! Zhejiang Library mediated CNKI login and full-text session client.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Response};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, LOCATION, ORIGIN, REFERER, USER_AGENT,
};
use reqwest::redirect::Policy;
use reqwest::Url;
use serde_json::{json, Value};

const WWW_BASE_URL: &str = "https://www.zjlib.cn";
const SHARE_BASE_URL: &str = "https://share.zjlib.cn";
const ZYPROXY_BASE_URL: &str = "https://http-10--18--17--173.elib.zyproxy.zjlib.cn";
const ENTRY_URL: &str = "https://share.zjlib.cn/entry/area/35594/2120";
const LIBRARY_REFER: &str = "http://10.18.17.173/kns55/";
const WFWFID: &str = "2120";
const BFF_ORG_ID: &str = "1916318653650423810";
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const TOKEN_EXPIRY_SKEW_SECONDS: i64 = 300;
const FULLTEXT_WARM_UP_TTL_SECONDS: i64 = 60 * 60;
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

/// JSON-serializable cookie state persisted with a CNKI session.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// Zhejiang Library CNKI client using a transport implementation.
#[derive(Debug, Clone)]
pub struct ZhejiangLibraryCnkiClient<T> {
    transport: T,
    bff_user_token: Option<String>,
    qr_uuid: Option<String>,
    fulltext_warmed_at: Option<i64>,
    final_zyproxy_url: Option<String>,
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
        let login = self.transport.start_qr_login()?;
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
        let token = self
            .transport
            .poll_qr_login(qr_uuid, timeout_seconds, interval_seconds)?;
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
            return Ok(self
                .final_zyproxy_url
                .clone()
                .unwrap_or_else(|| format!("{ZYPROXY_BASE_URL}/kns55/")));
        }
        let token = self.ensure_logged_in()?;
        self.transport.set_login_cookie(&token);
        let final_url = self.transport.warm_up_fulltext_session(&token)?;
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
        }
    }
}

/// Blocking HTTP transport for live Zhejiang Library CNKI login.
#[derive(Debug, Clone)]
pub struct LiveZjlibCnkiTransport {
    redirect_client: Client,
    no_redirect_client: Client,
    cookie_jar: Arc<Jar>,
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
        let cookie_jar = Arc::new(Jar::default());
        let timeout = Duration::from_secs(config.timeout_seconds.max(1));
        let redirect_client = Client::builder()
            .timeout(timeout)
            .cookie_provider(cookie_jar.clone())
            .redirect(Policy::limited(10))
            .build()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        let no_redirect_client = Client::builder()
            .timeout(timeout)
            .cookie_provider(cookie_jar.clone())
            .redirect(Policy::none())
            .build()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        Ok(Self {
            redirect_client,
            no_redirect_client,
            cookie_jar,
        })
    }

    fn build_share_sso_url(&mut self, token: &str) -> Result<String, ZjlibCnkiError> {
        let response = self
            .no_redirect_client
            .get(format!(
                "{WWW_BASE_URL}/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl"
            ))
            .query(&[("referURL", ENTRY_URL)])
            .headers(www_headers(Some(token)))
            .send()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        let payload = json_payload(response, "build Share SSO URL")?;
        let sso_url = payload
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !sso_url.starts_with(SHARE_BASE_URL) {
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
            .headers(html_headers(Some(&format!("{WWW_BASE_URL}/"))))
            .send()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        let response = raise_for_status(response, "enter Share protocolAuth")?;
        let response_url = response.url().to_string();
        let text = response
            .text()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        if let Some((sync_url, data)) = extract_share_cookie_sync(&text) {
            let response = self
                .redirect_client
                .post(sync_url)
                .form(&data)
                .headers(html_headers(Some(&response_url)))
                .header(ORIGIN, SHARE_BASE_URL)
                .send()
                .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
            raise_for_status(response, "sync Share login cookies")?;
        }
        let response = self
            .redirect_client
            .get(ENTRY_URL)
            .headers(html_headers(Some(sso_url)))
            .send()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        raise_for_status(response, "open Share entry")?;
        let response = self
            .redirect_client
            .get(format!("{SHARE_BASE_URL}/engine2/header/user-info"))
            .query(&[("t", current_millis().to_string())])
            .headers(ajax_headers(Some(ENTRY_URL)))
            .send()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        raise_for_status(response, "load Share user info")?;
        Ok(())
    }

    fn get_zyproxy_login_url(&mut self) -> Result<String, ZjlibCnkiError> {
        let response = self
            .no_redirect_client
            .get(format!("{SHARE_BASE_URL}/sso/api/auth/library/vpn358"))
            .query(&[("wfwfid", WFWFID), ("refer", LIBRARY_REFER)])
            .headers(html_headers(Some(ENTRY_URL)))
            .send()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        let response = raise_for_status(response, "get zyproxy login URL")?;
        let response_url = response.url().to_string();
        if let Some(location) = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
        {
            return join_url(&response_url, location);
        }
        let text = response
            .text()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        let login_url = extract_window_location(&text, &response_url)?;
        if !login_url.contains("login.elib.zyproxy.zjlib.cn") {
            return Err(ZjlibCnkiError::Parse(
                "Share library auth did not return login.elib redirect.".to_string(),
            ));
        }
        Ok(login_url)
    }

    fn enter_zyproxy(&mut self, login_url: &str) -> Result<String, ZjlibCnkiError> {
        let response = self
            .redirect_client
            .get(login_url)
            .headers(html_headers(Some(&format!("{SHARE_BASE_URL}/"))))
            .send()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
        let response = raise_for_status(response, "enter zyproxy")?;
        let final_url = response.url().to_string();
        if !final_url.contains("elib.zyproxy.zjlib.cn") {
            return Err(ZjlibCnkiError::Parse(format!(
                "Unexpected zyproxy final URL: {}",
                redact_url(&final_url)
            )));
        }
        if !self.has_unexpired_cookie("vpn358_sid", current_unix_time()) {
            return Err(ZjlibCnkiError::Parse(
                "zyproxy login did not set vpn358_sid.".to_string(),
            ));
        }
        Ok(final_url)
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
            .get(format!(
                "{WWW_BASE_URL}/bff-api/reader-sso-service/portal-pc-api/login/zfb-qr"
            ))
            .headers(www_headers(None))
            .send()
            .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
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
                .get(format!(
                    "{WWW_BASE_URL}/bff-api/reader-sso-service/portal-pc-api/qr/status"
                ))
                .query(&[("uuid", uuid)])
                .headers(www_headers(None))
                .send()
                .map_err(|error| ZjlibCnkiError::Request(error.to_string()))?;
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
        if let Ok(url) = Url::parse(WWW_BASE_URL) {
            self.cookie_jar
                .add_cookie_str(&format!("userToken={token}; Path=/; Secure"), &url);
        }
    }

    /// Prepare live Share and zyproxy cookies.
    fn warm_up_fulltext_session(&mut self, token: &str) -> Result<String, ZjlibCnkiError> {
        let sso_url = self.build_share_sso_url(token)?;
        self.enter_share(&sso_url)?;
        let login_url = self.get_zyproxy_login_url()?;
        self.enter_zyproxy(&login_url)
    }

    /// Load persisted cookies into the live cookie jar.
    fn load_cookies(&mut self, cookies: &[ZjlibCnkiCookie]) {
        for cookie in cookies {
            if cookie.name.trim().is_empty() {
                continue;
            }
            let Some(url) = cookie_url(cookie) else {
                continue;
            };
            self.cookie_jar.add_cookie_str(&cookie_string(cookie), &url);
        }
    }

    /// Snapshot live cookies from known Zhejiang Library domains.
    fn cookies(&self) -> Vec<ZjlibCnkiCookie> {
        let mut cookies = BTreeMap::new();
        for url in known_cookie_urls() {
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
            _ => None,
        }
    }
}

/// Deterministic Zhejiang Library CNKI transport for route and client tests.
#[derive(Debug, Clone)]
pub struct FixtureZjlibCnkiTransport {
    mode: FixtureZjlibCnkiMode,
    cookies: Vec<ZjlibCnkiCookie>,
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
}

fn www_headers(token: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static(DEFAULT_ACCEPT_LANGUAGE),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(REFERER, HeaderValue::from_static(WWW_BASE_URL));
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

fn json_payload(response: Response, action: &str) -> Result<Value, ZjlibCnkiError> {
    let response = raise_for_status(response, action)?;
    let payload = response.json::<Value>().map_err(|error| {
        ZjlibCnkiError::Parse(format!("{action} returned non-JSON response: {error}"))
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
    value
        .replace("\\/", "/")
        .replace("\\\"", "\"")
        .replace("\\'", "'")
        .replace("\\\\", "\\")
}

fn join_url(base_url: &str, reference: &str) -> Result<String, ZjlibCnkiError> {
    let base = Url::parse(base_url).map_err(|error| ZjlibCnkiError::Parse(error.to_string()))?;
    let joined = base
        .join(reference)
        .map_err(|error| ZjlibCnkiError::Parse(error.to_string()))?;
    Ok(joined.to_string())
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

fn known_cookie_urls() -> Vec<Url> {
    [WWW_BASE_URL, SHARE_BASE_URL, ZYPROXY_BASE_URL]
        .into_iter()
        .filter_map(|value| Url::parse(value).ok())
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

fn cookie_url(cookie: &ZjlibCnkiCookie) -> Option<Url> {
    let host = cookie.domain.trim().trim_start_matches('.');
    if host.is_empty() {
        return Url::parse(WWW_BASE_URL).ok();
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
    use super::{
        extract_share_cookie_sync, extract_window_location, FixtureZjlibCnkiMode,
        FixtureZjlibCnkiTransport, ZhejiangLibraryCnkiClient, ZjlibCnkiError,
    };

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

        assert_eq!(sync.0, "https://share.zjlib.cn/entry/sso-login/cookie/sync");
        assert_eq!(sync.1["sign"], "abc");
        assert_eq!(
            location,
            "https://login.elib.zyproxy.zjlib.cn/login?sid=abc"
        );
    }
}
