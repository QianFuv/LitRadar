// Append-only private cache and refresh observers; original implementations remain unchanged.
pub(super) fn observe_recovery(
    input: &serde_json::Value,
    config: &CfpSourceConfig,
) -> serde_json::Value {
    let original: CfpSource = serde_json::from_value(input["source"].clone()).unwrap();
    let mut cache = CaptureCache {
        documents: BTreeMap::new(),
        browser_attempts: BTreeSet::new(),
        bytes: 0,
    };
    for record in input["documents"].as_array().unwrap() {
        let url = record["url"].as_str().unwrap().to_owned();
        let document = if record["error"].as_bool().unwrap_or(false) {
            Err(CfpSourceError::Challenge)
        } else {
            Ok(CfpDocument {
                final_url: url.clone(),
                text: record["text"].as_str().unwrap().into(),
                format: "html".into(),
            })
        };
        cache.documents.insert(url.clone(), document);
        cache.browser_attempts.insert(url);
    }
    let options = CfpRefreshOptions::default();
    options.cancellation.store(
        input["cancelled"].as_bool().unwrap_or(false),
        Ordering::Release,
    );
    let duration = if input["expired"].as_bool().unwrap_or(false) {
        Duration::ZERO
    } else {
        Duration::from_secs(10)
    };
    match recover_original(
        &original,
        config,
        &mut cache,
        &options,
        Instant::now() + duration,
    ) {
        Ok(source) => json!(source),
        Err(error) => json!({"error":error.to_string()}),
    }
}
pub(super) fn observe_refresh(
    input: &serde_json::Value,
    path: &Path,
    config: &CfpSourceConfig,
    options: &CfpRefreshOptions,
    deadline: Instant,
    directory: &Path,
) -> serde_json::Value {
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
    if input.get("capture").is_some() {
        fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec(&input["capture"]).unwrap(),
        )
        .unwrap();
    }
    match refresh_full_text_source(path, config, options, deadline, Some(directory), true) {
        Ok(result) => json!(result),
        Err(error) => json!({"error":error.to_string()}),
    }
}
