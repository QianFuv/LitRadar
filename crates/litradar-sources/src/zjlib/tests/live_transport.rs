use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use reqwest::header::{ORIGIN, REFERER};

use crate::scholarly::test_support::CapturedLogs;

use super::super::{
    cnki_form_headers, www_headers, LiveZjlibCnkiConfig, LiveZjlibCnkiEndpoints,
    LiveZjlibCnkiTransport, ZhejiangLibraryCnkiClient, ZjlibCnkiArticleIdentity, ZjlibCnkiError,
    ZjlibCnkiTransport, ENTRY_URL, LIBRARY_REFER, SHARE_BASE_URL, WWW_BASE_URL, ZYPROXY_BASE_URL,
    ZYPROXY_LOGIN_HOST,
};

const ARTICLE_TITLE: &str = "LoopbackArticle";
const ARTICLE_AUTHORS: &str = "Ada Lovelace; Grace Hopper";
const ARTICLE_JOURNAL: &str = "LoopbackJournal";
const COOKIE_SENTINEL: &str = "COOKIE_SENTINEL_8F92";
const CREDENTIAL_SENTINEL: &str = "CREDENTIAL_SENTINEL_1A37";
const QUERY_SENTINEL: &str = "QUERY_SENTINEL_4D61";
const TITLE_SENTINEL: &str = "TITLE_SENTINEL_2C85";
const DOI_SENTINEL: &str = "DOI_SENTINEL_10_1234";
const BODY_SENTINEL: &str = "BODY_SENTINEL_7B43";
const LOCAL_PATH_SENTINEL: &str = "LOCAL_PATH_SENTINEL_C_DRIVE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureMode {
    Success,
    RetryThenSuccess,
    UnsafeRedirect,
    RedirectLimit,
    MissingSessionCookie,
    MalformedQr,
    SlowQr,
    MetadataMismatch,
    InvalidPdf,
    OversizedPdf,
    SensitivePdfFailure,
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Debug)]
struct FixtureState {
    base_url: String,
    mode: FixtureMode,
    proxy_entry_count: usize,
    requests: Vec<CapturedRequest>,
}

#[derive(Debug)]
struct LoopbackServer {
    address: SocketAddr,
    base_url: String,
    is_stopping: Arc<AtomicBool>,
    state: Arc<Mutex<FixtureState>>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
    thread: Option<JoinHandle<()>>,
}

impl LoopbackServer {
    fn start(mode: FixtureMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener should bind");
        listener
            .set_nonblocking(true)
            .expect("loopback listener should become nonblocking");
        let address = listener
            .local_addr()
            .expect("loopback address should resolve");
        let base_url = format!("http://{address}");
        let is_stopping = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(FixtureState {
            base_url: base_url.clone(),
            mode,
            proxy_entry_count: 0,
            requests: Vec::new(),
        }));
        let workers = Arc::new(Mutex::new(Vec::new()));
        let server_stopping = is_stopping.clone();
        let server_state = state.clone();
        let server_workers = workers.clone();
        let thread = thread::spawn(move || {
            while !server_stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if server_stopping.load(Ordering::SeqCst) {
                            break;
                        }
                        stream
                            .set_nonblocking(false)
                            .expect("loopback stream should become blocking");
                        let worker_state = server_state.clone();
                        let worker = thread::spawn(move || {
                            while let Some(request) = read_request(&mut stream) {
                                let response = respond(&worker_state, request);
                                if !write_response(&mut stream, response) {
                                    break;
                                }
                            }
                        });
                        server_workers
                            .lock()
                            .expect("worker list should lock")
                            .push(worker);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            base_url,
            is_stopping,
            state,
            workers,
            thread: Some(thread),
        }
    }

    fn transport(&self, maximum_document_bytes: usize) -> LiveZjlibCnkiTransport {
        LiveZjlibCnkiTransport::new_for_loopback(
            LiveZjlibCnkiConfig {
                timeout_seconds: 1,
                maximum_document_bytes,
            },
            &self.base_url,
        )
        .expect("loopback transport should build")
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.state
            .lock()
            .expect("fixture state should lock")
            .requests
            .clone()
    }

