//! Verify and mutate Go-produced databases through unchanged original Rust storage APIs.
use litradar_storage::*;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn observe(request: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let path = request["path"].as_str().unwrap_or(".");
    match request["operation"].as_str().unwrap() {
        "optimize" => Ok(json!(optimize_index_storage(
            &IndexStorageOptimizationOptions {
                storage_config: StorageConfig::from_project_root(path),
                confirmed: true,
            }
        )?)),
        "index" => {
            preflight_index_database(path)?;
            migrate_index_database(path)?;
            let connection = rusqlite::Connection::open(path)?;
            sqlite::load_index_tokenizer(&connection)?;
            let mut counts = serde_json::Map::new();
            for table in [
                "journals",
                "journal_identity_keys",
                "issues",
                "articles",
                "article_retraction_dois",
                "article_identity_keys",
                "article_listing",
                "article_change_events",
                "article_search",
            ] {
                let count: i64 =
                    connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })?;
                counts.insert(table.into(), json!(count));
            }
            let mut statement = connection.prepare("SELECT rowid FROM article_search WHERE article_search MATCH 'article' ORDER BY rowid")?;
            let ids = statement
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            if request["write"].as_bool().unwrap_or(false) {
                connection.execute("INSERT INTO articles(article_id,journal_id,title,authors_json) VALUES(9999,1,'Rust exchange article','[\"Writer\"]')", [])?;
                search_text::rebuild_article_search(&connection)?;
            }
            Ok(json!({"counts":counts,"search":ids}))
        }
        _ => panic!("unknown operation"),
    }
}

fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?).unwrap();
        let result = match observe(&request) {
            Ok(value) => json!({"output":value,"error":null}),
            Err(error) => json!({"output":null,"error":error.to_string()}),
        };
        writeln!(output, "{result}")?;
    }
    output.flush()
}
