//! Observe the original all-or-nothing legacy importer and its persisted table contents.

use litradar_storage::{StorageConfig, import_legacy_delivery_state_files, migrate_auth_database};
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Value, json};
use std::io::{self, BufRead};
use std::path::Path;

fn snapshot(path: &Path) -> Value {
    let connection = Connection::open(path).unwrap();
    let mut tables = serde_json::Map::new();
    for table in [
        "delivery_checkpoints",
        "delivery_runs",
        "delivery_run_items",
        "delivery_dedupe",
        "delivery_leases",
    ] {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY id"))
            .unwrap();
        let count = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                Ok((0..count)
                    .map(|index| match row.get_ref(index).unwrap() {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(value) => json!({"integer":value.to_string()}),
                        ValueRef::Real(value) => json!({"real":value}),
                        ValueRef::Text(value) => {
                            json!({"text":std::str::from_utf8(value).unwrap()})
                        }
                        ValueRef::Blob(_) => panic!("unexpected blob"),
                    })
                    .collect::<Vec<_>>())
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        tables.insert(table.into(), json!(rows));
    }
    Value::Object(tables)
}

fn observe(input: &Value) -> Value {
    let root = Path::new(input["root"].as_str().unwrap());
    let config = StorageConfig::from_project_root(root);
    migrate_auth_database(config.auth_db_path()).unwrap();
    Connection::open(config.auth_db_path()).unwrap().execute_batch("INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'fixture','hash','salt',1,1),(2,'second','hash','salt',1,1)").unwrap();
    let mut observations = Vec::new();
    for step in input["steps"].as_array().unwrap() {
        for file in step["files"].as_array().unwrap() {
            let path = root.join(file["path"].as_str().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file["body"].as_str().unwrap()).unwrap();
        }
        if let Some(sql) = step["sql"].as_str() {
            Connection::open(config.auth_db_path())
                .unwrap()
                .execute_batch(sql)
                .unwrap();
        }
        let outcome = match import_legacy_delivery_state_files(
            &config,
            step["now"].as_f64().unwrap_or(100.25),
        ) {
            Ok(result) => {
                json!({"discovered_count":result.discovered_count,"imported_count":result.imported_count,"skipped_count":result.skipped_count,"item_count":result.item_count,"dedupe_count":result.dedupe_count})
            }
            Err(error) => json!({"error":error.to_string()}),
        };
        observations.push(json!({"outcome":outcome,"tables":snapshot(config.auth_db_path())}));
    }
    json!(observations)
}

fn main() {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", observe(&input));
    }
}
