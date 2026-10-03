//! Observe unchanged canonical normalization, registry policy and provider conformance.
use litradar_domain::*;
use litradar_provider::conformance::*;
use litradar_provider::*;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::Arc;

struct NoExecution;
impl IndexContentProvider for NoExecution {
    fn fetch(
        &self,
        _: &JournalCatalogEntry,
        _: IndexFetchContext<'_>,
    ) -> Result<ProviderBatch, ProviderError> {
        unreachable!("registry oracle never fetches")
    }
}
impl ArticleAbstractProvider for NoExecution {
    fn supports_abstract(&self, _: &ArticleLocator) -> bool {
        unreachable!("registry oracle never resolves")
    }
    fn resolve_abstract(
        &self,
        _: &ArticleLocator,
        _: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError> {
        unreachable!("registry oracle never resolves")
    }
}
impl ArticleFullTextProvider for NoExecution {
    fn supports_full_text(&self, _: &ArticleLocator) -> bool {
        unreachable!("registry oracle never resolves")
    }
    fn resolve_full_text(
        &self,
        _: &ArticleLocator,
        _: ArticleAccessContext,
    ) -> Result<ArticleFullTextResolution, ProviderError> {
        unreachable!("registry oracle never resolves")
    }
}

fn verdict(result: Result<(), impl std::fmt::Display>) -> Value {
    match result {
        Ok(()) => json!({"error":null}),
        Err(error) => json!({"error":error.to_string()}),
    }
}

fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let input = &request["input"];
        macro_rules! decode {($kind:ty)=>{match serde_json::from_str::<$kind>(input.as_str().unwrap()){Ok(value)=>json!({"valid":true,"value":value}),Err(_)=>json!({"valid":false})}};}
        let result = match request["kind"].as_str().unwrap() {
            "decode" => match request["type"].as_str().unwrap() {
                "catalog" => decode!(JournalCatalogEntry),
                "rankings" => decode!(JournalRankings),
                "journal" => decode!(JournalDraft),
                "issue" => decode!(IssueDraft),
                "author" => decode!(ArticleAuthorDraft),
                "article" => decode!(ArticleDraft),
                "batch" => decode!(ProviderBatch),
                "progress" => decode!(ProviderProgress),
                "mode" => decode!(IndexSyncMode),
                _ => panic!("unknown contract"),
            },
            "normalize" => {
                let value = input.as_str().unwrap();
                json!({"text":normalize_contract_text(value),"date":normalize_contract_date(value).map(|date|json!({"value":date.value,"precision":date.precision})),"doi":normalize_contract_doi(value),"pmid":normalize_contract_pmid(value),"issn":normalize_contract_issn(value),"bibliographic":normalize_bibliographic_text(value),"label":normalize_bibliographic_label(value),"lowercase":value.to_lowercase()})
            }
            "catalog" => verdict(validate_catalog_entry(
                &serde_json::from_value(input.clone()).unwrap(),
            )),
            "batch" => verdict(validate_provider_batch(
                &serde_json::from_value(input["catalog"].clone()).unwrap(),
                &serde_json::from_value(input["batch"].clone()).unwrap(),
            )),
            "redirect" => verdict(validate_article_redirect(&ArticleRedirect {
                location: input.as_str().unwrap().to_string(),
            })),
            "document" => {
                let document = ArticleFullTextDocument {
                    content_type: input["content_type"].as_str().unwrap().to_string(),
                    filename: input["filename"].as_str().map(str::to_string),
                    bytes: vec![b'a'; input["size"].as_u64().unwrap() as usize],
                };
                verdict(validate_full_text_resolution(
                    &ArticleFullTextResolution::Document(document),
                    input["maximum"].as_u64().unwrap() as usize,
                ))
            }
            "registry" => {
                let flags = &input["capabilities"];
                let implementations = &input["implementations"];
                let descriptor = ProviderDescriptor {
                    name: input["name"].as_str().unwrap().to_string(),
                    capabilities: ProviderCapabilities {
                        index_content: flags[0].as_bool().unwrap(),
                        article_abstract: flags[1].as_bool().unwrap(),
                        article_full_text: flags[2].as_bool().unwrap(),
                    },
                    allowed_redirect_hosts: serde_json::from_value(input["hosts"].clone()).unwrap(),
                };
                let implementations = ProviderImplementations {
                    index_content: implementations[0]
                        .as_bool()
                        .unwrap()
                        .then(|| Arc::new(NoExecution) as Arc<dyn IndexContentProvider>),
                    article_abstract: implementations[1]
                        .as_bool()
                        .unwrap()
                        .then(|| Arc::new(NoExecution) as Arc<dyn ArticleAbstractProvider>),
                    article_full_text: implementations[2]
                        .as_bool()
                        .unwrap()
                        .then(|| Arc::new(NoExecution) as Arc<dyn ArticleFullTextProvider>),
                };
                verdict(ProviderRegistration::try_new(descriptor, implementations).map(|_| ()))
            }
            _ => panic!("unknown provider observation"),
        };
        request["output"] = result;
        writeln!(output, "{request}")?;
    }
    output.flush()
}
