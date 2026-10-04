//! Observe the original identity and catalog implementations without changing production sources.

#[path = "../../../crates/litradar-index/src/identity.rs"]
mod identity;

#[path = "control-observer.rs"]
mod control;

#[path = "../../../crates/litradar-index/src/changes.rs"]
mod changes;

use litradar_index::transforms;
#[path = "../../../crates/litradar-index/src/batch.rs"]
mod batch;
#[path = "batch-observer.rs"]
mod batch_observer;
#[path = "notify-observer.rs"]
mod notify_observer;
#[path = "worker-observer.rs"]
mod worker_observer;
#[path = "../../../crates/litradar-index/src/worker_protocol.rs"]
mod worker_protocol;

fn manifest(input: &Value) -> Value {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE article_change_events(event_id INTEGER PRIMARY KEY,content_revision TEXT,article_id INTEGER,change_kind TEXT,journal_id INTEGER,issue_id INTEGER,in_press INTEGER,created_at TEXT)").unwrap();
    for event in input["events"].as_array().unwrap() {
        let number = |field: &str| event[field].as_str().unwrap().parse::<i64>().unwrap();
        connection
            .execute(
                "INSERT INTO article_change_events VALUES(?1,'revision',?2,?3,?4,?5,?6,'epoch')",
                rusqlite::params![
                    number("event_id"),
                    number("article_id"),
                    event["kind"].as_str().unwrap(),
                    number("journal_id"),
                    event["issue_id"]
                        .as_str()
                        .map(|value| value.parse::<i64>().unwrap()),
                    number("in_press")
                ],
            )
            .unwrap();
    }
    let prepared = changes::prepare_content_change_manifest(
        &connection,
        input["database"].as_str().unwrap(),
        input["run"].as_str().unwrap(),
        input["generated"].as_str().unwrap(),
    )
    .unwrap();
    let mut limits = Vec::new();
    for limit in [0, 1, 1000, 10000, 10001] {
        limits.push(
            match changes::list_content_change_events(&connection, 0, limit) {
                Ok(values) => json!({"count":values.len()}),
                Err(error) => json!({"error":error.to_string()}),
            },
        );
    }
    json!({"payload":String::from_utf8(prepared.payload).unwrap(),"through":prepared.through_event_id.map(|value|value.to_string()),"count":prepared.event_count,"limits":limits})
}

use litradar_domain::{ArticleDraft, IssueDraft};
use serde_json::{Value, json};

fn snapshot(connection: &rusqlite::Connection) -> Value {
    snapshot_selected(
        connection,
        &[
            "journals",
            "journal_identity_keys",
            "issues",
            "articles",
            "article_retraction_dois",
            "article_identity_keys",
            "article_listing",
            "article_search",
            "article_change_events",
        ],
    )
}

