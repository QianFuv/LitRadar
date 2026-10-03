//! Exercise original backup serialization, creation, verification and restore for interoperability.
use litradar_storage::*;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn observe(request: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let root = request["root"].as_str().unwrap_or(".");
    let config = StorageConfig::from_project_root(root);
    Ok(match request["operation"].as_str().unwrap() {
        "parse" => match serde_json::from_str::<BackupManifest>(request["input"].as_str().unwrap())
        {
            Ok(manifest) => json!({"valid":true,"manifest":manifest}),
            Err(_) => json!({"valid":false,"manifest":null}),
        },
        "create" => json!(create_backup(&BackupCreateOptions {
            auth_db_path: config.auth_db_path().to_path_buf(),
            storage_config: config,
            output_dir: request["output"].as_str().unwrap().into(),
            include_index_databases: true,
            include_push_state: true
        })?),
        "verify" => json!(verify_backup(root)?),
        "restore" => json!(restore_backup(&BackupRestoreOptions {
            auth_db_path: config.auth_db_path().to_path_buf(),
            storage_config: config,
            backup_dir: request["backup"].as_str().unwrap().into()
        })?),
        _ => panic!("unknown operation"),
    })
}

fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        match observe(&request) {
            Ok(value) => {
                request["output"] = value;
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
