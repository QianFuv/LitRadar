//! Backend CFP refresh orchestration and supervised optional source helpers.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_sources::cfp::{
    acquire_cfp_source, decode_cfp_body, parse_cfp_page, CfpDocument, CfpHttpTransport,
    CfpSourceConfig, CfpSourceError, CfpTransport, CFP_MAX_CAPTURE_BYTES, CFP_MAX_PAGE_BYTES,
    CFP_SEED, CFP_SEED_ID,
};
use litradar_storage::business::cfp::{
    begin_cfp_refresh, fail_cfp_refresh, import_prepared_cfp_seed, load_cfp_source_originals,
    prepare_cfp_seed, publish_cfp_refresh, CfpImportResult, CfpPublication, CfpRepositoryError,
    PreparedCfpSeed,
};
use serde::{Deserialize, Serialize};

use crate::process_supervisor::SupervisedChild;

const OBSCURA_PROTOCOL: &str = "litradar.cfp.page.v1";
const OBSCURA_EVAL:&str="(() => { const html = document.documentElement ? document.documentElement.outerHTML : ''; if (html.length > 2097152) throw new Error('CFP capture too large'); return JSON.stringify({protocol:'litradar.cfp.page.v1',finalUrl:location.href,html}); })()";

mod full_text;
pub use full_text::{refresh_cfp_full_texts, CfpFullTextResult};

/// Normalize the packaged seed once per process and import it once per business DB.
pub fn ensure_cfp_seed(path: &Path) -> Result<CfpImportResult, CfpRepositoryError> {
    static PREPARED: LazyLock<Result<PreparedCfpSeed, String>> =
        LazyLock::new(|| prepare_cfp_seed(CFP_SEED).map_err(|error| error.to_string()));
    let prepared = PREPARED
        .as_ref()
        .map_err(|error| CfpRepositoryError::Invalid(error.clone()))?;
    import_prepared_cfp_seed(path, CFP_SEED_ID, prepared)
}

/// Finite acquisition budgets and optional backend executable configuration.
#[derive(Clone)]
pub struct CfpRefreshOptions {
    /// Total time for one source including HTTP, helper and parsing work.
    pub source_timeout: Duration,
    /// Total time for the selected batch; unstarted sources are reported explicitly.
    pub overall_timeout: Duration,
    /// Configured Obscura path; absent uses the inherited PATH.
    pub obscura_path: Option<PathBuf>,
    /// Configured pdftotext path; absent uses the inherited PATH.
    pub pdftotext_path: Option<PathBuf>,
    /// Cooperative cancellation shared by all source work and helper polling.
    pub cancellation: Arc<AtomicBool>,
}

