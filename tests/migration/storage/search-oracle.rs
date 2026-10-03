//! Observe unchanged Rust text, FTS operand and author-decoding behavior.
#[path = "../../../crates/litradar-storage/src/article_authors.rs"]
mod article_authors;

use litradar_domain::ArticleSearchMode;
use litradar_storage::search_text::{prepare_search_query, prepare_search_text};
use std::io::{BufRead, Write};

/// Stream scalar blocks and bounded requests without consulting Go output.
fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: serde_json::Value = serde_json::from_str(&line?).unwrap();
        match request["kind"].as_str().unwrap() {
            "scalars" => {
                for start in (0..0x110000).step_by(4096) {
                    let values: Vec<_> = (start..start + 4096)
                        .filter_map(char::from_u32)
                        .map(|character| {
                            prepare_search_text(&character.to_string(), true).into_owned()
                        })
                        .collect();
                    writeln!(
                        output,
                        "{}",
                        serde_json::json!({"start":start,"values":values})
                    )?;
                }
            }
            "authors" => {
                let result = article_authors::decode_article_author_names(
                    request["input"].as_str().unwrap(),
                );
                request["valid"] = result.is_ok().into();
                request["output"] = result.ok().into();
                writeln!(output, "{request}")?;
            }
            "text" => {
                let input = request["input"].as_str().unwrap();
                let uses_simple = request["uses_simple"].as_bool().unwrap();
                let mode = if request["mode"] == "advanced" {
                    ArticleSearchMode::Advanced
                } else {
                    ArticleSearchMode::Simple
                };
                let text = prepare_search_text(input, uses_simple).into_owned();
                let query = prepare_search_query(input, uses_simple, mode).into_owned();
                request["text"] = text.into();
                request["query"] = query.into();
                writeln!(output, "{request}")?;
            }
            _ => panic!("unknown observation"),
        }
    }
    output.flush()
}
