//! Observe original weekly queries with visibility-only access to their fixed-clock entry points.
pub use litradar_storage::{
    DatabaseResolutionError, StorageConfig, open_sqlite_connection, search_text,
};
#[allow(dead_code)]
#[path = "../../../output/migration/execution/weekly-query-index/mod.rs"]
mod index;
pub use index::IndexRepositoryError;
#[path = "../../../crates/litradar-storage/src/article_authors.rs"]
mod article_authors;
#[path = "../../../crates/litradar-storage/src/weekly_manifest.rs"]
mod weekly_manifest;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn observe(config: &StorageConfig, request: &Value) -> Result<Value, IndexRepositoryError> {
    let end = DateTime::parse_from_rfc3339(request["end"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    Ok(match request["operation"].as_str().unwrap() {
        "summary" => json!(index::get_weekly_updates_summary_at(config, end)?),
        "legacy" => json!(index::get_weekly_updates_at(config, end)?),
        "available" => json!(weekly_manifest::load_available_weekly_manifests(config,end,&serde_json::from_value::<Vec<String>>(request["selected"].clone()).unwrap())?.into_iter().map(|manifest| json!({"db_name":manifest.db_name,"run_id":manifest.run_id,"ids":manifest.article_ids.iter().map(i64::to_string).collect::<Vec<_>>()})).collect::<Vec<_>>()),
        "page" => {
            let params=&request["params"];
            json!(index::get_weekly_update_articles(config,&index::WeeklyArticlePageParams{
                db_name:params["db"].as_str().unwrap().to_owned(),
                journal_id:params["journal_id"].as_i64().unwrap(),
                window_end:params["window_end"].as_str().unwrap_or(request["end"].as_str().unwrap()).to_owned(),
                q:params["q"].as_str().map(str::to_owned),limit:params["limit"].as_i64().unwrap(),
                cursor:params["cursor"].as_str().map(str::to_owned),
            })?)
        },
        _ => panic!("unknown observation"),
    })
}

/// Use a fresh copy per case so mutations cannot leak into subsequent observations.
fn main() -> std::io::Result<()> {
    let arguments: Vec<_> = std::env::args().collect();
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for (position, line) in std::io::stdin().lock().lines().enumerate() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let config = StorageConfig::from_project_root(
            std::path::Path::new(&arguments[1]).join(position.to_string()),
        );
        std::fs::create_dir_all(config.index_dir())?;
        let filename = config.index_dir().join("metadata.sqlite");
        std::fs::copy(&arguments[2], &filename)?;
        if let Some(sql) = request["sql"].as_str() {
            open_sqlite_connection(&filename)
                .unwrap()
                .execute_batch(sql)
                .unwrap();
        }
        let manifests = config.project_root().join("data").join("push_state");
        std::fs::create_dir_all(&manifests)?;
        for (name, raw) in request["manifests"].as_object().unwrap() {
            std::fs::write(manifests.join(name), raw.as_str().unwrap())?;
        }
        match observe(&config, &request) {
            Ok(result) => {
                request["output"] = result;
                request["error"] = Value::Null;
            }
            Err(error) => {
                request["output"] = Value::Null;
                request["error"] = error.to_string().into();
            }
        }
        writeln!(output, "{request}")?;
    }
    output.flush()
}