    fn transcript(&self) -> String {
        self.requests()
            .into_iter()
            .map(|request| {
                format!(
                    "{} {} {:?} {}",
                    request.method, request.target, request.headers, request.body
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.is_stopping.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("loopback server should stop cleanly");
        }
        for worker in self
            .workers
            .lock()
            .expect("worker list should lock")
            .drain(..)
        {
            worker.join().expect("loopback worker should stop cleanly");
        }
    }
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    content_type: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay: Duration,
}

impl HttpResponse {
    fn ok(content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type,
            headers: Vec::new(),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn redirect(location: String) -> Self {
        Self {
            status: 302,
            content_type: "text/plain",
            headers: vec![("Location".to_string(), location)],
            body: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

fn read_request(stream: &mut TcpStream) -> Option<CapturedRequest> {
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("request read timeout should configure");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected_length = None;
    loop {
        let read_length = match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read_length) => read_length,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                break;
            }
            Err(error) => panic!("loopback request read failed: {error}"),
        };
        bytes.extend_from_slice(&buffer[..read_length]);
        if expected_length.is_none() {
            if let Some(header_end) = find_header_end(&bytes) {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                expected_length = Some(header_end + 4 + content_length);
            }
        }
        if expected_length.is_some_and(|length| bytes.len() >= length) {
            break;
        }
    }
    let header_end = find_header_end(&bytes)?;
    let headers_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = headers_text.lines();
    let request_line = lines.next()?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next()?.to_string();
    let target = request_parts.next()?.to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    Some(CapturedRequest {
        method,
        target,
        headers,
        body: String::from_utf8_lossy(&bytes[header_end + 4..]).to_string(),
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn respond(state: &Arc<Mutex<FixtureState>>, request: CapturedRequest) -> HttpResponse {
    let mut state = state.lock().expect("fixture state should lock");
    let path = request
        .target
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    state.requests.push(request);
    match path.as_str() {
        "/www/bff-api/reader-sso-service/portal-pc-api/login/zfb-qr" => {
            if state.mode == FixtureMode::MalformedQr {
                return HttpResponse::ok("application/json", b"{not-json".to_vec());
            }
            let response = HttpResponse::ok(
                "application/json",
                br#"{"success":true,"data":{"uuid":"loopback-uuid","qrCode":"loopback-qr","status":"WAITING_SCAN"}}"#.to_vec(),
            );
            if state.mode == FixtureMode::SlowQr {
                response.with_delay(Duration::from_millis(1_250))
            } else {
                response
            }
        }
        "/www/bff-api/reader-sso-service/portal-pc-api/qr/status" => HttpResponse::ok(
            "application/json",
            format!(
                r#"{{"success":true,"data":{{"status":"COMPLETE","data":"{CREDENTIAL_SENTINEL}"}}}}"#
            ),
        ),
        "/www/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl" => {
            HttpResponse::ok(
                "application/json",
                format!(
                    r#"{{"success":true,"data":"{}/share/protocol-auth?ticket={CREDENTIAL_SENTINEL}"}}"#,
                    state.base_url
                ),
            )
        }
        "/share/protocol-auth" => HttpResponse::ok(
            "text/html",
            format!(
                r#"<script>var sign = "share-sign"; var url = "{0}/share/entry/area/35594/2120"; var domainUrl = "{0}"; var portalContextPath = "/share";</script><form action="/share/sso-login/cookie/sync"></form>"#,
                state.base_url
            ),
        ),
        "/share/sso-login/cookie/sync" => HttpResponse::ok("text/plain", "synced")
            .with_header("Set-Cookie", "share_sid=share-cookie; Path=/; HttpOnly"),
        "/share/entry/area/35594/2120" | "/share/engine2/header/user-info" => {
            HttpResponse::ok("text/html", "ready")
        }
        "/share/sso/api/auth/library/vpn358" => match state.mode {
            FixtureMode::UnsafeRedirect => {
                HttpResponse::redirect("http://example.invalid/kns55/".to_string())
            }
            FixtureMode::RedirectLimit => {
                HttpResponse::redirect(format!("{}/login/step-0", state.base_url))
            }
            _ => HttpResponse::redirect(format!(
                "{}/login/index.php?enc={QUERY_SENTINEL}&username={CREDENTIAL_SENTINEL}",
                state.base_url
            )),
        },
        "/login/index.php" => HttpResponse::redirect(format!("{}/proxy/kns55/", state.base_url)),
        "/proxy/kns55/" => {
            state.proxy_entry_count += 1;
            if state.mode == FixtureMode::RetryThenSuccess && state.proxy_entry_count == 1 {
                return HttpResponse::redirect(format!("{}/login/index.php", state.base_url));
            }
            let response = HttpResponse::ok("text/html", "proxy ready");
            if state.mode == FixtureMode::MissingSessionCookie {
                response
            } else {
                response.with_header(
                    "Set-Cookie",
                    &format!("vpn358_sid={COOKIE_SENTINEL}; Path=/; HttpOnly"),
                )
            }
        }
        "/login/step-0" => HttpResponse::redirect(format!("{}/proxy/step-1", state.base_url)),
        "/proxy/step-1" => HttpResponse::redirect(format!("{}/login/step-2", state.base_url)),
        "/login/step-2" => HttpResponse::redirect(format!("{}/proxy/step-3", state.base_url)),
        "/proxy/step-3" => HttpResponse::redirect(format!("{}/login/step-4", state.base_url)),
        "/login/step-4" => HttpResponse::redirect(format!("{}/proxy/step-5", state.base_url)),
        "/proxy/kns55/brief/result.aspx" | "/proxy/kns55/request/SearchHandler.ashx" => {
            HttpResponse::ok("text/html", "accepted")
        }
        "/proxy/kns55/brief/brief.aspx" => HttpResponse::ok(
            "text/html",
            format!(
                r#"<table><tr><td><a href="{}/proxy/kns55/detail/detail.aspx?FileName=loopback&amp;DbName=CJFDLAST2026&amp;DbCode=CJFD">{}</a></td></tr></table>"#,
                state.base_url,
                article_title(state.mode)
            ),
        ),
        "/proxy/kns55/detail/detail.aspx" => HttpResponse::ok(
            "text/html",
            format!(
                r#"<html><head><meta name="citation_title" content="{}"><meta name="citation_author" content="{}"><meta name="citation_journal_title" content="{}"></head><body><a href="{}/proxy/kcms/download.aspx?filename={QUERY_SENTINEL}&amp;token={CREDENTIAL_SENTINEL}&amp;doi={DOI_SENTINEL}&amp;title={TITLE_SENTINEL}&amp;path={LOCAL_PATH_SENTINEL}&amp;dflag=pdfdown">PDF</a></body></html>"#,
                article_title(state.mode),
                article_authors(state.mode),
                article_journal(state.mode),
                state.base_url
            ),
        ),
        "/proxy/kcms/download.aspx" => match state.mode {
            FixtureMode::InvalidPdf => HttpResponse::ok("text/html", BODY_SENTINEL),
            FixtureMode::OversizedPdf => {
                HttpResponse::ok("application/pdf", b"%PDF-oversized-document".to_vec())
            }
            FixtureMode::SensitivePdfFailure => HttpResponse {
                status: 500,
                content_type: "text/plain",
                headers: Vec::new(),
                body: BODY_SENTINEL.as_bytes().to_vec(),
                delay: Duration::ZERO,
            },
            _ => HttpResponse::ok("application/pdf", b"%PDF-1.7\nloopback\n".to_vec()),
        },
        _ => HttpResponse {
            status: 404,
            content_type: "text/plain",
            headers: Vec::new(),
            body: b"not found".to_vec(),
            delay: Duration::ZERO,
        },
    }
}

fn article_title(mode: FixtureMode) -> &'static str {
    if mode == FixtureMode::SensitivePdfFailure {
        TITLE_SENTINEL
    } else if mode == FixtureMode::MetadataMismatch {
        "DifferentArticle"
    } else {
        ARTICLE_TITLE
    }
}

fn article_authors(mode: FixtureMode) -> &'static str {
    if mode == FixtureMode::SensitivePdfFailure {
        LOCAL_PATH_SENTINEL
    } else {
        ARTICLE_AUTHORS
    }
}

fn article_journal(mode: FixtureMode) -> &'static str {
    if mode == FixtureMode::SensitivePdfFailure {
        DOI_SENTINEL
    } else {
        ARTICLE_JOURNAL
    }
}

fn write_response(stream: &mut TcpStream, response: HttpResponse) -> bool {
    if !response.delay.is_zero() {
        thread::sleep(response.delay);
    }
    let reason = match response.status {
        200 => "OK",
        302 => "Found",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Response",
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    for (name, value) in response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).is_ok()
        && stream.write_all(&response.body).is_ok()
        && stream.flush().is_ok()
}

fn logged_in_client(
    server: &LoopbackServer,
    maximum_document_bytes: usize,
) -> ZhejiangLibraryCnkiClient<LiveZjlibCnkiTransport> {
    let mut client = ZhejiangLibraryCnkiClient::new(server.transport(maximum_document_bytes));
    let login = client.start_qr_login().expect("QR start should succeed");
    assert_eq!(login.uuid, "loopback-uuid");
    let token = client
        .poll_qr_login(1, 0.1)
        .expect("QR poll should complete");
    assert_eq!(token, CREDENTIAL_SENTINEL);
    client
}

fn warmed_client(
    server: &LoopbackServer,
    maximum_document_bytes: usize,
) -> ZhejiangLibraryCnkiClient<LiveZjlibCnkiTransport> {
    let mut client = logged_in_client(server, maximum_document_bytes);
    if let Err(error) = client.warm_up_fulltext_session() {
        panic!(
            "full-text warm-up should succeed: {error}; transcript: {}",
            server.transcript()
        );
    }
    client
}

fn identity() -> ZjlibCnkiArticleIdentity {
    ZjlibCnkiArticleIdentity {
        title: ARTICLE_TITLE.to_string(),
        authors: ARTICLE_AUTHORS.to_string(),
        journal_title: ARTICLE_JOURNAL.to_string(),
    }
}

#[test]
fn production_endpoint_defaults_remain_exact() {
    let endpoints = LiveZjlibCnkiEndpoints::default();

    assert_eq!(endpoints.www_base_url.as_str(), format!("{WWW_BASE_URL}/"));
    assert_eq!(
        endpoints.share_base_url.as_str(),
        format!("{SHARE_BASE_URL}/")
    );
    assert_eq!(
        endpoints.zyproxy_base_url.as_str(),
        format!("{ZYPROXY_BASE_URL}/")
    );
    assert_eq!(
        endpoints.zyproxy_login_base_url.as_str(),
        format!("https://{ZYPROXY_LOGIN_HOST}/")
    );
    assert_eq!(endpoints.entry_url.as_str(), ENTRY_URL);
    assert_eq!(endpoints.library_refer.as_str(), LIBRARY_REFER);
    assert_eq!(
        www_headers(&endpoints.www_base_url, None)[REFERER],
        WWW_BASE_URL
    );
    assert_eq!(
        cnki_form_headers(&format!("{ZYPROXY_BASE_URL}/kns55/"), ZYPROXY_BASE_URL)[ORIGIN],
        ZYPROXY_BASE_URL
    );
}

#[test]
fn loopback_transport_completes_login_warmup_search_metadata_and_pdf() {
    let server = LoopbackServer::start(FixtureMode::Success);
    let mut client = warmed_client(&server, 1_024);
    let downloaded = client
        .download_matching_pdf(&identity(), 3)
        .expect("loopback PDF should download");

    assert_eq!(downloaded.filename, format!("{ARTICLE_TITLE}.pdf"));
    assert_eq!(downloaded.content_type, "application/pdf");
    assert!(downloaded.content.starts_with(b"%PDF"));
    let state = client.to_state_data();
    assert!(state["final_zyproxy_url"]
        .as_str()
        .is_some_and(|url| url.ends_with("/proxy/kns55/")));
    let requests = server.requests();
    assert!(requests.iter().any(|request| {
        request.method == "POST"
            && request.target == "/proxy/kns55/brief/result.aspx"
            && request.body.contains(ARTICLE_TITLE)
    }));
    assert!(requests.iter().any(|request| {
        request
            .target
            .starts_with("/www/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl?")
            && request
                .headers
                .get("bff-user-token")
                .is_some_and(|value| value == CREDENTIAL_SENTINEL)
    }));
    assert!(requests.iter().any(|request| {
        request.target.starts_with("/proxy/kns55/brief/brief.aspx?")
            && request
                .headers
                .get("cookie")
                .is_some_and(|value| value.contains(COOKIE_SENTINEL))
    }));
}

#[test]
fn loopback_transport_recovers_expected_retry_and_rejects_redirect_abuse() {
    let retry_server = LoopbackServer::start(FixtureMode::RetryThenSuccess);
    let mut retry_client = logged_in_client(&retry_server, 1_024);
    let final_url = retry_client
        .warm_up_fulltext_session()
        .expect("expected zyproxy loop should retry successfully");
    assert!(final_url.ends_with("/proxy/kns55/"));
    assert_eq!(
        retry_server
            .requests()
            .iter()
            .filter(|request| request.target.starts_with("/login/index.php"))
            .count(),
        2
    );

    let unsafe_server = LoopbackServer::start(FixtureMode::UnsafeRedirect);
    let mut unsafe_client = logged_in_client(&unsafe_server, 1_024);
    let unsafe_error = unsafe_client
        .warm_up_fulltext_session()
        .expect_err("external redirect should be rejected");
    assert!(unsafe_error.to_string().contains("unexpected endpoint"));
    assert!(!unsafe_server.transcript().contains("example.invalid"));

    let limit_server = LoopbackServer::start(FixtureMode::RedirectLimit);
    let mut limit_client = logged_in_client(&limit_server, 1_024);
    let limit_error = limit_client
        .warm_up_fulltext_session()
        .expect_err("redirect hop budget should be enforced");
    assert!(limit_error.to_string().contains("exceeded 4 redirect hops"));
}

#[test]
fn loopback_transport_bounds_time_and_response_parsing() {
    let malformed_server = LoopbackServer::start(FixtureMode::MalformedQr);
    let malformed_error = malformed_server
        .transport(1_024)
        .start_qr_login()
        .expect_err("malformed QR response should fail");
    assert!(matches!(malformed_error, ZjlibCnkiError::Parse(_)));
    assert!(malformed_error.to_string().contains("non-JSON response"));

    let slow_server = LoopbackServer::start(FixtureMode::SlowQr);
    let started_at = Instant::now();
    let timeout_error = slow_server
        .transport(1_024)
        .start_qr_login()
        .expect_err("slow QR response should time out");
    assert!(matches!(timeout_error, ZjlibCnkiError::Request(_)));
    assert!(started_at.elapsed() < Duration::from_secs(3));

    let oversized_server = LoopbackServer::start(FixtureMode::OversizedPdf);
    let mut oversized_client = warmed_client(&oversized_server, 8);
    let oversized_error = oversized_client
        .download_matching_pdf(&identity(), 1)
        .expect_err("oversized PDF should fail");
    assert!(oversized_error
        .to_string()
        .contains("configured document size limit"));
}

#[test]
fn loopback_transport_rejects_missing_session_metadata_and_pdf_mismatches() {
    let missing_cookie_server = LoopbackServer::start(FixtureMode::MissingSessionCookie);
    let mut missing_cookie_client = logged_in_client(&missing_cookie_server, 1_024);
    let missing_cookie_error = missing_cookie_client
        .warm_up_fulltext_session()
        .expect_err("missing session cookie should fail");
    assert!(missing_cookie_error
        .to_string()
        .contains("did not set vpn358_sid"));

    let metadata_server = LoopbackServer::start(FixtureMode::MetadataMismatch);
    let mut metadata_client = warmed_client(&metadata_server, 1_024);
    let metadata_error = metadata_client
        .download_matching_pdf(&identity(), 1)
        .expect_err("metadata mismatch should skip download");
    assert!(metadata_error.to_string().contains("metadata mismatch"));
    assert!(!metadata_server
        .requests()
        .iter()
        .any(|request| request.target.starts_with("/proxy/kcms/download.aspx")));

    let pdf_server = LoopbackServer::start(FixtureMode::InvalidPdf);
    let mut pdf_client = warmed_client(&pdf_server, 1_024);
    let pdf_error = pdf_client
        .download_matching_pdf(&identity(), 1)
        .expect_err("non-PDF response should fail");
    assert!(pdf_error.to_string().contains("did not return PDF"));
}

#[test]
fn live_transport_errors_and_logs_omit_sensitive_material() {
    let server = LoopbackServer::start(FixtureMode::SensitivePdfFailure);
    let mut client = warmed_client(&server, 1_024);
    let expected = ZjlibCnkiArticleIdentity {
        title: TITLE_SENTINEL.to_string(),
        authors: LOCAL_PATH_SENTINEL.to_string(),
        journal_title: DOI_SENTINEL.to_string(),
    };
    let logs = CapturedLogs::default();
    let error = tracing::subscriber::with_default(logs.subscriber(), || {
        client.download_matching_pdf(&expected, 1)
    })
    .expect_err("sensitive PDF failure should fail safely");
    let error_text = error.to_string();
    let log_text = logs.text();
    let transcript = server.transcript();

    assert!(transcript.contains(COOKIE_SENTINEL));
    assert!(transcript.contains(CREDENTIAL_SENTINEL));
    assert!(transcript.contains(QUERY_SENTINEL));
    assert!(transcript.contains(TITLE_SENTINEL));
    for sentinel in [
        COOKIE_SENTINEL,
        CREDENTIAL_SENTINEL,
        QUERY_SENTINEL,
        TITLE_SENTINEL,
        DOI_SENTINEL,
        BODY_SENTINEL,
        LOCAL_PATH_SENTINEL,
    ] {
        assert!(!error_text.contains(sentinel));
        assert!(!log_text.contains(sentinel));
    }
}
