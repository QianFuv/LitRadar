//! Bounded outbound HTTP transport shared by notification providers.

use std::error::Error;
use std::fmt;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::blocking::{Client, ClientBuilder};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::Url;
use serde_json::Value;
use tokio::sync::Semaphore;

pub(crate) const MAX_JSON_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const MAX_CONCURRENT_DNS_LOOKUPS: usize = 8;
const REQUEST_ID_HEADERS: [&str; 3] = ["x-request-id", "request-id", "cf-ray"];
static DNS_LOOKUP_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

/// Format only the destination host, port, and path for safe diagnostics.
pub(crate) fn redacted_url(value: &str) -> String {
    let Ok(url) = Url::parse(value) else {
        return "[INVALID URL]".to_string();
    };
    let Some(host) = url.host_str() else {
        return "[INVALID URL]".to_string();
    };
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{host}{port}{}", url.path())
}

/// Safe failure categories for bounded outbound requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutboundHttpError {
    /// URL parsing or credential/query validation failed.
    InvalidUrl,
    /// Production request did not use HTTPS.
    HttpsRequired,
    /// URL did not contain a destination host.
    HostRequired,
    /// DNS returned no usable address set.
    DnsResolutionFailed,
    /// At least one resolved address was not publicly routable.
    DisallowedAddress,
    /// Connection establishment failed.
    ConnectFailed,
    /// Request exceeded its total timeout.
    TimedOut,
    /// Request or response streaming failed.
    RequestFailed,
    /// Response declared a non-identity content encoding.
    UnsupportedContentEncoding,
    /// Successful response did not declare a JSON media type.
    UnexpectedContentType,
    /// Successful response exceeded the configured byte limit.
    ResponseTooLarge,
    /// Successful response body was not valid JSON.
    InvalidJson,
}

impl fmt::Display for OutboundHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidUrl => "Outbound request URL is invalid",
            Self::HttpsRequired => "Outbound requests require HTTPS",
            Self::HostRequired => "Outbound request URL requires a host",
            Self::DnsResolutionFailed => "Outbound endpoint DNS resolution failed",
            Self::DisallowedAddress => "Outbound endpoint resolved to a disallowed address",
            Self::ConnectFailed => "Outbound endpoint connection failed",
            Self::TimedOut => "Outbound request timed out",
            Self::RequestFailed => "Outbound request failed",
            Self::UnsupportedContentEncoding => {
                "Outbound response uses an unsupported content encoding"
            }
            Self::UnexpectedContentType => "Outbound response must use a JSON content type",
            Self::ResponseTooLarge => "Outbound response exceeded the size limit",
            Self::InvalidJson => "Outbound response contained invalid JSON",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsPolicyError {
    ResolutionFailed,
    DisallowedAddress,
    ResolverUnavailable,
}

impl fmt::Display for DnsPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolutionFailed => "Outbound endpoint DNS resolution failed",
            Self::DisallowedAddress => "Outbound endpoint resolved to a disallowed address",
            Self::ResolverUnavailable => "Outbound endpoint DNS resolver is unavailable",
        })
    }
}

impl Error for DnsPolicyError {}

#[derive(Debug, Clone, Copy)]
struct ValidatingDnsResolver {
    is_private_address_allowed: bool,
}

