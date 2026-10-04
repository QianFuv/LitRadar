// Append-only observation entry point for the unchanged original module.
pub fn observe(input: &serde_json::Value) -> serde_json::Value {
    use litradar_storage::business::cfp::{import_cfp_seed, load_cfp_journals};
    use serde_json::json;
    let config: CfpSourceConfig = serde_json::from_value(input["config"].clone()).unwrap();
    if input["op"] == "envelope" {
        return match decode_obscura_document(&config, input["payload"].as_str().unwrap().as_bytes())
        {
            Ok(document) => {
                json!({"finalUrl":document.final_url,"text":document.text,"format":document.format})
            }
            Err(error) => json!({"error":error.to_string()}),
        };
    }
    if input["op"] == "recover" {
        return full_text::observe_recovery(input, &config);
    }
    let directory = tempfile::tempdir().unwrap();
    let root = input["directory"]
        .as_str()
        .map(Path::new)
        .unwrap_or(directory.path());
    let path = root.join("state.sqlite");
    if !input["resume"].as_bool().unwrap_or(false) {
        litradar_storage::migrate_auth_database(&path).unwrap();
        import_cfp_seed(&path, "fixture", input["seed"].as_str().unwrap().as_bytes()).unwrap();
    }
    let options = CfpRefreshOptions::default();
    options.cancellation.store(
        input["cancelled"].as_bool().unwrap_or(false),
        Ordering::Release,
    );
    let duration = if input["expired"].as_bool().unwrap_or(false) {
        Duration::ZERO
    } else {
        Duration::from_secs(20)
    };
    let outcome = if input["op"] == "full" {
        full_text::observe_refresh(
            input,
            &path,
            &config,
            &options,
            Instant::now() + duration,
            root,
        )
    } else {
        struct Fixture(String);
        impl CfpTransport for Fixture {
            fn fetch(
                &self,
                config: &CfpSourceConfig,
                _url: &str,
                _deadline: Instant,
            ) -> Result<CfpDocument, CfpSourceError> {
                Ok(CfpDocument {
                    final_url: config.discovery_url.clone(),
                    text: self.0.clone(),
                    format: "html".into(),
                })
            }
        }
        match refresh_cfp_source(
            &path,
            &config,
            &Fixture(input["html"].as_str().unwrap().into()),
            Instant::now() + duration,
        ) {
            Ok(result) => json!(result),
            Err(error) => json!({"error":error.to_string()}),
        }
    };
    let originals = load_cfp_source_originals(&path, &config.source_key).unwrap();
    let state:Vec<_>=load_cfp_journals(&path).unwrap().iter().flat_map(|journal|journal.sources.iter().map(|source|json!({"sourceKey":source.source_key,"status":source.status,"revision":source.revision,"hasAttempt":source.last_attempt.is_some(),"hasSuccess":source.last_success.is_some(),"hasLease":source.lease_expires_at.is_some(),"error":source.last_error}))).collect();
    json!({"outcome":outcome,"originals":originals,"state":state})
}