fn snapshot_selected(connection: &rusqlite::Connection, selected: &[&str]) -> Value {
    let mut tables = serde_json::Map::new();
    for table in selected {
        if !connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [table],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        {
            continue;
        }
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
            .unwrap();
        let count = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..count)
                    .map(|column| {
                        Ok(match row.get_ref(column)? {
                            rusqlite::types::ValueRef::Null => Value::Null,
                            rusqlite::types::ValueRef::Integer(value) => {
                                json!({"integer":value.to_string()})
                            }
                            rusqlite::types::ValueRef::Real(value) => {
                                json!({"real":value.to_string()})
                            }
                            rusqlite::types::ValueRef::Text(value) => {
                                json!(String::from_utf8_lossy(value))
                            }
                            rusqlite::types::ValueRef::Blob(value) => json!({"blob":value}),
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        tables.insert(table.to_string(), json!(rows));
    }
    Value::Object(tables)
}

fn content(input: &Value) -> Value {
    let connection = match input["path"].as_str() {
        Some(path) => rusqlite::Connection::open(path).unwrap(),
        None => rusqlite::Connection::open_in_memory().unwrap(),
    };
    litradar_index::schema::init_content_db(&connection).unwrap();
    let mut operations = Vec::new();
    for operation in input["operations"].as_array().unwrap() {
        let result = match operation["op"].as_str().unwrap() {
            "sql" => connection.execute_batch(operation["sql"].as_str().unwrap()).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "reconcile" => litradar_index::schema::reconcile_catalog_identities(&connection,&serde_json::from_value::<Vec<_>>(operation["catalogs"].clone()).unwrap()).map(|_|json!({"ok":true})).map_err(|error|error.to_string()),
            "write" => litradar_index::schema::write_content_batch(&connection,&serde_json::from_value(operation["catalog"].clone()).unwrap(),&serde_json::from_value(operation["batch"].clone()).unwrap(),operation["revision"].as_str().unwrap(),"2026-10-04T00:00:00Z").map(|value|json!({"articles_seen":value.articles_seen,"articles_changed":value.articles_changed,"identity_aliases_added":value.identity_aliases_added,"change_events_emitted":value.change_events_emitted})).map_err(|error|error.to_string()),
            _ => panic!("unknown content operation"),
        };
        operations.push(match result {
            Ok(value) => value,
            Err(error) => json!({"error":error}),
        });
    }
    json!({"operations":operations,"tables":snapshot(&connection)})
}
use std::collections::BTreeMap;
use std::io::{self, BufRead};

fn observe(input: &Value) -> Value {
    match input["op"].as_str().unwrap() {
        "content" => content(input),
        "control" => control::observe(input),
        "manifest" => manifest(input),
        "batch" => batch_observer::observe(input),
        "worker_wire" => worker_observer::observe(input),
        "notify_wire" => notify_observer::observe(input),
        "catalog_path" => {
            let path = std::path::Path::new(input["path"].as_str().unwrap());
            json!({"filename":path.file_name().unwrap().to_str().unwrap(),"csv":path.extension().and_then(|value|value.to_str()) == Some("csv")})
        }
        "journal" => json!(
            identity::journal_id_from_catalog_id(input["catalog_id"].as_str().unwrap()).to_string()
        ),
        "issue" => {
            let issue: IssueDraft = serde_json::from_value(input["issue"].clone()).unwrap();
            let journal = input["journal_id"].as_str().unwrap().parse().unwrap();
            json!({"key":identity::issue_identity_value(journal, &issue), "id":identity::issue_id_from_draft(journal,&issue).map(|id|id.to_string())})
        }
        "identity" => {
            let article: ArticleDraft = serde_json::from_value(input["article"].clone()).unwrap();
            let keys = identity::article_identity_keys(&article);
            let aliases = input["aliases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|alias| {
                    let kind = match alias["kind"].as_str().unwrap() {
                        "doi" => identity::ArticleIdentityKind::Doi,
                        "pmid" => identity::ArticleIdentityKind::Pmid,
                        "bibliographic" => identity::ArticleIdentityKind::Bibliographic,
                        _ => panic!("unknown identity kind"),
                    };
                    (
                        identity::ArticleIdentityKey {
                            kind,
                            value: alias["value"].as_str().unwrap().to_string(),
                        },
                        alias["owner"].as_str().unwrap().parse().unwrap(),
                    )
                })
                .collect::<BTreeMap<_, i64>>();
            let resolution = match identity::resolve_article_identity(&article, &aliases) {
                Ok(value) => {
                    json!({"article_id":value.article_id.to_string(),"is_existing":value.is_existing,"identity_key":{"kind":value.identity_key.kind.as_str(),"value":value.identity_key.value}})
                }
                Err(error) => json!({"error":error.to_string()}),
            };
            json!({"keys":keys.iter().map(|key|json!({"kind":key.kind.as_str(),"value":key.value})).collect::<Vec<_>>(),"resolution":resolution})
        }
        "merge" => {
            let left: ArticleDraft = serde_json::from_value(input["left"].clone()).unwrap();
            let right: ArticleDraft = serde_json::from_value(input["right"].clone()).unwrap();
            let result = if input["resolved"].as_bool().unwrap() {
                identity::merge_resolved_article_drafts(&left, &right)
            } else {
                identity::merge_article_drafts(&left, &right)
            };
            match result {
                Ok(value) => json!({"article":value}),
                Err(error) => json!({"error":error.to_string()}),
            }
        }
        "catalog" => {
            let result =
                litradar_index::transforms::parse_catalog_csv(input["csv"].as_str().unwrap());
            match result {
                Ok(value) => json!({"entries":value}),
                Err(error) => json!({"error":error.to_string()}),
            }
        }
        "catalog_rows" => {
            let rows: Vec<BTreeMap<String, String>> =
                serde_json::from_value(input["rows"].clone()).unwrap();
            match litradar_index::transforms::build_catalog_entries(&rows) {
                Ok(value) => json!({"entries":value}),
                Err(error) => json!({"error":error.to_string()}),
            }
        }
        _ => panic!("unknown observation"),
    }
}

fn main() {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", json!({"input":input,"expected":observe(&input)}));
    }
}
