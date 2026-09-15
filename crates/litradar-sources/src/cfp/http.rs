//! Deadline-bound public HTTP GETs with strict original-text decoding.

use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use encoding_rs::{Encoding, UTF_8};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::{redirect::Policy, Url};
use tokio::sync::Semaphore;

use super::{CfpDocument, CfpSourceConfig, CfpSourceError, CfpTransport, CFP_MAX_PAGE_BYTES};

/// A bounded raw response for optional PDF extraction by the worker.
pub struct CfpHttpDocument {
    /// Validated final publisher URL.
    pub final_url: String,
    /// Original response content type.
    pub content_type: String,
    /// Complete bounded bytes, with no lossy UTF-8 substitution.
    pub bytes: Vec<u8>,
}

/// HTTP transport with redirects and DNS constrained to public registered sources.
pub struct CfpHttpTransport {
    client: Client,
    #[cfg(test)]
    is_fixture: bool,
}

struct PublicResolver;
impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        resolve_with_lookup(move || {
            (host.as_str(), 0)
                .to_socket_addrs()
                .map(|addresses| addresses.collect())
        })
    }
}

/// Bound uncancellable OS lookups independently of the request runtime's teardown.
fn resolve_with_lookup(
    lookup: impl FnOnce() -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
) -> Resolving {
    Box::pin(async move {
        static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let permit = SLOTS
            .get_or_init(|| Arc::new(Semaphore::new(8)))
            .clone()
            .acquire_owned()
            .await?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("cfp-dns".into())
            .spawn(move || {
                let result = lookup();
                drop(permit);
                let _ = sender.send(result);
            })?;
        let addresses = receiver.await??;
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|address| !is_public_address(address.ip()))
        {
            return Err(
                Box::new(CfpSourceError::DisallowedUrl) as Box<dyn std::error::Error + Send + Sync>
            );
        }
        Ok(Box::new(addresses.into_iter()) as Addrs)
    })
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
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
                || (octets[0] == 198 && matches!(octets[1], 18 | 19))
                || octets[0] >= 240)
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4() {
                return is_public_address(IpAddr::V4(mapped));
            }
            let segments = address.segments();
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && (segments[1] <= 0x01ff || segments[1] == 0x0db8))
                && !(segments[0] == 0x3fff && (segments[1] & 0xf000) == 0)
                && segments[0] != 0x2002
                && !(segments[0] == 0x2620 && segments[1] == 0x004f && segments[2] == 0x8000)
        }
    }
}

impl CfpHttpTransport {
    /// Construct a direct public-source client with automatic redirects disabled.
    pub fn new() -> Result<Self, CfpSourceError> {
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .dns_resolver(Arc::new(PublicResolver))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .user_agent("LitRadar/CFP (+original publisher announcements)")
            .build()
            .map_err(|_| CfpSourceError::Request)?;
        Ok(Self {
            client,
            #[cfg(test)]
            is_fixture: false,
        })
    }

    /// Fetch complete bounded bytes, validating every redirect before requesting it.
    pub fn fetch_bytes(
        &self,
        config: &CfpSourceConfig,
        value: &str,
        deadline: Instant,
    ) -> Result<CfpHttpDocument, CfpSourceError> {
        let mut url = Url::parse(value).map_err(|_| CfpSourceError::DisallowedUrl)?;
        for redirect in 0..=5 {
            self.validate_url(config, &url)?;
            let mut response = None;
            for attempt in 0..2 {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .filter(|duration| !duration.is_zero())
                    .ok_or(CfpSourceError::Deadline)?;
                match self
                    .client
                    .get(url.clone())
                    .timeout(remaining.min(Duration::from_secs(20)))
                    .header("Accept", "text/html,application/pdf,text/plain;q=0.8")
                    .send()
                {
                    Ok(value) => {
                        response = Some(value);
                        break;
                    }
                    Err(error) if error.is_timeout() => {
                        if Instant::now() >= deadline {
                            return Err(CfpSourceError::Deadline);
                        }
                        if attempt == 1 {
                            return Err(CfpSourceError::Request);
                        }
                    }
                    Err(error) => {
                        let mut cause = std::error::Error::source(&error);
                        while let Some(value) = cause {
                            if value.downcast_ref::<CfpSourceError>()
                                == Some(&CfpSourceError::DisallowedUrl)
                            {
                                return Err(CfpSourceError::DisallowedUrl);
                            }
                            cause = value.source();
                        }
                        if attempt == 1 {
                            return Err(CfpSourceError::Request);
                        }
                    }
                }
            }
            let response = response.ok_or(CfpSourceError::Request)?;
            if response.status().is_redirection() {
                if redirect == 5 {
                    return Err(CfpSourceError::DisallowedUrl);
                }
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .ok_or(CfpSourceError::DisallowedUrl)?;
                url = url
                    .join(location)
                    .map_err(|_| CfpSourceError::DisallowedUrl)?;
                continue;
            }
            if !response.status().is_success() {
                return Err(CfpSourceError::HttpStatus(response.status().as_u16()));
            }
            if response
                .content_length()
                .is_some_and(|length| length > CFP_MAX_PAGE_BYTES as u64)
            {
                return Err(CfpSourceError::TooLarge);
            }
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let mut bytes = Vec::new();
            response
                .take(CFP_MAX_PAGE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| {
                    if Instant::now() >= deadline {
                        CfpSourceError::Deadline
                    } else {
                        CfpSourceError::Request
                    }
                })?;
            if bytes.len() > CFP_MAX_PAGE_BYTES {
                return Err(CfpSourceError::TooLarge);
            }
            if Instant::now() >= deadline {
                return Err(CfpSourceError::Deadline);
            }
            return Ok(CfpHttpDocument {
                final_url: url.to_string(),
                content_type,
                bytes,
            });
        }
        Err(CfpSourceError::DisallowedUrl)
    }

