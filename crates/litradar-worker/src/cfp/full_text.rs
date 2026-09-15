//! Recapture existing announcements through their original pages and matched detail links.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use litradar_domain::cfp::CfpSource;
use litradar_sources::cfp::{
    cfp_original_links, extract_cfp_full_text, is_cfp_challenge, CfpUrlRule,
};
use litradar_storage::business::cfp::publish_cfp_full_text_refresh;
use serde_json::json;

use super::*;

/// Per-journal full-text outcome, including titles that could not be verified.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfpFullTextResult {
    /// Registered acquisition identity.
    pub source_key: String,
    /// Maintained catalog identity.
    pub catalog_id: String,
    /// Success, partial, failed, no_notices, or not_attempted.
    pub status: String,
    /// Number of existing notices inspected.
    pub notices: usize,
    /// Number of verified complete announcements found.
    pub recovered: usize,
    /// Number of complete announcements actually written to storage.
    pub updated: usize,
    /// Existing titles for which no complete original could be verified.
    pub unresolved: Vec<String>,
    /// Sanitized operational failure, if any.
    pub error: Option<String>,
}

struct CaptureCache {
    documents: BTreeMap<String, Result<CfpDocument, CfpSourceError>>,
    browser_attempts: BTreeSet<String>,
    bytes: usize,
}

impl CaptureCache {
    fn capture(
        &mut self,
        config: &CfpSourceConfig,
        url: &str,
        options: &CfpRefreshOptions,
        deadline: Instant,
        should_use_browser: bool,
    ) -> Result<CfpDocument, CfpSourceError> {
        if !should_use_browser || self.browser_attempts.contains(url) {
            if let Some(document) = self.documents.get(url) {
                return document.clone();
            }
        }
        if Instant::now() >= deadline || options.cancellation.load(Ordering::Acquire) {
            return Err(CfpSourceError::Deadline);
        }
        let url = reqwest::Url::parse(url).map_err(|_| CfpSourceError::DisallowedUrl)?;
        let host = url.host_str().ok_or(CfpSourceError::DisallowedUrl)?;
        let mut config = config.clone();
        config.allowed_urls.push(CfpUrlRule {
            host: host.to_owned(),
            path_prefix: url.path().to_owned(),
        });
        let should_use_browser = should_use_browser || host == "link.springer.com";
        let transport = CfpLiveTransport::new(options.clone())?;
        let mut result = if should_use_browser {
            self.browser_attempts.insert(url.to_string());
            transport.obscura_document(&config, url.as_str(), deadline)
        } else {
            transport.http_document(&config, url.as_str(), deadline)
        };
        if result.as_ref().is_ok_and(is_cfp_challenge) {
            result = Err(CfpSourceError::Challenge);
        }
        if !should_use_browser
            && matches!(
                result,
                Err(CfpSourceError::Request
                    | CfpSourceError::Challenge
                    | CfpSourceError::HttpStatus(403 | 429 | 503))
            )
        {
            self.browser_attempts.insert(url.to_string());
            result = transport.obscura_document(&config, url.as_str(), deadline);
        }
        if let Ok(document) = &result {
            if is_cfp_challenge(document) {
                result = Err(CfpSourceError::Challenge);
            } else if self.bytes + document.text.len() > CFP_MAX_CAPTURE_BYTES {
                result = Err(CfpSourceError::TooLarge);
            } else {
                self.bytes += document.text.len();
            }
        }
        self.documents.insert(url.to_string(), result.clone());
        result
    }
}

fn recover_original(
    original: &CfpSource,
    config: &CfpSourceConfig,
    cache: &mut CaptureCache,
    options: &CfpRefreshOptions,
    deadline: Instant,
) -> Result<CfpSource, CfpSourceError> {
    let mut pending = VecDeque::from([(original.source_url.clone(), 0)]);
    let mut visited = BTreeSet::new();
    let mut last_error = CfpSourceError::Unrecognized;
    while let Some((url, depth)) = pending.pop_front() {
        if Instant::now() >= deadline {
            return Err(CfpSourceError::Deadline);
        }
        if visited.len() >= 8 {
            break;
        }
        if !visited.insert(url.clone()) {
            continue;
        }
        let document = match cache.capture(config, &url, options, deadline, false) {
            Ok(document) => document,
            Err(error) => {
                last_error = error;
                continue;
            }
        };
        if let Ok(source) = extract_cfp_full_text(original, &document) {
            return Ok(source);
        }
        let mut links = cfp_original_links(original, &document);
        if links.is_empty() && document.format == "html" && !cache.browser_attempts.contains(&url) {
            if let Ok(rendered) = cache.capture(config, &url, options, deadline, true) {
                if let Ok(source) = extract_cfp_full_text(original, &rendered) {
                    return Ok(source);
                }
                links = cfp_original_links(original, &rendered);
            }
        }
        if depth < 2 {
            for link in links.into_iter().take(4) {
                pending.push_back((link, depth + 1));
            }
        }
    }
    Err(last_error)
}

