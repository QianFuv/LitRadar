//! Run original journal and issue repositories against a nonempty Rust-created database.
use litradar_storage::index::*;
use litradar_storage::{StorageConfig, migrate_index_database, open_sqlite_connection};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn optional_string(request: &Value, key: &str) -> Option<String> {
    request[key].as_str().map(str::to_owned)
}

/// Convert a successful original response or its exact repository error into a fixture observation.
fn observe(config: &StorageConfig, request: &Value) -> Result<Value, IndexRepositoryError> {
    let name = request["db"].as_str();
    let params = &request["params"];
    let limit = params["limit"].as_i64().unwrap_or(0);
    let offset = params["offset"].as_i64().unwrap_or(0);
    Ok(match request["operation"].as_str().unwrap() {
        "journals" => json!(list_journals(
            config,
            name,
            &JournalListParams {
                area: optional_string(params, "area"),
                ratings: serde_json::from_value(
                    params.get("ratings").cloned().unwrap_or(json!({}))
                )
                .unwrap(),
                has_articles: params["has_articles"].as_bool(),
                year: params["year"].as_i64(),
                sort: optional_string(params, "sort"),
                limit,
                offset,
            }
        )?),
        "issues" => json!(list_issues(
            config,
            name,
            &IssueListParams {
                journal_id: params["journal_id"].as_i64(),
                year: params["year"].as_i64(),
                sort: optional_string(params, "sort"),
                limit,
                offset,
            }
        )?),
        "journal" => json!(get_journal(config, name, request["id"].as_i64().unwrap())?),
        "issue" => json!(get_issue(config, name, request["id"].as_i64().unwrap())?),
        "areas" => json!(list_areas(config, name)?),
        "ratings" => json!(list_journal_ratings(config, name)?),
        "options" => json!(list_journal_options(config, name)?),
        "years" => json!(list_years(config, name)?),
        "issue_counts" => json!(collect_issue_article_counts(
            config.index_dir().join("metadata.sqlite")
        )?),
        "inpress_counts" => json!(collect_inpress_article_counts(
            config.index_dir().join("metadata.sqlite")
        )?),
        "issue_candidates" => json!(fetch_candidates_for_issue_keys(
            config.index_dir().join("metadata.sqlite"),
            &serde_json::from_value::<Vec<String>>(request["keys"].clone()).unwrap()
        )?),
        "inpress_candidates" => json!(fetch_candidates_for_inpress_keys(
            config.index_dir().join("metadata.sqlite"),
            &serde_json::from_value::<Vec<String>>(request["keys"].clone()).unwrap()
        )?),
        "article_candidates" => json!(fetch_candidates_for_article_ids(
            config.index_dir().join("metadata.sqlite"),
            &serde_json::from_value::<Vec<i64>>(request["ids"].clone()).unwrap()
        )?),
        "article" => json!(get_article(config, name, request["id"].as_i64().unwrap())?),
        "articles" => {
            let mut selected = ArticleListParams::default();
            if let Some(limit) = params["limit"].as_i64() {
                selected.limit = limit;
            }
            selected.offset = offset;
            selected.journal_id =
                serde_json::from_value(params.get("journal_id").cloned().unwrap_or(json!([])))
                    .unwrap();
            selected.issue_id = params["issue_id"].as_i64();
            selected.year = params["year"].as_i64();
            selected.area =
                serde_json::from_value(params.get("area").cloned().unwrap_or(json!([]))).unwrap();
            selected.ratings =
                serde_json::from_value(params.get("ratings").cloned().unwrap_or(json!({})))
                    .unwrap();
            selected.in_press = params["in_press"].as_bool();
            selected.open_access = params["open_access"].as_bool();
            selected.date_from = optional_string(params, "date_from");
            selected.date_to = optional_string(params, "date_to");
            selected.doi = optional_string(params, "doi");
            selected.pmid = optional_string(params, "pmid");
            selected.q = optional_string(params, "q");
            if let Some(mode) = params.get("search_mode") {
                selected.search_mode = serde_json::from_value(mode.clone()).unwrap();
            }
            if params.get("sort").is_some() {
                selected.sort = optional_string(params, "sort");
            }
            selected.cursor = optional_string(params, "cursor");
            selected.include_total = params["include_total"].as_bool();
            json!(list_articles(config, name, &selected)?)
        }
        _ => panic!("unknown metadata observation"),
    })
}

/// Create the fixture through original migration and preserve original query serialization.
fn main() -> std::io::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let config = StorageConfig::from_project_root(&args[1]);
    std::fs::create_dir_all(config.index_dir())?;
    let filename = config.index_dir().join("metadata.sqlite");
    assert!(
        !filename.exists(),
        "refusing to overwrite an existing fixture"
    );
    migrate_index_database(&filename).unwrap();
    open_sqlite_connection(&filename)
        .unwrap()
        .execute_batch(&std::fs::read_to_string(&args[2])?)
        .unwrap();
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
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
    open_sqlite_connection(filename)
        .unwrap()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .unwrap();
    output.flush()
}
