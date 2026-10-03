//! Observe original favorite reads and validation against Rust-created owned folders.
use litradar_domain::UserId;
use litradar_storage::{
    StorageConfig, business::*, initialize_auth_database, open_sqlite_connection,
};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn observe(config: &StorageConfig, request: &Value) -> Result<Value, BusinessRepositoryError> {
    let owner = UserId(request["owner"].as_i64().unwrap_or(1));
    let folder = request["folder"].as_i64().unwrap_or(10);
    let limit = request["limit"].as_i64().unwrap_or(50);
    Ok(match request["operation"].as_str().unwrap() {
        "folders" => json!(list_folders(config.auth_db_path(), owner)?),
        "tracking" => json!(get_tracking_folder(config.auth_db_path(), owner)?),
        "count" => json!(count_favorites(
            config.auth_db_path(),
            owner,
            request["folder"].as_i64()
        )?),
        "list" => json!(list_favorite_articles(
            config,
            owner,
            request["folder"].as_i64(),
            limit,
            request["offset"].as_i64().unwrap_or(0)
        )?),
        "page" => json!(list_favorite_article_page(
            config,
            owner,
            folder,
            limit,
            request["cursor"].as_str()
        )?),
        "snapshot" => {
            let snapshot = load_favorite_citation_snapshot(
                config.auth_db_path(),
                owner,
                folder,
                limit as usize,
            )?;
            json!({"folder_name":snapshot.folder_name,"has_more":snapshot.has_more,"references":snapshot.references.into_iter().map(|item|json!({"article_id":item.article_id,"db_name":item.db_name})).collect::<Vec<_>>()})
        }
        "citation" => {
            let references = request["references"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| FavoriteCitationReference {
                    article_id: litradar_domain::ArticleId(item["id"].as_i64().unwrap()),
                    db_name: item["db"].as_str().unwrap().to_owned(),
                })
                .collect::<Vec<_>>();
            json!(load_favorite_citation_records(config,&references)?.into_iter().map(|item|json!({"article_id":item.article_id,"db_name":item.db_name,"title":item.title,"authors":item.authors,"journal_title":item.journal_title,"date":item.date,"doi":item.doi})).collect::<Vec<_>>())
        }
        "check" => json!(is_favorited(
            config.auth_db_path(),
            owner,
            request["id"].as_i64().unwrap(),
            request["db"].as_str().unwrap()
        )?),
        "batch" => json!(batch_is_favorited(
            config.auth_db_path(),
            owner,
            &serde_json::from_value::<Vec<i64>>(request["ids"].clone()).unwrap(),
            request["db"].as_str().unwrap()
        )?),
        _ => panic!("unknown operation"),
    })
}

/// Copy the same pre-mutation fixture into every isolated observation directory.
fn main() -> std::io::Result<()> {
    let arguments: Vec<_> = std::env::args().collect();
    let base = std::path::Path::new(&arguments[1]);
    let seed = base.join("auth.sqlite");
    initialize_auth_database(&seed).unwrap();
    open_sqlite_connection(&seed)
        .unwrap()
        .execute_batch(&std::fs::read_to_string(&arguments[3])?)
        .unwrap();
    open_sqlite_connection(&seed)
        .unwrap()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .unwrap();
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for (position, line) in std::io::stdin().lock().lines().enumerate() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let config = StorageConfig::from_project_root(base.join(position.to_string()));
        std::fs::create_dir_all(config.index_dir())?;
        std::fs::copy(&seed, config.auth_db_path())?;
        let index = config.index_dir().join("metadata.sqlite");
        std::fs::copy(&arguments[2], &index)?;
        if request["ambiguous"].as_bool().unwrap_or(false) {
            std::fs::copy(&index, config.index_dir().join("other.sqlite"))?;
        }
        if let Some(sql) = request["sql"].as_str() {
            open_sqlite_connection(config.auth_db_path())
                .unwrap()
                .execute_batch(sql)
                .unwrap();
        }
        if let Some(sql) = request["index_sql"].as_str() {
            open_sqlite_connection(&index)
                .unwrap()
                .execute_batch(sql)
                .unwrap();
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
