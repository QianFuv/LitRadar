// Capture original public source calls while retaining every original assertion.
use litradar_domain::cfp::CfpSource;
use litradar_sources::cfp::{CfpAcquisition, CfpParsedPage};
use serde_json::{Value, json};
fn document_json(document: &CfpDocument) -> Value {
    json!({"finalUrl":document.final_url,"text":document.text,"format":document.format})
}
fn error_json(error: &CfpSourceError) -> Value {
    json!({"error":error.to_string()})
}
fn emit(input: Value, expected: Value) {
    println!(
        "CFP_OBSERVATION:{}",
        json!({"name":std::thread::current().name().unwrap_or("source"),"input":input,"expected":expected})
    )
}
fn parse_cfp_page(
    config: &CfpSourceConfig,
    document: &CfpDocument,
    checked: &str,
    discovery: bool,
) -> Result<CfpParsedPage, CfpSourceError> {
    let result = litradar_sources::cfp::parse_cfp_page(config, document, checked, discovery);
    let expected = match &result {
        Ok(page) => {
            json!({"Sources":page.sources,"EmptyJournals":page.empty_journals,"DetailUrls":page.detail_urls,"DetailTitles":page.detail_titles})
        }
        Err(error) => error_json(error),
    };
    emit(
        json!({"op":"parse","config":config,"document":document_json(document),"checked":checked,"discovery":discovery}),
        expected,
    );
    result
}
fn extract_cfp_full_text(
    source: &CfpSource,
    document: &CfpDocument,
) -> Result<CfpSource, CfpSourceError> {
    let result = litradar_sources::cfp::extract_cfp_full_text(source, document);
    let expected = match &result {
        Ok(source) => json!(source),
        Err(error) => error_json(error),
    };
    emit(
        json!({"op":"full","source":source,"document":document_json(document)}),
        expected,
    );
    result
}
fn cfp_original_links(source: &CfpSource, document: &CfpDocument) -> Vec<String> {
    let result = litradar_sources::cfp::cfp_original_links(source, document);
    emit(
        json!({"op":"links","source":source,"document":document_json(document)}),
        json!(result),
    );
    result
}
fn is_cfp_challenge(document: &CfpDocument) -> bool {
    let result = litradar_sources::cfp::is_cfp_challenge(document);
    emit(
        json!({"op":"challenge","document":document_json(document)}),
        json!(result),
    );
    result
}
struct ObservedTransport<'a, T: CfpTransport> {
    inner: &'a T,
    calls: std::sync::Mutex<Vec<Value>>,
}
impl<T: CfpTransport> CfpTransport for ObservedTransport<'_, T> {
    fn fetch(
        &self,
        config: &CfpSourceConfig,
        url: &str,
        deadline: Instant,
    ) -> Result<CfpDocument, CfpSourceError> {
        let result = self.inner.fetch(config, url, deadline);
        let document = match &result {
            Ok(document) => document_json(document),
            Err(error) => error_json(error),
        };
        self.calls
            .lock()
            .unwrap()
            .push(json!({"url":url,"result":document}));
        result
    }
}
fn acquire_cfp_source(
    transport: &impl CfpTransport,
    config: &CfpSourceConfig,
    checked: &str,
    deadline: Instant,
) -> Result<CfpAcquisition, CfpSourceError> {
    let observed = ObservedTransport {
        inner: transport,
        calls: std::sync::Mutex::new(Vec::new()),
    };
    let result = litradar_sources::cfp::acquire_cfp_source(&observed, config, checked, deadline);
    let expected = match &result {
        Ok(acquired) => {
            json!({"Documents":acquired.documents.iter().map(document_json).collect::<Vec<_>>(),"Sources":acquired.sources,"EmptyJournals":acquired.empty_journals})
        }
        Err(error) => error_json(error),
    };
    emit(
        json!({"op":"acquire","config":config,"checked":checked,"calls":observed.calls.into_inner().unwrap()}),
        expected,
    );
    result
}