    fn validate_url(&self, config: &CfpSourceConfig, url: &Url) -> Result<(), CfpSourceError> {
        #[cfg(test)]
        if self.is_fixture
            && url.host_str() == Some("127.0.0.1")
            && config
                .allowed_urls
                .iter()
                .any(|rule| rule.host == "127.0.0.1")
        {
            return Ok(());
        }
        if !config.permits_url(url) {
            return Err(CfpSourceError::DisallowedUrl);
        }
        if let Some(address) = url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
        {
            if !is_public_address(address) {
                return Err(CfpSourceError::DisallowedUrl);
            }
        }
        Ok(())
    }
}

impl CfpTransport for CfpHttpTransport {
    fn fetch(
        &self,
        config: &CfpSourceConfig,
        url: &str,
        deadline: Instant,
    ) -> Result<CfpDocument, CfpSourceError> {
        let response = self.fetch_bytes(config, url, deadline)?;
        if response.bytes.starts_with(b"%PDF-")
            || response
                .content_type
                .to_ascii_lowercase()
                .contains("application/pdf")
        {
            return Err(CfpSourceError::ContentType);
        }
        let text = decode_cfp_body(&response.bytes, &response.content_type)?;
        Ok(CfpDocument {
            final_url: response.final_url,
            text,
            format: "html".into(),
        })
    }
}

/// Decode BOM, HTTP charset or HTML meta charset, rejecting invalid byte sequences.
pub fn decode_cfp_body(bytes: &[u8], content_type: &str) -> Result<String, CfpSourceError> {
    if bytes.len() > CFP_MAX_PAGE_BYTES {
        return Err(CfpSourceError::TooLarge);
    }
    let charset = Regex::new(r#"(?i)charset\s*=\s*["']?\s*([a-z0-9_-]+)"#).expect("charset regex");
    let header = charset
        .captures(content_type)
        .map(|capture| capture[1].to_owned());
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let meta = Regex::new(r"(?is)<meta\b[^>]*>")
        .expect("meta regex")
        .find_iter(&prefix)
        .find_map(|tag| {
            charset
                .captures(tag.as_str())
                .map(|capture| capture[1].to_owned())
        });
    let (encoding, skip) = if let Some((encoding, skip)) = Encoding::for_bom(bytes) {
        (encoding, skip)
    } else {
        let label = header.or(meta);
        let encoding = match label {
            Some(label) => Encoding::for_label(label.as_bytes()).ok_or(CfpSourceError::Encoding)?,
            None => UTF_8,
        };
        (encoding, 0)
    };
    encoding
        .decode_without_bom_handling_and_without_replacement(&bytes[skip..])
        .map(|text| text.into_owned())
        .ok_or(CfpSourceError::Encoding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfp::{CfpAdapter, CfpUrlRule};

    #[test]
    fn cfp_request_timeout_and_client_drop_do_not_wait_for_slow_os_dns() {
        struct SlowResolver;
        impl Resolve for SlowResolver {
            fn resolve(&self, _name: Name) -> Resolving {
                resolve_with_lookup(|| {
                    std::thread::sleep(Duration::from_secs(2));
                    Ok(vec!["1.1.1.1:0".parse().unwrap()])
                })
            }
        }
        let started = Instant::now();
        let client = Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(SlowResolver))
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        assert!(client
            .get("http://cfp-delayed-dns.invalid/")
            .send()
            .unwrap_err()
            .is_timeout());
        drop(client);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "request teardown waited for an uncancellable OS lookup"
        );
    }
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn cfp_decode_preserves_utf8_gb18030_and_rejects_lossy_fallback() {
        let original = "<meta charset='gb18030'><h1>原文征稿：人工智能</h1>";
        let (bytes, _, errors) = encoding_rs::GB18030.encode(original);
        assert!(!errors);
        assert_eq!(decode_cfp_body(&bytes, "text/html").unwrap(), original);
        assert_eq!(
            decode_cfp_body(
                "Original English 原文".as_bytes(),
                "text/html; charset=UTF-8"
            )
            .unwrap(),
            "Original English 原文"
        );
        assert_eq!(
            decode_cfp_body(&[0xff, 0xfe, 0x61], "text/html"),
            Err(CfpSourceError::Encoding)
        );
        assert_eq!(
            decode_cfp_body(&[0xff], "text/html"),
            Err(CfpSourceError::Encoding)
        );
    }

    #[test]
    fn cfp_transport_rejects_private_and_mapped_addresses_and_foreign_redirects() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "2002:7f00:1::",
        ] {
            assert!(!is_public_address(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_address("1.1.1.1".parse().unwrap()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
            write!(stream,"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let config = CfpSourceConfig {
            source_key: "fixture".into(),
            catalog_ids: vec!["fixture".into()],
            journal_title: "Fixture".into(),
            discovery_url: format!("http://{address}/calls"),
            adapter: CfpAdapter::ElsevierCalls,
            config_version: 1,
            allowed_urls: vec![CfpUrlRule {
                host: "127.0.0.1".into(),
                path_prefix: "/calls".into(),
            }],
            identity_texts: vec!["Fixture".into()],
            empty_statements: Vec::new(),
            capability_note: None,
            retains_previous_notices: false,
        };
        let transport = CfpHttpTransport {
            client: Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .build()
                .unwrap(),
            is_fixture: true,
        };
        assert!(matches!(
            transport.fetch_bytes(
                &config,
                &config.discovery_url,
                Instant::now() + Duration::from_secs(2)
            ),
            Err(CfpSourceError::DisallowedUrl)
        ));
        server.join().unwrap();
    }
}