fn refresh_full_text_source(
    path: &Path,
    config: &CfpSourceConfig,
    options: &CfpRefreshOptions,
    deadline: Instant,
    capture_directory: Option<&Path>,
    should_resume_captures: bool,
) -> Result<CfpFullTextResult, CfpRepositoryError> {
    let originals = load_cfp_source_originals(path, &config.source_key)?;
    let mut result = CfpFullTextResult {
        source_key: config.source_key.clone(),
        catalog_id: config.catalog_ids[0].clone(),
        status: "no_notices".into(),
        notices: originals.len(),
        recovered: 0,
        updated: 0,
        unresolved: Vec::new(),
        error: None,
    };
    if originals.is_empty() {
        return Ok(result);
    }
    if Instant::now() >= deadline || options.cancellation.load(Ordering::Acquire) {
        result.status = "not_attempted".into();
        return Ok(result);
    }
    let now = utc_now();
    let seconds = deadline
        .saturating_duration_since(Instant::now())
        .as_secs()
        .saturating_add(1)
        .clamp(1, 600) as i64;
    let lease = begin_cfp_refresh(path, &config.source_key, now.timestamp(), seconds)?;
    let acquisition_deadline = deadline
        .checked_sub(Duration::from_secs(5))
        .unwrap_or(deadline);
    let originals = load_cfp_source_originals(path, &config.source_key)?;
    result.notices = originals.len();
    let mut cache = CaptureCache {
        documents: BTreeMap::new(),
        browser_attempts: BTreeSet::new(),
        bytes: 0,
    };
    let capture_path = capture_directory.map(|directory| {
        let name: String = config
            .source_key
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        directory.join(format!("{name}.json"))
    });
    if should_resume_captures {
        if let Some(capture_path) = capture_path.as_ref().filter(|path| path.is_file()) {
            let bytes = fs::read(capture_path).map_err(|error| {
                CfpRepositoryError::Invalid(format!("Could not read the saved capture: {error}"))
            })?;
            let saved: serde_json::Value = serde_json::from_slice(&bytes)?;
            if saved["result"]["sourceKey"].as_str() != Some(config.source_key.as_str()) {
                return Err(CfpRepositoryError::Invalid(
                    "Saved capture journal identity does not match".into(),
                ));
            }
            for document in saved["documents"].as_array().into_iter().flatten() {
                if let (Some(requested), Some(url), Some(text), Some(format)) = (
                    document["requestedUrl"].as_str(),
                    document["url"].as_str(),
                    document["text"].as_str(),
                    document["format"].as_str(),
                ) {
                    cache.bytes += text.len();
                    if cache.bytes > CFP_MAX_CAPTURE_BYTES {
                        return Err(CfpRepositoryError::Invalid(
                            "Saved capture exceeds the source budget".into(),
                        ));
                    }
                    let captured = CfpDocument {
                        final_url: url.into(),
                        text: text.into(),
                        format: format.into(),
                    };
                    cache
                        .documents
                        .insert(requested.into(), Ok(captured.clone()));
                    cache.documents.insert(url.into(), Ok(captured));
                    if url.starts_with("https://link.springer.com/") {
                        cache.browser_attempts.insert(requested.into());
                    }
                }
            }
            for url in saved["browserAttempts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
            {
                cache.browser_attempts.insert(url.into());
            }
        }
    }
    let mut sources = Vec::new();
    for original in originals {
        match recover_original(&original, config, &mut cache, options, acquisition_deadline) {
            Ok(source) => {
                result.recovered += 1;
                sources.push(source);
            }
            Err(error) => {
                result.unresolved.push(original.title.clone());
                result.error = Some(error.to_string());
                sources.push(original);
            }
        }
    }
    let documents: Vec<_> = cache.documents.iter().map(|(url, document)| match document {
        Ok(document) => json!({"requestedUrl":url,"url":document.final_url,"format":document.format,"text":document.text}),
        Err(error) => json!({"requestedUrl":url,"error":error.to_string()}),
    }).collect();
    let capture = serde_json::to_string(&documents)?;
    if result.recovered > 0
        && capture.len() <= CFP_MAX_CAPTURE_BYTES
        && !options.cancellation.load(Ordering::Acquire)
    {
        let publication = CfpPublication {
            capture,
            capture_format: "original_documents_json".into(),
            source_url: config.discovery_url.clone(),
            config_version: config.config_version,
            sources: sources.clone(),
            empty_journals: Vec::new(),
        };
        match publish_cfp_full_text_refresh(
            path,
            &lease,
            &publication,
            utc_now().timestamp(),
            result.unresolved.len(),
        ) {
            Ok(()) => {
                result.updated = result.recovered;
                result.status = if result.unresolved.is_empty() {
                    "success"
                } else {
                    "partial"
                }
                .into()
            }
            Err(error) => {
                result.status = "failed".into();
                result.error = Some(error.to_string());
                fail_cfp_refresh(path, &lease, "Full-text publication failed", false)?;
            }
        }
    } else {
        result.status = "failed".into();
        fail_cfp_refresh(
            path,
            &lease,
            "Complete original text could not be verified for every notice",
            false,
        )?;
    }
    if let Some(capture_path) = capture_path {
        let evidence = json!({"result":result,"sources":sources,"documents":documents,"browserAttempts":cache.browser_attempts});
        fs::write(capture_path, serde_json::to_vec(&evidence)?).map_err(|error| {
            CfpRepositoryError::Invalid(format!(
                "Could not save full-text capture evidence: {error}"
            ))
        })?;
    }
    tracing::info!(
        event = "cfp.full_text.completed",
        catalog_id = %result.catalog_id,
        status = %result.status,
        recovered = result.recovered,
        notices = result.notices,
        "CFP full-text refresh completed"
    );
    Ok(result)
}

/// Recapture every stored original, including snapshot-only sources, with finite per-source budgets.
pub fn refresh_cfp_full_texts(
    path: &Path,
    configs: &[CfpSourceConfig],
    options: &CfpRefreshOptions,
    capture_directory: Option<&Path>,
    should_resume_captures: bool,
) -> Result<Vec<CfpFullTextResult>, CfpRepositoryError> {
    if options.source_timeout.is_zero()
        || options.source_timeout > Duration::from_secs(600)
        || options.overall_timeout.is_zero()
        || options.overall_timeout > Duration::from_secs(3600)
    {
        return Err(CfpRepositoryError::Invalid(
            "Invalid CFP full-text acquisition time budget".into(),
        ));
    }
    if let Some(directory) = capture_directory {
        fs::create_dir_all(directory).map_err(|error| {
            CfpRepositoryError::Invalid(format!(
                "Could not create full-text capture directory: {error}"
            ))
        })?;
    }
    ensure_cfp_seed(path)?;
    let deadline = Instant::now() + options.overall_timeout;
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(config) = configs.get(index) else {
                    break;
                };
                let result = refresh_full_text_source(
                    path,
                    config,
                    options,
                    deadline.min(Instant::now() + options.source_timeout),
                    capture_directory,
                    should_resume_captures,
                );
                results
                    .lock()
                    .expect("full-text results lock")
                    .push((index, result));
            });
        }
    });
    let mut results = results.into_inner().expect("full-text results lock");
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use litradar_sources::cfp::CfpAdapter;

    #[test]
    fn full_text_recovery_follows_the_existing_title_and_preserves_timeline_semantics() {
        let original: CfpSource = serde_json::from_value(json!({"catalogIds":["journal-a"],"journalTitle":"Example Journal","title":"Original topic","typeText":"Special Issue","dateText":"Submission deadline: 31 December 2026","sourceUrl":"https://example.org/calls","scope":"Original preview...","checkedOn":"2026-09-15"})).unwrap();
        let config = CfpSourceConfig {
            source_key: "journal:journal-a".into(),
            catalog_ids: original.catalog_ids.clone(),
            journal_title: original.journal_title.clone(),
            discovery_url: original.source_url.clone(),
            adapter: CfpAdapter::SnapshotOnly,
            config_version: 1,
            allowed_urls: vec![],
            identity_texts: vec![],
            empty_statements: vec![],
            capability_note: None,
            retains_previous_notices: false,
        };
        let mut cache = CaptureCache { documents: BTreeMap::from([
            (original.source_url.clone(), Ok(CfpDocument { final_url: original.source_url.clone(), format:"html".into(), text:"<h1>Example Journal</h1><h2><a href='/collections/original'>Original topic</a></h2><p>Original preview...</p>".into() })),
            ("https://example.org/collections/original".into(), Ok(CfpDocument { final_url:"https://example.org/collections/original".into(), format:"html".into(), text:"<h1 data-test='collection-title'>Original topic</h1><div data-test='collection-description'><p>Every original research topic.</p><p>Authors should prepare the complete experimental protocol.</p></div>".into() })),
        ]), browser_attempts:BTreeSet::new(), bytes:0 };
        let recovered = recover_original(
            &original,
            &config,
            &mut cache,
            &CfpRefreshOptions::default(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(recovered.scope, "Every original research topic.");
        assert_eq!(
            recovered.requirements,
            "Authors should prepare the complete experimental protocol."
        );
        assert_eq!(recovered.date_text, original.date_text);
        assert_eq!(
            recovered.source_url,
            "https://example.org/collections/original"
        );
        cache
            .documents
            .insert(recovered.source_url, Err(CfpSourceError::Challenge));
        assert!(recover_original(
            &original,
            &config,
            &mut cache,
            &CfpRefreshOptions::default(),
            Instant::now() + Duration::from_secs(1)
        )
        .is_err());
    }

    #[test]
    fn full_text_rejects_unbounded_options_before_any_database_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.sqlite");
        let options = CfpRefreshOptions {
            source_timeout: Duration::ZERO,
            ..Default::default()
        };
        assert!(refresh_cfp_full_texts(&path, &[], &options, None, false).is_err());
        assert!(!path.exists());
    }
}
