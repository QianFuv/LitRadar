//! Loopback contract tests for managed HTTP, HTTPS, SOCKS5, and SOCKS5h proxies.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use litradar_sources::{
    DomesticCnkiTransport, LiveCnkiConfig, LiveCnkiTransport, LiveDomesticCnkiConfig,
    LiveDomesticCnkiTransport, LiveJfbymSolver, LiveScholarlyConfig, LiveScholarlyTransport,
    LiveZjlibCnkiConfig, LiveZjlibCnkiTransport, ProviderProxy, ProviderProxyError,
    ProviderProxySelection,
};
use reqwest::blocking::{Client, ClientBuilder};

const SERVER_DEADLINE: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(3);
const CHILD_MODE_ENV: &str = "LITRADAR_PROVIDER_PROXY_CHILD_MODE";
const CHILD_TARGET_ENV: &str = "LITRADAR_PROVIDER_PROXY_CHILD_TARGET";

#[derive(Debug, PartialEq, Eq)]
enum SocksAddress {
    Ip(IpAddr),
    Domain(String),
}

#[derive(Debug)]
struct SocksCapture {
    methods: Vec<u8>,
    username: Option<String>,
    password: Option<String>,
    address: SocksAddress,
    port: u16,
    http_request: String,
}

#[test]
fn provider_proxy_locked_reqwest_contract_covers_schemes_dns_ports_and_authentication() {
    for url in [
        "http://127.0.0.1:8080",
        "https://127.0.0.1:8443",
        "socks5://127.0.0.1:1081",
        "socks5h://127.0.0.1:1081",
    ] {
        managed_client(&ProviderProxy::explicit(url).expect("supported proxy should validate"))
            .expect("supported proxy client should build");
    }

    let (http_proxy_url, http_proxy) = spawn_http_server("http-proxy-response", SERVER_DEADLINE);
    let authenticated_http_proxy = http_proxy_url.replacen("http://", "http://user:password@", 1);
    let response = managed_client(
        &ProviderProxy::explicit(authenticated_http_proxy)
            .expect("authenticated HTTP proxy should validate"),
    )
    .expect("HTTP proxy client should build")
    .get("http://provider-target.test/article")
    .send()
    .expect("HTTP proxy request should succeed")
    .text()
    .expect("HTTP proxy response should decode");
    assert_eq!(response, "http-proxy-response");
    let http_request = http_proxy
        .join()
        .expect("HTTP proxy thread should finish")
        .expect("HTTP proxy should receive one request");
    assert!(http_request.starts_with("GET http://provider-target.test/article HTTP/1.1\r\n"));
    assert!(http_request
        .to_ascii_lowercase()
        .contains("proxy-authorization: basic dxnlcjpwyxnzd29yza=="));

    let local_dns_listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("SOCKS5 listener should bind");
    let local_dns_address = local_dns_listener
        .local_addr()
        .expect("SOCKS5 listener address should resolve");
    let local_dns_server = spawn_socks_server(local_dns_listener, None);
    let local_dns_body = managed_client(
        &ProviderProxy::explicit(format!("socks5://{local_dns_address}"))
            .expect("SOCKS5 proxy should validate"),
    )
    .expect("SOCKS5 client should build")
    .get("http://localhost:45671/local-dns")
    .send()
    .expect("SOCKS5 request should succeed")
    .text()
    .expect("SOCKS5 response should decode");
    assert_eq!(local_dns_body, "socks-response");
    let local_dns_capture = local_dns_server
        .join()
        .expect("SOCKS5 server should finish");
    assert!(matches!(local_dns_capture.address, SocksAddress::Ip(_)));
    assert_eq!(local_dns_capture.port, 45671);
    assert_eq!(local_dns_capture.methods, vec![0]);
    assert!(local_dns_capture
        .http_request
        .starts_with("GET /local-dns HTTP/1.1\r\n"));

    let proxy_dns_listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("SOCKS5h listener should bind");
    let proxy_dns_address = proxy_dns_listener
        .local_addr()
        .expect("SOCKS5h listener address should resolve");
    let proxy_dns_server = spawn_socks_server(
        proxy_dns_listener,
        Some(("socks-user".to_string(), "socks-password".to_string())),
    );
    let proxy_dns_body = managed_client(
        &ProviderProxy::explicit(format!(
            "socks5h://socks-user:socks-password@{proxy_dns_address}"
        ))
        .expect("authenticated SOCKS5h proxy should validate"),
    )
    .expect("SOCKS5h client should build")
    .get("http://localhost:45672/proxy-dns")
    .send()
    .expect("SOCKS5h request should succeed")
    .text()
    .expect("SOCKS5h response should decode");
    assert_eq!(proxy_dns_body, "socks-response");
    let proxy_dns_capture = proxy_dns_server
        .join()
        .expect("SOCKS5h server should finish");
    assert_eq!(
        proxy_dns_capture.address,
        SocksAddress::Domain("localhost".to_string())
    );
    assert_eq!(proxy_dns_capture.port, 45672);
    assert!(proxy_dns_capture.methods.contains(&2));
    assert_eq!(proxy_dns_capture.username.as_deref(), Some("socks-user"));
    assert_eq!(
        proxy_dns_capture.password.as_deref(),
        Some("socks-password")
    );

    let default_port_listener = TcpListener::bind(("127.0.0.1", 1080))
        .expect("locked reqwest SOCKS default-port gate requires free loopback port 1080");
    let default_port_server = spawn_socks_server(default_port_listener, None);
    let default_port_body = managed_client(
        &ProviderProxy::explicit("socks5://127.0.0.1")
            .expect("portless SOCKS5 proxy should validate"),
    )
    .expect("portless SOCKS5 client should build")
    .get("http://localhost:45673/default-port")
    .send()
    .expect("portless SOCKS5 request should use port 1080")
    .text()
    .expect("default-port response should decode");
    assert_eq!(default_port_body, "socks-response");
    let default_port_capture = default_port_server
        .join()
        .expect("default-port SOCKS5 server should finish");
    assert_eq!(default_port_capture.port, 45673);
}

