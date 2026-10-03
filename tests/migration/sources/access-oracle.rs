//! Observe original built-in abstract access adapters through their public interfaces.
use litradar_domain::{ArticleAccessContext, ArticleId, ArticleLocator};
use litradar_provider::ArticleAbstractProvider;
use litradar_sources::{
    DomesticCnkiArticleAccessProvider, FixtureDomesticCnkiTransport, ScholarlyArticleAccessProvider,
};
use serde_json::{Value, json};
use std::io::{self, BufRead};

fn main() {
    for line in io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let input = &request["article"];
        let text = |name: &str| input[name].as_str().map(str::to_owned);
        let article = ArticleLocator {
            article_id: ArticleId(1),
            catalog_id: "catalog".into(),
            journal_title: text("journal_title").unwrap_or_default(),
            journal_issns: input
                .get("journal_issns")
                .map(|value| serde_json::from_value(value.clone()).unwrap())
                .unwrap_or_default(),
            title: text("title").unwrap_or_default(),
            publication_year: input["publication_year"].as_i64(),
            date: text("date"),
            authors: Vec::new(),
            volume: text("volume"),
            issue_number: text("issue_number"),
            start_page: None,
            end_page: None,
            doi: text("doi"),
            pmid: text("pmid"),
        };
        let provider: Box<dyn ArticleAbstractProvider> = if request["kind"] == "scholarly" {
            Box::new(ScholarlyArticleAccessProvider)
        } else {
            Box::new(DomesticCnkiArticleAccessProvider::new(
                FixtureDomesticCnkiTransport::new(
                    serde_json::from_value(request["fixture"].clone()).unwrap(),
                ),
            ))
        };
        let supports = provider.supports_abstract(&article);
        let outcome = match provider.resolve_abstract(&article, ArticleAccessContext::default()) {
            Ok(result) => json!({"location":result.location}),
            Err(error) => json!({"error":error.to_string(),"kind":format!("{:?}",error.kind())}),
        };
        request["output"] = json!({"supports":supports,"outcome":outcome});
        println!("{request}");
    }
}
