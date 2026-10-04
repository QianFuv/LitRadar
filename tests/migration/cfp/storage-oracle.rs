//! Observe the unchanged Rust CFP domain and durable repository on disposable databases.
use litradar_domain::cfp::*;
use litradar_storage::business::cfp::*;
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Value, json};
use std::io::{self, BufRead};
use std::path::Path;

fn text<'a>(value: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    value[key].as_str().unwrap_or(fallback)
}
fn number(value: &Value, key: &str, fallback: i64) -> i64 {
    value[key].as_i64().unwrap_or(fallback)
}
fn lease(step: &Value) -> CfpRefreshLease {
    CfpRefreshLease {
        source_key: text(step, "source", "journal:a").into(),
        generation: number(step, "generation", 1),
        expires_at: number(step, "expires", 110),
    }
}
fn publication(step: &Value) -> Result<CfpPublication, CfpRepositoryError> {
    Ok(CfpPublication {
        capture: text(step, "capture", "<main>verified</main>").into(),
        capture_format: text(step, "format", "html").into(),
        source_url: text(step, "url", "https://example.org/cfp").into(),
        config_version: step["version"].as_u64().unwrap_or(1) as u32,
        sources: serde_json::from_value(step.get("sources").cloned().unwrap_or(json!([])))?,
        empty_journals: serde_json::from_value(step.get("empty").cloned().unwrap_or(json!([])))?,
    })
}
fn perform(path: &Path, step: &Value) -> Result<Value, CfpRepositoryError> {
    let source = text(step, "source", "journal:a");
    match text(step,"op","") {
        "import"=>Ok(json!(import_cfp_seed(path,text(step,"id","seed"),text(step,"payload","").as_bytes())?)),
        "begin"=>{let value=begin_cfp_refresh(path,source,number(step,"now",100),number(step,"seconds",10))?;Ok(json!({"sourceKey":value.source_key,"generation":value.generation,"expiresAt":value.expires_at}))},
        "fail"=>{fail_cfp_refresh(path,&lease(step),text(step,"reason","failed"),step["unsupported"].as_bool().unwrap_or(false))?;Ok(json!("ok"))},
        "publish"=>{publish_cfp_refresh(path,&lease(step),&publication(step)?,number(step,"now",105))?;Ok(json!("ok"))},
        "full"=>{publish_cfp_full_text_refresh(path,&lease(step),&publication(step)?,number(step,"now",105),step["unresolved"].as_u64().unwrap_or(0) as usize)?;Ok(json!("ok"))},
        "originals"=>Ok(json!(load_cfp_source_originals(path,source)?)),
        "load"=>Ok(json!(load_cfp_journals(path)?.into_iter().map(|value|json!({"JournalKey":value.journal_key,"CatalogIds":value.catalog_ids,"JournalTitle":value.journal_title,"CheckedOn":value.checked_on,"SourceUrl":value.source_url,"SourceStatement":value.source_statement,"Notices":value.notices,"Sources":value.sources})).collect::<Vec<_>>())),
        "noop"=>Ok(json!("ok")),
        _=>panic!("unknown operation")
    }
}
fn snapshot(path: &Path) -> Value {
    let connection = Connection::open(path).unwrap();
    let mut tables = serde_json::Map::new();
    for (table, order) in [
        ("cfp_journals", "journal_key"),
        ("cfp_journal_aliases", "catalog_id"),
        ("cfp_sources", "source_key"),
        ("cfp_source_journals", "source_key,journal_key"),
        ("cfp_notices", "journal_key,notice_key"),
        ("cfp_seed_imports", "seed_id"),
    ] {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .unwrap();
        let count = statement.column_count();
        let rows=statement.query_map([],|row|Ok((0..count).map(|index|match row.get_ref(index).unwrap(){ValueRef::Null=>Value::Null,ValueRef::Integer(value)=>json!({"integer":value.to_string()}),ValueRef::Real(value)=>json!({"real":value}),ValueRef::Text(value)=>json!({"text":std::str::from_utf8(value).unwrap()}),ValueRef::Blob(value)=>json!({"blob":value.iter().map(|byte|format!("{byte:02x}")).collect::<String>()})}).collect::<Vec<_>>())).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        tables.insert(table.into(), json!(rows));
    }
    Value::Object(tables)
}
fn domain(input: &Value) -> Value {
    match text(input, "op", "") {
        "dates" => {
            let stage: Result<CfpDateStage, _> =
                serde_json::from_value(input.get("stage").cloned().unwrap_or(json!("paper")));
            match stage {
                Ok(stage) => json!(parse_cfp_dates(text(input, "text", ""), stage)),
                Err(_) => json!({"error":"json"}),
            }
        }
        "source" => {
            let source: Result<CfpSource, _> = serde_json::from_str(text(input, "raw", ""));
            match source {
                Ok(source) => {
                    let notice = parse_cfp_source(&source);
                    let states = notice.as_ref().map(|notice| {
                        input["times"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|instant| notice.state(instant.as_str().unwrap().parse().unwrap()))
                            .collect::<Vec<_>>()
                    });
                    json!({"source":source,"notice":notice,"states":states})
                }
                Err(_) => json!({"error":"json"}),
            }
        }
        "seed" => match serde_json::from_str::<CfpSeed>(text(input, "raw", "")) {
            Ok(seed) => json!(seed),
            Err(_) => json!({"error":"json"}),
        },
        "notice" => match serde_json::from_str::<CfpNotice>(text(input, "raw", "")) {
            Ok(notice) => json!(notice),
            Err(_) => json!({"error":"json"}),
        },
        _ => panic!("unknown domain observation"),
    }
}
fn main() {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if input["mode"] == "domain" {
            println!("{}", domain(&input));
            continue;
        };
        let path = Path::new(input["path"].as_str().unwrap());
        if !input["resume"].as_bool().unwrap_or(false) {
            litradar_storage::migrate_auth_database(path).unwrap()
        };
        let mut observations = Vec::new();
        for step in input["steps"].as_array().unwrap() {
            if let Some(sql) = step["sql"].as_str() {
                Connection::open(path).unwrap().execute_batch(sql).unwrap()
            };
            let outcome = match perform(path, step) {
                Ok(value) => value,
                Err(error) => {
                    json!({"error":match error{CfpRepositoryError::Invalid(message)=>message,CfpRepositoryError::StaleRefresh=>"CFP refresh was superseded or expired".into(),CfpRepositoryError::Json(_)=>"json".into(),CfpRepositoryError::Sqlite(_)=>"storage".into()}})
                }
            };
            observations.push(json!({"outcome":outcome,"tables":snapshot(path)}))
        }
        println!("{}", json!(observations))
    }
}