impl Default for CfpRefreshOptions {
    fn default() -> Self {
        Self {
            source_timeout: Duration::from_secs(90),
            overall_timeout: Duration::from_secs(600),
            obscura_path: std::env::var_os("LITRADAR_OBSCURA_PATH").map(PathBuf::from),
            pdftotext_path: std::env::var_os("LITRADAR_PDFTOTEXT_PATH").map(PathBuf::from),
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Live per-source transport, allowing at most one Obscura fallback for the whole source.
pub struct CfpLiveTransport {
    http: CfpHttpTransport,
    options: CfpRefreshOptions,
    has_used_obscura: AtomicBool,
}

impl CfpLiveTransport {
    /// Create a fresh source-scoped HTTP/helper budget.
    pub fn new(options: CfpRefreshOptions) -> Result<Self, CfpSourceError> {
        Ok(Self {
            http: CfpHttpTransport::new()?,
            options,
            has_used_obscura: AtomicBool::new(false),
        })
    }

    fn check(&self, deadline: Instant) -> Result<(), CfpSourceError> {
        if self.options.cancellation.load(Ordering::Acquire) {
            Err(CfpSourceError::Cancelled)
        } else if Instant::now() >= deadline {
            Err(CfpSourceError::Deadline)
        } else {
            Ok(())
        }
    }

    fn http_document(
        &self,
        config: &CfpSourceConfig,
        url: &str,
        deadline: Instant,
    ) -> Result<CfpDocument, CfpSourceError> {
        let response = self.http.fetch_bytes(config, url, deadline)?;
        self.check(deadline)?;
        if response.bytes.starts_with(b"%PDF-") {
            let directory = tempfile::tempdir().map_err(|_| CfpSourceError::Helper)?;
            let input = directory.path().join("source.pdf");
            let output_file = directory.path().join("source.txt");
            fs::write(&input, &response.bytes).map_err(|_| CfpSourceError::Helper)?;
            let mut command = Command::new(
                self.options
                    .pdftotext_path
                    .as_deref()
                    .unwrap_or_else(|| Path::new("pdftotext")),
            );
            command
                .args(["-enc", "UTF-8", "-eol", "unix", "-nopgbrk"])
                .arg(&input)
                .arg(&output_file);
            let output = run_helper(
                &mut command,
                deadline,
                &self.options.cancellation,
                &output_file,
            )?;
            let text = String::from_utf8(output).map_err(|_| CfpSourceError::Encoding)?;
            if text.trim().is_empty() {
                return Err(CfpSourceError::Unrecognized);
            }
            Ok(CfpDocument {
                final_url: response.final_url,
                text,
                format: "pdf_text".into(),
            })
        } else {
            let text = decode_cfp_body(&response.bytes, &response.content_type)?;
            Ok(CfpDocument {
                final_url: response.final_url,
                text,
                format: "html".into(),
            })
        }
    }

    fn obscura_document(
        &self,
        config: &CfpSourceConfig,
        url: &str,
        deadline: Instant,
    ) -> Result<CfpDocument, CfpSourceError> {
        self.check(deadline)?;
        if self.has_used_obscura.swap(true, Ordering::AcqRel) {
            return Err(CfpSourceError::Helper);
        }
        let directory = tempfile::tempdir().map_err(|_| CfpSourceError::Helper)?;
        let output = directory.path().join("page.json");
        let seconds = deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
            .clamp(1, 40)
            .to_string();
        let mut command = Command::new(
            self.options
                .obscura_path
                .as_deref()
                .unwrap_or_else(|| Path::new("obscura")),
        );
        command
            .args([
                "fetch",
                url,
                "--stealth",
                "--timeout",
                &seconds,
                "--eval",
                OBSCURA_EVAL,
                "--quiet",
                "--output",
            ])
            .arg(&output)
            .env_remove("OBSCURA_ALLOW_PRIVATE_NETWORK");
        if reqwest::Url::parse(url).is_ok_and(|url| url.host_str() == Some("link.springer.com")) {
            command.args(["--wait-until", "domcontentloaded", "--wait", "0"]);
        }
        let bytes = run_helper(&mut command, deadline, &self.options.cancellation, &output)?;
        decode_obscura_document(config, &bytes)
    }
}

impl CfpTransport for CfpLiveTransport {
    fn fetch(
        &self,
        config: &CfpSourceConfig,
        url: &str,
        deadline: Instant,
    ) -> Result<CfpDocument, CfpSourceError> {
        self.check(deadline)?;
        let attempted = self.http_document(config, url, deadline);
        let attempted = attempted.and_then(|document| {
            if document.format == "pdf_text" && url != config.discovery_url {
                return Ok(document);
            }
            let checked_on = utc_now().date_naive().to_string();
            let is_discovery = url == config.discovery_url;
            match parse_cfp_page(config, &document, &checked_on, is_discovery) {
                Ok(_) => Ok(document),
                Err(error) => Err(error),
            }
        });
        match attempted {
            Ok(document) => Ok(document),
            Err(
                error @ (CfpSourceError::Request
                | CfpSourceError::Deadline
                | CfpSourceError::HttpStatus(403 | 429 | 503)
                | CfpSourceError::Challenge
                | CfpSourceError::Unrecognized),
            ) if !self.has_used_obscura.load(Ordering::Acquire) => {
                let document = self.obscura_document(config, url, deadline)?;
                let checked_on = utc_now().date_naive().to_string();
                parse_cfp_page(config, &document, &checked_on, url == config.discovery_url)
                    .map_err(|found| {
                        if found == CfpSourceError::Unsupported {
                            error
                        } else {
                            found
                        }
                    })?;
                Ok(document)
            }
            Err(error) => Err(error),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObscuraPage {
    protocol: String,
    final_url: String,
    html: String,
}

fn decode_obscura_document(
    config: &CfpSourceConfig,
    bytes: &[u8],
) -> Result<CfpDocument, CfpSourceError> {
    if bytes.len() > CFP_MAX_PAGE_BYTES {
        return Err(CfpSourceError::TooLarge);
    }
    let page: ObscuraPage = serde_json::from_slice(bytes).map_err(|_| CfpSourceError::Helper)?;
    if page.protocol != OBSCURA_PROTOCOL || page.html.trim().is_empty() {
        return Err(CfpSourceError::Unrecognized);
    }
    let url = reqwest::Url::parse(&page.final_url).map_err(|_| CfpSourceError::DisallowedUrl)?;
    if !config.permits_url(&url) {
        return Err(CfpSourceError::DisallowedUrl);
    }
    Ok(CfpDocument {
        final_url: page.final_url,
        text: page.html,
        format: "html".into(),
    })
}

fn run_helper(
    command: &mut Command,
    deadline: Instant,
    cancellation: &AtomicBool,
    output_file: &Path,
) -> Result<Vec<u8>, CfpSourceError> {
    if cancellation.load(Ordering::Acquire) {
        return Err(CfpSourceError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(CfpSourceError::Deadline);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn_hidden(command).map_err(|_| CfpSourceError::Helper)?;
    let status = loop {
        let error = if cancellation.load(Ordering::Acquire) {
            Some(CfpSourceError::Cancelled)
        } else if Instant::now() >= deadline {
            Some(CfpSourceError::Deadline)
        } else if fs::metadata(output_file)
            .is_ok_and(|metadata| metadata.len() > CFP_MAX_PAGE_BYTES as u64)
        {
            Some(CfpSourceError::TooLarge)
        } else {
            None
        };
        if let Some(error) = error {
            child
                .force_kill_remaining_tree()
                .map_err(|_| CfpSourceError::Helper)?;
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                child
                    .force_kill_remaining_tree()
                    .map_err(|_| CfpSourceError::Helper)?;
                return Err(CfpSourceError::Helper);
            }
        }
    };
    child
        .force_kill_remaining_tree()
        .map_err(|_| CfpSourceError::Helper)?;
    if !status.success() {
        return Err(CfpSourceError::Helper);
    }
    let file = fs::File::open(output_file).map_err(|_| CfpSourceError::Helper)?;
    let mut bytes = Vec::new();
    file.take(CFP_MAX_PAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CfpSourceError::Helper)?;
    if bytes.len() > CFP_MAX_PAGE_BYTES {
        return Err(CfpSourceError::TooLarge);
    }
    if Instant::now() >= deadline {
        return Err(CfpSourceError::Deadline);
    }
    Ok(bytes)
}

fn utc_now() -> chrono::DateTime<chrono::Utc> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    chrono::DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos())
        .expect("current UTC timestamp")
}

/// Outcome of a source attempt, suitable for structured CLI reporting.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfpRefreshResult {
    /// Acquisition unit in the backend registry.
    pub source_key: String,
    /// Canonical journal identity.
    pub catalog_id: String,
    /// Success, failed, unsupported or not_attempted.
    pub status: String,
    /// Published source count, including retained records for filtered lists.
    pub notices: usize,
    /// Stable sanitized reason when this attempt did not publish.
    pub error: Option<String>,
}

/// Refresh and atomically publish one registered source using a supplied transport.
pub fn refresh_cfp_source(
    path: &Path,
    config: &CfpSourceConfig,
    transport: &impl CfpTransport,
    deadline: Instant,
) -> Result<CfpRefreshResult, CfpRepositoryError> {
    let now = utc_now();
    let seconds = deadline
        .saturating_duration_since(Instant::now())
        .as_secs()
        .saturating_add(1)
        .clamp(1, 600) as i64;
    let lease = begin_cfp_refresh(path, &config.source_key, now.timestamp(), seconds)?;
    let acquired = acquire_cfp_source(transport, config, &now.date_naive().to_string(), deadline);
    let acquired = acquired.and_then(|value| {
        if Instant::now() >= deadline {
            Err(CfpSourceError::Deadline)
        } else {
            Ok(value)
        }
    });
    match acquired {
        Ok(mut acquired) => {
            if config.retains_previous_notices {
                let originals = load_cfp_source_originals(path, &config.source_key)?;
                for original in originals {
                    if !acquired
                        .sources
                        .iter()
                        .any(|source| source.title == original.title)
                    {
                        acquired.sources.push(original);
                    }
                }
                if !acquired.sources.is_empty() {
                    acquired.empty_journals.clear();
                }
            }
            let notices = acquired.sources.len();
            let capture=serde_json::to_string(&acquired.documents.iter().map(|document|serde_json::json!({"url":document.final_url,"format":document.format,"text":document.text})).collect::<Vec<_>>())?;
            if capture.len() > CFP_MAX_CAPTURE_BYTES {
                fail_cfp_refresh(path, &lease, &CfpSourceError::TooLarge.to_string(), false)?;
                return Ok(failed_result(config, CfpSourceError::TooLarge));
            }
            let publication = CfpPublication {
                capture,
                capture_format: "original_documents_json".into(),
                source_url: config.discovery_url.clone(),
                config_version: config.config_version,
                sources: acquired.sources,
                empty_journals: acquired.empty_journals,
            };
            if let Err(error) =
                publish_cfp_refresh(path, &lease, &publication, utc_now().timestamp())
            {
                let _ = fail_cfp_refresh(
                    path,
                    &lease,
                    "Source publication failed or was superseded",
                    false,
                );
                return Err(error);
            }
            Ok(CfpRefreshResult {
                source_key: config.source_key.clone(),
                catalog_id: config.catalog_ids[0].clone(),
                status: "success".into(),
                notices,
                error: None,
            })
        }
        Err(error) => {
            fail_cfp_refresh(
                path,
                &lease,
                &error.to_string(),
                error == CfpSourceError::Unsupported,
            )?;
            Ok(failed_result(config, error))
        }
    }
}

fn failed_result(config: &CfpSourceConfig, error: CfpSourceError) -> CfpRefreshResult {
    CfpRefreshResult {
        source_key: config.source_key.clone(),
        catalog_id: config.catalog_ids[0].clone(),
        status: if error == CfpSourceError::Unsupported {
            "unsupported"
        } else {
            "failed"
        }
        .into(),
        notices: 0,
        error: Some(error.to_string()),
    }
}

/// Refresh selected backend registrations with two concurrent sources and a finite batch deadline.
pub fn refresh_cfp_sources(
    path: &Path,
    configs: &[CfpSourceConfig],
    options: &CfpRefreshOptions,
) -> Result<Vec<CfpRefreshResult>, CfpRepositoryError> {
    if options.source_timeout.is_zero()
        || options.source_timeout > Duration::from_secs(600)
        || options.overall_timeout.is_zero()
        || options.overall_timeout > Duration::from_secs(3600)
    {
        return Err(CfpRepositoryError::Invalid(
            "Invalid CFP acquisition time budget".into(),
        ));
    }
    ensure_cfp_seed(path)?;
    let deadline = Instant::now() + options.overall_timeout;
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(config) = configs.get(index) else {
                    break;
                };
                let result = if Instant::now() >= deadline
                    || options.cancellation.load(Ordering::Acquire)
                {
                    Ok(CfpRefreshResult {
                        source_key: config.source_key.clone(),
                        catalog_id: config.catalog_ids[0].clone(),
                        status: "not_attempted".into(),
                        notices: 0,
                        error: Some("Batch deadline or cancellation prevented this attempt".into()),
                    })
                } else {
                    match CfpLiveTransport::new(options.clone()) {
                        Ok(transport) => refresh_cfp_source(
                            path,
                            config,
                            &transport,
                            deadline.min(Instant::now() + options.source_timeout),
                        ),
                        Err(error) => Ok(failed_result(config, error)),
                    }
                };
                results
                    .lock()
                    .expect("CFP result lock")
                    .push((index, result));
            });
        }
    });
    let mut results = results.into_inner().expect("CFP result lock");
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use litradar_domain::cfp::CfpSeed;
    use litradar_sources::cfp::{CfpAdapter, CfpUrlRule};
    use litradar_storage::business::cfp::{import_cfp_seed, load_cfp_journals};
    use litradar_storage::migrate_auth_database;

    fn config() -> CfpSourceConfig {
        CfpSourceConfig {
            source_key: "journal:fixture".into(),
            catalog_ids: vec!["fixture".into()],
            journal_title: "Example Journal".into(),
            discovery_url: "https://example.org/calls".into(),
            adapter: CfpAdapter::ElsevierCalls,
            config_version: 1,
            allowed_urls: vec![CfpUrlRule {
                host: "example.org".into(),
                path_prefix: "/calls".into(),
            }],
            identity_texts: vec!["Example Journal".into()],
            empty_statements: Vec::new(),
            capability_note: None,
            retains_previous_notices: false,
        }
    }

    struct FixtureTransport {
        html: Mutex<String>,
    }
    impl CfpTransport for FixtureTransport {
        fn fetch(
            &self,
            config: &CfpSourceConfig,
            _url: &str,
            _deadline: Instant,
        ) -> Result<CfpDocument, CfpSourceError> {
            Ok(CfpDocument {
                final_url: config.discovery_url.clone(),
                text: self.html.lock().unwrap().clone(),
                format: "html".into(),
            })
        }
    }

    #[test]
    fn cfp_refresh_persists_discovered_originals_across_reopen_and_retains_them_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.sqlite");
        migrate_auth_database(&path).unwrap();
        let source=serde_json::from_value(serde_json::json!({"catalogIds":["fixture"],"journalTitle":"Example Journal","title":"Old reviewed call","typeText":"Special Issue","dateText":"Submission deadline: 1 March 2026","sourceUrl":"https://example.org/calls","checkedOn":"2026-09-15"})).unwrap();
        let seed = CfpSeed {
            format_version: 1,
            sources: vec![source],
            empty_journals: Vec::new(),
            expected_journals: None,
            expected_notices: None,
        };
        import_cfp_seed(&path, "fixture", &serde_json::to_vec(&seed).unwrap()).unwrap();
        let mut config = config();
        config.retains_previous_notices = true;
        let transport=FixtureTransport {html:Mutex::new("<h1>Example Journal</h1><h2>Call for papers</h2><h3>New original announcement</h3><p>New research scope.</p><p>Submission deadline: 1 December 2027</p>".into())};
        let result = refresh_cfp_source(
            &path,
            &config,
            &transport,
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(result.status, "success");
        assert_eq!(result.notices, 2);
        let after_restart = load_cfp_journals(&path).unwrap();
        assert_eq!(
            after_restart[0].notices[0].title,
            "New original announcement"
        );
        assert_eq!(after_restart[0].notices[1].title, "Old reviewed call");
        assert_eq!(after_restart[0].sources[0].revision, 2);
        *transport.html.lock().unwrap() = "<title>Just a moment...</title>".into();
        let result = refresh_cfp_source(
            &path,
            &config,
            &transport,
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(result.status, "failed");
        let retained = load_cfp_journals(&path).unwrap();
        assert_eq!(retained[0].notices, after_restart[0].notices);
        assert_eq!(
            retained[0].sources[0].last_success,
            after_restart[0].sources[0].last_success
        );
        assert_eq!(retained[0].sources[0].revision, 2);
    }

    #[test]
    fn cfp_obscura_protocol_rejects_extra_output_foreign_urls_and_exit_zero_challenges() {
        let config = config();
        for bytes in [b"log\n{}".as_slice(),b"{}\n{}",br#"{"protocol":"litradar.cfp.page.v1","finalUrl":"https://evil.example/calls","html":"original"}"#] {assert!(decode_obscura_document(&config,bytes).is_err());}
        let bytes=serde_json::to_vec(&serde_json::json!({"protocol":OBSCURA_PROTOCOL,"finalUrl":config.discovery_url,"html":"<title>Just a moment...</title>"})).unwrap();
        let document = decode_obscura_document(&config, &bytes).unwrap();
        assert!(matches!(
            parse_cfp_page(&config, &document, "2026-09-15", true),
            Err(CfpSourceError::Challenge)
        ));
    }

    fn output_command(is_slow: bool, output_file: &Path) -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                if is_slow {
                    "Start-Sleep -Seconds 30"
                } else {
                    "[System.IO.File]::WriteAllText($env:LITRADAR_CFP_TEST_OUTPUT, ('x' * 4194305))"
                },
            ]);
            command.env("LITRADAR_CFP_TEST_OUTPUT", output_file);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                if is_slow {
                    "sleep 30"
                } else {
                    "head -c 4194305 /dev/zero > \"$LITRADAR_CFP_TEST_OUTPUT\""
                },
            ]);
            command.env("LITRADAR_CFP_TEST_OUTPUT", output_file);
            command
        }
    }

    #[test]
    fn cfp_helpers_terminate_on_timeout_cancellation_and_output_overflow() {
        let directory = tempfile::tempdir().unwrap();
        let output_file = directory.path().join("helper-output");
        let cancellation = AtomicBool::new(false);
        let started = Instant::now();
        assert!(matches!(
            run_helper(
                &mut output_command(true, &output_file),
                started + Duration::from_millis(250),
                &cancellation,
                &output_file
            ),
            Err(CfpSourceError::Deadline)
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(matches!(
            run_helper(
                &mut output_command(false, &output_file),
                Instant::now() + Duration::from_secs(15),
                &cancellation,
                &output_file
            ),
            Err(CfpSourceError::TooLarge)
        ));
        cancellation.store(true, Ordering::Release);
        assert!(matches!(
            run_helper(
                &mut output_command(true, &output_file),
                Instant::now() + Duration::from_secs(2),
                &cancellation,
                &output_file
            ),
            Err(CfpSourceError::Cancelled)
        ));
    }
}