impl Resolve for ValidatingDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let is_private_address_allowed = self.is_private_address_allowed;
        Box::pin(async move {
            let lookup_permit = dns_lookup_slots()
                .acquire_owned()
                .await
                .map_err(|_| boxed_dns_error(DnsPolicyError::ResolverUnavailable))?;
            let addresses = tokio::task::spawn_blocking(move || {
                let result = (host.as_str(), 0)
                    .to_socket_addrs()
                    .map(|addresses| addresses.collect::<Vec<_>>());
                drop(lookup_permit);
                result
            })
            .await
            .map_err(|_| boxed_dns_error(DnsPolicyError::ResolverUnavailable))?
            .map_err(|_| boxed_dns_error(DnsPolicyError::ResolutionFailed))?;
            validate_address_policy(&addresses, is_private_address_allowed)
                .map_err(boxed_dns_error)?;
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

/// Bounded JSON response without upstream error-body material.
#[derive(Clone, PartialEq)]
pub(crate) struct BoundedJsonResponse {
    /// HTTP status code.
    pub(crate) status_code: u16,
    /// Safe upstream request identifier when supplied.
    pub(crate) request_id: Option<String>,
    /// Numeric Retry-After delay when supplied.
    pub(crate) retry_after_seconds: Option<u64>,
    /// Parsed success payload; non-success bodies are never read.
    pub(crate) body: Option<Value>,
}

impl fmt::Debug for BoundedJsonResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedJsonResponse")
            .field("status_code", &self.status_code)
            .field("request_id", &self.request_id)
            .field("retry_after_seconds", &self.retry_after_seconds)
            .field("body", &self.body.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Strict request-time HTTP policy for external notification providers.
#[derive(Clone)]
pub(crate) struct BoundedHttpClient {
    connect_timeout: Duration,
    total_timeout: Duration,
    max_response_bytes: usize,
    is_http_allowed: bool,
    is_private_address_allowed: bool,
}

impl fmt::Debug for BoundedHttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedHttpClient")
            .field("connect_timeout", &self.connect_timeout)
            .field("total_timeout", &self.total_timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish_non_exhaustive()
    }
}

impl BoundedHttpClient {
    /// Build a production outbound client policy.
    pub(crate) fn new(timeout_seconds: u64) -> Self {
        let total_timeout = Duration::from_secs(timeout_seconds.max(1));
        Self {
            connect_timeout: Duration::from_secs(
                DEFAULT_CONNECT_TIMEOUT_SECONDS.min(timeout_seconds.max(1)),
            ),
            total_timeout,
            max_response_bytes: MAX_JSON_RESPONSE_BYTES,
            is_http_allowed: false,
            is_private_address_allowed: false,
        }
    }

    /// Send one JSON POST after validating and pinning the destination addresses.
    #[cfg(test)]
    pub(crate) fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<BoundedJsonResponse, OutboundHttpError> {
        self.post_json_with_timeout(url, headers, body, self.total_timeout)
    }

    /// Send one JSON POST with a caller-supplied timeout capped by a total job deadline.
    pub(crate) fn post_json_with_timeout(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
        timeout: Duration,
    ) -> Result<BoundedJsonResponse, OutboundHttpError> {
        let url = self.validate_url(url)?;
        let resolver = Arc::new(ValidatingDnsResolver {
            is_private_address_allowed: self.is_private_address_allowed,
        });
        self.post_json_with_builder(
            url,
            self.client_builder(timeout).dns_resolver(resolver),
            headers,
            body,
        )
    }

    #[cfg(test)]
    fn post_json_to_resolved(
        &self,
        url: Url,
        host: String,
        addresses: Vec<SocketAddr>,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<BoundedJsonResponse, OutboundHttpError> {
        let mut client_builder = self.client_builder(self.total_timeout);
        if parse_ip_host(&host).is_none() {
            client_builder = client_builder.resolve_to_addrs(&host, &addresses);
        }
        self.post_json_with_builder(url, client_builder, headers, body)
    }

    #[cfg(test)]
    fn post_json_with_resolver<R: Resolve + 'static>(
        &self,
        url: Url,
        resolver: Arc<R>,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<BoundedJsonResponse, OutboundHttpError> {
        self.post_json_with_builder(
            url,
            self.client_builder(self.total_timeout)
                .dns_resolver(resolver),
            headers,
            body,
        )
    }

    fn client_builder(&self, timeout: Duration) -> ClientBuilder {
        let timeout = timeout.max(Duration::from_millis(1));
        Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(self.connect_timeout.min(timeout))
            .timeout(timeout)
            .gzip(false)
    }

    fn post_json_with_builder(
        &self,
        url: Url,
        client_builder: ClientBuilder,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<BoundedJsonResponse, OutboundHttpError> {
        let client = client_builder
            .build()
            .map_err(|_| OutboundHttpError::RequestFailed)?;
        let mut request = client.post(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.json(body).send().map_err(classify_request_error)?;
        let status_code = response.status().as_u16();
        let request_id = response_request_id(response.headers());
        let retry_after_seconds = response_retry_after_seconds(response.headers());
        if !(200..300).contains(&status_code) {
            return Ok(BoundedJsonResponse {
                status_code,
                request_id,
                retry_after_seconds,
                body: None,
            });
        }
        validate_response_headers(response.headers(), self.max_response_bytes)?;
        let mut bytes = Vec::with_capacity(self.max_response_bytes.min(64 * 1024));
        response
            .take((self.max_response_bytes + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| OutboundHttpError::RequestFailed)?;
        if bytes.len() > self.max_response_bytes {
            return Err(OutboundHttpError::ResponseTooLarge);
        }
        let body = serde_json::from_slice(&bytes).map_err(|_| OutboundHttpError::InvalidJson)?;
        Ok(BoundedJsonResponse {
            status_code,
            request_id,
            retry_after_seconds,
            body: Some(body),
        })
    }

    fn validate_url(&self, value: &str) -> Result<Url, OutboundHttpError> {
        let url = Url::parse(value).map_err(|_| OutboundHttpError::InvalidUrl)?;
        if url.scheme() != "https" && !(self.is_http_allowed && url.scheme() == "http") {
            return Err(OutboundHttpError::HttpsRequired);
        }
        if outbound_authority_has_userinfo(value)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port() == Some(0)
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(OutboundHttpError::InvalidUrl);
        }
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or(OutboundHttpError::HostRequired)?;
        if let Some(address) = parse_ip_host(host) {
            self.validate_resolved_addresses(&[SocketAddr::new(address, 0)])?;
        }
        Ok(url)
    }

    fn validate_resolved_addresses(
        &self,
        addresses: &[SocketAddr],
    ) -> Result<(), OutboundHttpError> {
        validate_address_policy(addresses, self.is_private_address_allowed).map_err(Into::into)
    }

    #[cfg(test)]
    fn for_local_test(max_response_bytes: usize) -> Self {
        Self {
            connect_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_secs(2),
            max_response_bytes,
            is_http_allowed: true,
            is_private_address_allowed: true,
        }
    }
}

impl From<DnsPolicyError> for OutboundHttpError {
    fn from(error: DnsPolicyError) -> Self {
        match error {
            DnsPolicyError::ResolutionFailed => Self::DnsResolutionFailed,
            DnsPolicyError::DisallowedAddress => Self::DisallowedAddress,
            DnsPolicyError::ResolverUnavailable => Self::RequestFailed,
        }
    }
}

fn dns_lookup_slots() -> Arc<Semaphore> {
    Arc::clone(
        DNS_LOOKUP_SLOTS.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_DNS_LOOKUPS))),
    )
}

fn boxed_dns_error(error: DnsPolicyError) -> Box<dyn Error + Send + Sync> {
    Box::new(error)
}

fn validate_address_policy(
    addresses: &[SocketAddr],
    is_private_address_allowed: bool,
) -> Result<(), DnsPolicyError> {
    if addresses.is_empty() {
        return Err(DnsPolicyError::ResolutionFailed);
    }
    if !is_private_address_allowed
        && addresses
            .iter()
            .any(|address| !is_public_address(address.ip()))
    {
        return Err(DnsPolicyError::DisallowedAddress);
    }
    Ok(())
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

fn parse_ip_host(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

fn classify_request_error(error: reqwest::Error) -> OutboundHttpError {
    if error.is_timeout() {
        OutboundHttpError::TimedOut
    } else if let Some(error) = dns_policy_error(&error) {
        error.into()
    } else if error.is_connect() {
        OutboundHttpError::ConnectFailed
    } else {
        OutboundHttpError::RequestFailed
    }
}

fn dns_policy_error(error: &(dyn Error + 'static)) -> Option<DnsPolicyError> {
    let mut current = Some(error);
    while let Some(candidate) = current {
        if let Some(error) = candidate.downcast_ref::<DnsPolicyError>() {
            return Some(*error);
        }
        current = candidate.source();
    }
    None
}

fn validate_response_headers(
    headers: &reqwest::header::HeaderMap,
    max_response_bytes: usize,
) -> Result<(), OutboundHttpError> {
    if headers
        .get_all(CONTENT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(OutboundHttpError::UnsupportedContentEncoding);
    }
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type != "application/json" && !content_type.ends_with("+json") {
        return Err(OutboundHttpError::UnexpectedContentType);
    }
    if headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > max_response_bytes)
    {
        return Err(OutboundHttpError::ResponseTooLarge);
    }
    Ok(())
}

fn response_request_id(headers: &reqwest::header::HeaderMap) -> Option<String> {
    REQUEST_ID_HEADERS.iter().find_map(|name| {
        headers
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| is_safe_request_id(value))
            .map(str::to_string)
    })
}

fn is_safe_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn response_retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_documentation()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || octets[0] >= 240)
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(address) = address.to_ipv4() {
        return is_public_ipv4(address);
    }
    let segments = address.segments();
    let is_current_global_unicast = (segments[0] & 0xe000) == 0x2000;
    let is_ietf_protocol_assignment = segments[0] == 0x2001 && segments[1] <= 0x01ff;
    let is_documentation = (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || (segments[0] == 0x3fff && (segments[1] & 0xf000) == 0);
    let is_six_to_four = segments[0] == 0x2002;
    let is_as112_delegation =
        segments[0] == 0x2620 && segments[1] == 0x004f && segments[2] == 0x8000;
    is_current_global_unicast
        && !is_ietf_protocol_assignment
        && !is_documentation
        && !is_six_to_four
        && !is_as112_delegation
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    use serde_json::json;

    use super::*;

    #[derive(Debug)]
    struct PendingDnsResolver;

    impl Resolve for PendingDnsResolver {
        fn resolve(&self, _name: Name) -> Resolving {
            Box::pin(std::future::pending())
        }
    }

    #[test]
    fn outbound_policy_rejects_literal_and_resolved_private_addresses() {
        let client = BoundedHttpClient::new(5);
        for endpoint in [
            "https://0.0.0.0/v1",
            "https://127.0.0.1/v1",
            "https://127.1/v1",
            "https://2130706433/v1",
            "https://10.0.0.1/v1",
            "https://172.16.0.1/v1",
            "https://192.168.0.1/v1",
            "https://169.254.169.254/v1",
            "https://224.0.0.1/v1",
            "https://[::]/v1",
            "https://[::1]/v1",
            "https://[fc00::1]/v1",
            "https://[fe80::1]/v1",
            "https://[ff02::1]/v1",
            "https://[64:ff9b:1::1]/v1",
            "https://[64:ff9b::a9fe:a9fe]/v1",
            "https://[100::1]/v1",
            "https://[2001::1]/v1",
            "https://[2001:db8::1]/v1",
            "https://[2002:7f00:1::]/v1",
            "https://[2620:4f:8000::1]/v1",
            "https://[3fff::1]/v1",
        ] {
            assert_eq!(
                client.validate_url(endpoint).unwrap_err(),
                OutboundHttpError::DisallowedAddress
            );
        }
        assert_eq!(
            client
                .post_json("https://localhost/v1", &[], &json!({}))
                .unwrap_err(),
            OutboundHttpError::DisallowedAddress
        );
    }

    #[test]
    fn outbound_policy_rejects_unsafe_url_syntax_before_resolution() {
        let client = BoundedHttpClient::new(5);
        for endpoint in [
            "http://api.example/v1",
            "https://user:secret@api.example/v1",
            "https://@api.example/v1",
            "https://api.example:0/v1",
            "https://api.example/v1?target=private",
            "https://api.example/v1#private",
        ] {
            assert!(
                matches!(
                    client.validate_url(endpoint),
                    Err(OutboundHttpError::HttpsRequired | OutboundHttpError::InvalidUrl)
                ),
                "unsafe endpoint should fail before DNS resolution"
            );
        }
    }

    #[test]
    fn outbound_policy_rejects_mixed_dns_results() {
        let client = BoundedHttpClient::new(5);
        let addresses = [
            SocketAddr::from(([93, 184, 216, 34], 443)),
            SocketAddr::from(([127, 0, 0, 1], 443)),
        ];

        assert_eq!(
            client.validate_resolved_addresses(&addresses).unwrap_err(),
            OutboundHttpError::DisallowedAddress
        );
    }

    #[test]
    fn outbound_policy_accepts_a_public_https_address_set() {
        let client = BoundedHttpClient::new(5);
        let addresses = [
            SocketAddr::from(([93, 184, 216, 34], 443)),
            SocketAddr::new("2606:4700:4700::1111".parse().unwrap(), 443),
        ];

        client
            .validate_resolved_addresses(&addresses)
            .expect("public HTTPS destination should pass address policy");
    }

    #[test]
    fn outbound_total_timeout_includes_dns_resolution() {
        let client = BoundedHttpClient {
            connect_timeout: Duration::from_millis(50),
            total_timeout: Duration::from_millis(50),
            max_response_bytes: 128,
            is_http_allowed: true,
            is_private_address_allowed: true,
        };
        let started_at = Instant::now();
        let error = client
            .post_json_with_resolver(
                Url::parse("http://pending-dns.example/v1").unwrap(),
                Arc::new(PendingDnsResolver),
                &[],
                &json!({}),
            )
            .unwrap_err();

        assert_eq!(error, OutboundHttpError::TimedOut);
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn outbound_client_reaches_only_the_pinned_fixture_address() {
        let address = spawn_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"ok\":true}",
        );
        let url = Url::parse(&format!("http://pinned.example:{}/v1", address.port()))
            .expect("fixture URL should parse");
        let response = BoundedHttpClient::for_local_test(128)
            .post_json_to_resolved(
                url,
                "pinned.example".to_string(),
                vec![address],
                &[],
                &json!({}),
            )
            .expect("pinned fixture should receive the request");

        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, Some(json!({"ok": true})));
    }

    #[test]
    fn outbound_client_does_not_follow_redirects_or_read_error_bodies() {
        let endpoint = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private\r\nX-Request-Id: request-123\r\nRetry-After: 7\r\nContent-Length: 18\r\n\r\nprivate-error-body",
        );
        let response = BoundedHttpClient::for_local_test(128)
            .post_json(&endpoint, &[], &json!({}))
            .expect("redirect response should be returned without following it");

        assert_eq!(response.status_code, 302);
        assert_eq!(response.request_id.as_deref(), Some("request-123"));
        assert_eq!(response.retry_after_seconds, Some(7));
        assert_eq!(response.body, None);

        let malicious_request_id = serve_once(
            "HTTP/1.1 503 Service Unavailable\r\nX-Request-Id: private response sentinel\r\nContent-Length: 0\r\n\r\n",
        );
        let response = BoundedHttpClient::for_local_test(128)
            .post_json(&malicious_request_id, &[], &json!({}))
            .expect("error response should remain body-free");
        assert_eq!(response.status_code, 503);
        assert_eq!(response.request_id, None);
    }

    #[test]
    fn outbound_client_rejects_oversized_encoded_and_non_json_responses() {
        let oversized = serve_once(&format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 129\r\n\r\n{}",
            "x".repeat(129)
        ));
        assert_eq!(
            BoundedHttpClient::for_local_test(128)
                .post_json(&oversized, &[], &json!({}))
                .unwrap_err(),
            OutboundHttpError::ResponseTooLarge
        );

        let chunked_body = "x".repeat(129);
        let chunked = serve_once(&format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n81\r\n{chunked_body}\r\n0\r\n\r\n"
        ));
        assert_eq!(
            BoundedHttpClient::for_local_test(128)
                .post_json(&chunked, &[], &json!({}))
                .unwrap_err(),
            OutboundHttpError::ResponseTooLarge
        );

        let encoded = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: 2\r\n\r\n{}",
        );
        assert_eq!(
            BoundedHttpClient::for_local_test(128)
                .post_json(&encoded, &[], &json!({}))
                .unwrap_err(),
            OutboundHttpError::UnsupportedContentEncoding
        );

        let text = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{}",
        );
        assert_eq!(
            BoundedHttpClient::for_local_test(128)
                .post_json(&text, &[], &json!({}))
                .unwrap_err(),
            OutboundHttpError::UnexpectedContentType
        );
    }

    #[test]
    fn outbound_response_debug_redacts_json_content() {
        let response = BoundedJsonResponse {
            status_code: 200,
            request_id: Some("request-789".to_string()),
            retry_after_seconds: None,
            body: Some(json!({"secret": "response-sentinel"})),
        };

        let debug = format!("{response:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("response-sentinel"));
    }

    fn serve_once(response: &str) -> String {
        let address = spawn_server(response);
        format!("http://{address}/")
    }

    fn spawn_server(response: &str) -> SocketAddr {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("fixture listener should bind");
        let address = listener
            .local_addr()
            .expect("fixture listener address should resolve");
        let response = response.as_bytes().to_vec();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("fixture request should arrive");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            stream
                .write_all(&response)
                .expect("fixture response should write");
        });
        address
    }
}