#[test]
fn provider_proxy_selection_is_per_provider_fail_closed_and_redacted() {
    let password_sentinel = "selection-proxy-password-sentinel";
    let selection = ProviderProxySelection::new(
        &format!("http://proxy-user:{password_sentinel}@127.0.0.1:8080"),
        r#"{"cnki":true,"scholarly":false,"zjlib":true}"#,
    )
    .expect("known Provider proxy selection should validate");
    assert!(selection.for_provider("cnki").is_explicit());
    assert!(selection.for_provider("zjlib").is_explicit());
    assert!(!selection.for_provider("scholarly").is_explicit());
    assert!(!selection.for_provider("cnki_oversea").is_explicit());
    assert!(selection
        .proxy_url_for_provider("cnki")
        .is_some_and(|url| url.contains(password_sentinel)));
    assert!(selection.proxy_url_for_provider("scholarly").is_none());
    assert!(!format!("{selection:?}").contains(password_sentinel));
    assert!(!format!("{:?}", selection.for_provider("cnki")).contains(password_sentinel));

    assert_eq!(
        ProviderProxySelection::new("", r#"{"cnki":true}"#)
            .expect_err("enabled Provider without URL should fail"),
        ProviderProxyError::MissingUrl
    );
    assert_eq!(
        ProviderProxySelection::new("http://127.0.0.1:8080", r#"{"unknown":true}"#)
            .expect_err("unknown Provider should fail"),
        ProviderProxyError::UnknownProvider("unknown".to_string())
    );
    for url in [
        "socks4://user:secret@127.0.0.1:1080",
        "socks4a://user:secret@127.0.0.1:1080",
        "ftp://user:secret@127.0.0.1:21",
    ] {
        let error = ProviderProxy::explicit(url).expect_err("unsupported proxy scheme should fail");
        assert_eq!(error, ProviderProxyError::InvalidUrl);
        assert!(!error.to_string().contains("secret"));
    }
}

#[test]
fn provider_proxy_constructs_every_live_provider_client_and_rebuilds_domestic() {
    let password_sentinel = "constructor-proxy-password-sentinel";
    let proxy = ProviderProxy::explicit(format!(
        "socks5h://proxy-user:{password_sentinel}@127.0.0.1:1081"
    ))
    .expect("shared Provider proxy should validate");

    LiveScholarlyTransport::new_with_proxy(
        LiveScholarlyConfig::from_value_pools(1, "", "", ""),
        proxy.clone(),
    )
    .expect("Scholarly client should accept the managed proxy");
    LiveCnkiTransport::new_with_proxy(LiveCnkiConfig { timeout_seconds: 1 }, proxy.clone())
        .expect("CNKI Overseas client should accept the managed proxy");
    let mut domestic = LiveDomesticCnkiTransport::new_with_proxy(
        LiveDomesticCnkiConfig {
            timeout_seconds: 1,
            captcha_token: None,
        },
        proxy.clone(),
    )
    .expect("domestic CNKI client should accept the managed proxy");
    domestic
        .reset_transient_state()
        .expect("domestic CNKI rebuild should retain the managed proxy");
    LiveJfbymSolver::new_with_proxy("captcha-token", 1, proxy.clone())
        .expect("JFBYM solver should accept the CNKI-managed proxy");
    LiveZjlibCnkiTransport::new_with_proxy(LiveZjlibCnkiConfig::default(), proxy.clone())
        .expect("ZJLib redirect and no-redirect clients should accept the managed proxy");

    assert!(!format!("{proxy:?}").contains(password_sentinel));
}

#[test]
fn provider_proxy_disabled_ignores_ambient_http_proxy() {
    if std::env::var_os(CHILD_MODE_ENV).is_some() {
        let target = std::env::var(CHILD_TARGET_ENV)
            .expect("child target URL should be supplied by the parent");
        let ambient_body = ClientBuilder::new()
            .timeout(CLIENT_TIMEOUT)
            .build()
            .expect("ambient client should build")
            .get(&target)
            .send()
            .expect("ambient client should reach the environment proxy")
            .text()
            .expect("ambient proxy response should decode");
        assert_eq!(ambient_body, "ambient-proxy-response");
        let direct_body = managed_client(&ProviderProxy::direct())
            .expect("managed direct client should build")
            .get(&target)
            .send()
            .expect("managed direct client should bypass the environment proxy")
            .text()
            .expect("direct target response should decode");
        assert_eq!(direct_body, "direct-target-response");
        return;
    }

    let (target_url, target_server) = spawn_http_server("direct-target-response", SERVER_DEADLINE);
    let (ambient_proxy_url, ambient_proxy_server) =
        spawn_http_server("ambient-proxy-response", SERVER_DEADLINE);
    let status = Command::new(
        std::env::current_exe().expect("current integration test executable should resolve"),
    )
    .args([
        "--exact",
        "provider_proxy_disabled_ignores_ambient_http_proxy",
        "--nocapture",
    ])
    .env(CHILD_MODE_ENV, "1")
    .env(CHILD_TARGET_ENV, &target_url)
    .env("HTTP_PROXY", &ambient_proxy_url)
    .env("http_proxy", &ambient_proxy_url)
    .env("HTTPS_PROXY", &ambient_proxy_url)
    .env("https_proxy", &ambient_proxy_url)
    .env("NO_PROXY", "")
    .env("no_proxy", "")
    .status()
    .expect("isolated ambient-proxy child should run");
    assert!(status.success());

    let target_request = target_server
        .join()
        .expect("target server should finish")
        .expect("managed direct client should reach the target");
    let ambient_proxy_request = ambient_proxy_server
        .join()
        .expect("ambient proxy server should finish")
        .expect("ordinary client should reach the environment proxy");
    assert!(target_request.starts_with("GET / HTTP/1.1\r\n"));
    assert!(ambient_proxy_request.starts_with(&format!("GET {target_url} HTTP/1.1\r\n")));
}

#[test]
fn provider_proxy_unreachable_explicit_proxy_does_not_fallback_or_leak() {
    let target_listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("direct target listener should bind");
    target_listener
        .set_nonblocking(true)
        .expect("direct target listener should become nonblocking");
    let target_address = target_listener
        .local_addr()
        .expect("direct target address should resolve");
    let closed_proxy_listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("proxy port reservation should bind");
    let closed_proxy_address = closed_proxy_listener
        .local_addr()
        .expect("reserved proxy address should resolve");
    drop(closed_proxy_listener);

    let password_sentinel = "unreachable-proxy-password-sentinel";
    let client = managed_client(
        &ProviderProxy::explicit(format!(
            "http://proxy-user:{password_sentinel}@{closed_proxy_address}"
        ))
        .expect("unreachable HTTP proxy should still validate"),
    )
    .expect("unreachable HTTP proxy client should build");
    let error = client
        .get(format!("http://{target_address}/must-not-connect"))
        .send()
        .expect_err("unreachable explicit proxy should fail");
    assert!(!error.to_string().contains(password_sentinel));
    thread::sleep(Duration::from_millis(50));
    assert!(target_listener.accept().is_err());
}

fn managed_client(proxy: &ProviderProxy) -> Result<Client, ProviderProxyError> {
    proxy
        .apply(
            ClientBuilder::new()
                .timeout(CLIENT_TIMEOUT)
                .resolve("localhost", SocketAddr::from(([127, 0, 0, 1], 0))),
        )?
        .build()
        .map_err(|_| ProviderProxyError::InvalidUrl)
}

fn spawn_http_server(
    response_body: &str,
    deadline: Duration,
) -> (String, thread::JoinHandle<Option<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("HTTP listener should bind");
    let address = listener
        .local_addr()
        .expect("HTTP listener address should resolve");
    listener
        .set_nonblocking(true)
        .expect("HTTP listener should become nonblocking");
    let response_body = response_body.to_string();
    let server = thread::spawn(move || {
        let mut stream = accept_until(&listener, deadline)?;
        stream
            .set_nonblocking(false)
            .expect("HTTP stream should become blocking");
        stream
            .set_read_timeout(Some(CLIENT_TIMEOUT))
            .expect("HTTP stream read timeout should configure");
        let request = read_http_head(&mut stream).expect("HTTP request should be readable");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("HTTP response should write");
        Some(request)
    });
    (format!("http://{address}/"), server)
}

fn spawn_socks_server(
    listener: TcpListener,
    expected_credentials: Option<(String, String)>,
) -> thread::JoinHandle<SocksCapture> {
    listener
        .set_nonblocking(true)
        .expect("SOCKS listener should become nonblocking");
    thread::spawn(move || {
        let mut stream =
            accept_until(&listener, SERVER_DEADLINE).expect("SOCKS client should connect");
        stream
            .set_nonblocking(false)
            .expect("SOCKS stream should become blocking");
        stream
            .set_read_timeout(Some(CLIENT_TIMEOUT))
            .expect("SOCKS stream read timeout should configure");
        let mut greeting = [0_u8; 2];
        stream
            .read_exact(&mut greeting)
            .expect("SOCKS greeting should read");
        assert_eq!(greeting[0], 5);
        let mut methods = vec![0_u8; usize::from(greeting[1])];
        stream
            .read_exact(&mut methods)
            .expect("SOCKS methods should read");
        let selected_method = if expected_credentials.is_some() { 2 } else { 0 };
        assert!(methods.contains(&selected_method));
        stream
            .write_all(&[5, selected_method])
            .expect("SOCKS method selection should write");

        let (username, password) =
            if let Some((expected_username, expected_password)) = expected_credentials {
                let mut auth_header = [0_u8; 2];
                stream
                    .read_exact(&mut auth_header)
                    .expect("SOCKS auth header should read");
                assert_eq!(auth_header[0], 1);
                let mut username = vec![0_u8; usize::from(auth_header[1])];
                stream
                    .read_exact(&mut username)
                    .expect("SOCKS username should read");
                let mut password_length = [0_u8; 1];
                stream
                    .read_exact(&mut password_length)
                    .expect("SOCKS password length should read");
                let mut password = vec![0_u8; usize::from(password_length[0])];
                stream
                    .read_exact(&mut password)
                    .expect("SOCKS password should read");
                let username = String::from_utf8(username).expect("SOCKS username should be UTF-8");
                let password = String::from_utf8(password).expect("SOCKS password should be UTF-8");
                assert_eq!(username, expected_username);
                assert_eq!(password, expected_password);
                stream
                    .write_all(&[1, 0])
                    .expect("SOCKS auth success should write");
                (Some(username), Some(password))
            } else {
                (None, None)
            };

        let mut request_header = [0_u8; 4];
        stream
            .read_exact(&mut request_header)
            .expect("SOCKS connect header should read");
        assert_eq!(&request_header[..3], &[5, 1, 0]);
        let address = match request_header[3] {
            1 => {
                let mut octets = [0_u8; 4];
                stream
                    .read_exact(&mut octets)
                    .expect("SOCKS IPv4 address should read");
                SocksAddress::Ip(IpAddr::V4(Ipv4Addr::from(octets)))
            }
            3 => {
                let mut length = [0_u8; 1];
                stream
                    .read_exact(&mut length)
                    .expect("SOCKS domain length should read");
                let mut domain = vec![0_u8; usize::from(length[0])];
                stream
                    .read_exact(&mut domain)
                    .expect("SOCKS domain should read");
                SocksAddress::Domain(
                    String::from_utf8(domain).expect("SOCKS domain should be UTF-8"),
                )
            }
            4 => {
                let mut octets = [0_u8; 16];
                stream
                    .read_exact(&mut octets)
                    .expect("SOCKS IPv6 address should read");
                SocksAddress::Ip(IpAddr::V6(Ipv6Addr::from(octets)))
            }
            address_type => panic!("unexpected SOCKS address type {address_type}"),
        };
        let mut port_bytes = [0_u8; 2];
        stream
            .read_exact(&mut port_bytes)
            .expect("SOCKS destination port should read");
        let port = u16::from_be_bytes(port_bytes);
        stream
            .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
            .expect("SOCKS connect success should write");
        let http_request =
            read_http_head(&mut stream).expect("HTTP request through SOCKS should read");
        let response_body = "socks-response";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("SOCKS HTTP response should write");
        SocksCapture {
            methods,
            username,
            password,
            address,
            port,
            http_request,
        }
    })
}

fn accept_until(listener: &TcpListener, timeout: Duration) -> Option<TcpStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("listener accept failed: {error}"),
        }
    }
}

fn read_http_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while bytes.len() < 65_536 {
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "HTTP request headers exceeded the test bound",
    ))
}
