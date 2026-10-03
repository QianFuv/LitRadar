//! Execute the unchanged original citation serializers on synthetic observations.
#[path = "../../../crates/litradar-api/src/citation.rs"]
mod citation;

use litradar_storage::business::FavoriteCitationRecord;
use std::io::{BufRead, Write};

/// Preserve null fields when constructing the original storage projection.
fn optional(value: &serde_json::Value, name: &str) -> Option<String> {
    value[name].as_str().map(str::to_owned)
}

/// Read one bounded request per line and emit the original serializer's result.
fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut value: serde_json::Value = serde_json::from_str(&line?).unwrap();
        let articles: Vec<_> = value["articles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|article| FavoriteCitationRecord {
                article_id: serde_json::from_str("\"1001\"").unwrap(),
                db_name: "synthetic.sqlite".into(),
                title: optional(article, "title"),
                authors: article["authors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|author| author.as_str().unwrap().to_owned())
                    .collect(),
                journal_title: optional(article, "journal_title"),
                date: optional(article, "date"),
                doi: optional(article, "doi"),
            })
            .collect();
        let maximum = value["maximum_bytes"].as_u64().unwrap() as usize;
        let result = match value["format"].as_str().unwrap() {
            "bibtex" => citation::serialize_bibtex(&articles, maximum),
            "ris" => citation::serialize_ris(&articles, maximum),
            "endnote" => citation::serialize_endnote_xml(&articles, maximum),
            _ => panic!("unknown citation format"),
        };
        value["limited"] = result.is_err().into();
        value["output"] = result.ok().into();
        writeln!(output, "{value}")?;
    }
    output.flush()
}
