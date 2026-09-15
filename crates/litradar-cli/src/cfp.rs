//! Operational import and bounded refresh entrypoints for backend-owned CFP data.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::time::Duration;

use litradar_index::transforms::read_catalog_csv;
use litradar_sources::cfp::{cfp_source_registry, CfpSourceConfig, CFP_SEED, CFP_SEED_ID};
use litradar_storage::business::cfp::{import_prepared_cfp_seed, prepare_cfp_seed};
use litradar_storage::{migrate_auth_database, StorageConfig};
use litradar_worker::cfp::{
    ensure_cfp_seed, refresh_cfp_full_texts, refresh_cfp_sources, CfpRefreshOptions,
};
use serde_json::{json, Value};

use crate::{
    extract_path_option, extract_project_root, extract_string_option, has_help, print_help,
    print_result, remove_flag, run_cli_command,
};

fn usage() -> String {
    json!({"usage":["litradar cfp import --project-root PATH --input FILE","litradar cfp refresh --project-root PATH (--db NAME | --catalog-id ID | --all) [--full-text] [--capture-dir PATH] [--resume-captures] [--obscura-path PATH] [--pdftotext-path PATH] [--source-timeout SECONDS] [--timeout SECONDS]"],"defaults":{"source_timeout":90,"timeout":600,"concurrency":2},"fullText":"Recapture stored notices and follow original-title detail links, including snapshot-only sources. Unresolved notices retain their previous originals.","import":"Additive, validated, immutable seed import. Existing source ownership and online refresh data are preserved."}).to_string()
}

/// Run backend CFP operations, printing structured outcomes and failing on incomplete refreshes.
pub fn run_cfp_command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    run_cli_command("cfp", move || {
        if has_help(&args) {
            print_help(&usage());
            return Ok(());
        }
        let result = run_command(args)?;
        print_result(&serde_json::to_string(&result)?);
        if result.get("failed").and_then(Value::as_u64).unwrap_or(0) > 0
            || result
                .get("notAttempted")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
        {
            return Err("CFP refresh completed with failed or unattempted sources; last-good data was retained".into());
        }
        Ok(())
    })
}

fn run_command(mut args: Vec<String>) -> Result<Value, Box<dyn Error>> {
    let project_root = extract_project_root(&mut args)?;
    let config = StorageConfig::from_project_root(project_root);
    let Some(command) = args.first().cloned() else {
        return Err(usage().into());
    };
    args.remove(0);
    match command.as_str() {
        "import" => {
            let input = extract_path_option(&mut args, "--input")?.ok_or_else(usage)?;
            if !args.is_empty() {
                return Err(format!("Unexpected CFP import arguments: {}", args.join(" ")).into());
            }
            if fs::metadata(&input)?.len() > 16 * 1024 * 1024 {
                return Err("CFP input exceeds 16 MiB".into());
            }
            let bytes = fs::read(input)?;
            let prepared = prepare_cfp_seed(&bytes)?;
            migrate_auth_database(config.auth_db_path())?;
            let identity = if bytes == CFP_SEED {
                CFP_SEED_ID.to_owned()
            } else {
                ensure_cfp_seed(config.auth_db_path())?;
                format!("operator:{}", prepared.content_hash())
            };
            Ok(serde_json::to_value(import_prepared_cfp_seed(
                config.auth_db_path(),
                &identity,
                &prepared,
            )?)?)
        }
        "refresh" => {
            let database = extract_string_option(&mut args, "--db")?;
            let catalog_id = extract_string_option(&mut args, "--catalog-id")?;
            let is_all = remove_flag(&mut args, "--all");
            let is_full_text = remove_flag(&mut args, "--full-text");
            let should_resume_captures = remove_flag(&mut args, "--resume-captures");
            let capture_directory = extract_path_option(&mut args, "--capture-dir")?;
            let mut options = CfpRefreshOptions::default();
            if let Some(path) = extract_path_option(&mut args, "--obscura-path")? {
                options.obscura_path = Some(path);
            }
            if let Some(path) = extract_path_option(&mut args, "--pdftotext-path")? {
                options.pdftotext_path = Some(path);
            }
            if let Some(value) = extract_string_option(&mut args, "--source-timeout")? {
                options.source_timeout = Duration::from_secs(value.parse()?);
            }
            if let Some(value) = extract_string_option(&mut args, "--timeout")? {
                options.overall_timeout = Duration::from_secs(value.parse()?);
            }
            if !args.is_empty()
                || (capture_directory.is_some() && !is_full_text)
                || (should_resume_captures && (!is_full_text || capture_directory.is_none()))
                || usize::from(database.is_some())
                    + usize::from(catalog_id.is_some())
                    + usize::from(is_all)
                    != 1
            {
                return Err(usage().into());
            }
            if options.source_timeout.is_zero()
                || options.source_timeout > Duration::from_secs(600)
                || options.overall_timeout.is_zero()
                || options.overall_timeout > Duration::from_secs(3600)
            {
                return Err(
                    "CFP source timeout must be 1..600 seconds and batch timeout 1..3600 seconds"
                        .into(),
                );
            }
            let sources =
                select_sources(&config, database.as_deref(), catalog_id.as_deref(), is_all)?;
            migrate_auth_database(config.auth_db_path())?;
            if is_full_text {
                let results = refresh_cfp_full_texts(
                    config.auth_db_path(),
                    &sources,
                    &options,
                    capture_directory.as_deref(),
                    should_resume_captures,
                )?;
                return Ok(
                    json!({"success":results.iter().filter(|result|result.status=="success").count(),"partial":results.iter().filter(|result|result.status=="partial").count(),"failed":results.iter().filter(|result|matches!(result.status.as_str(),"failed"|"partial")).count(),"noNotices":results.iter().filter(|result|result.status=="no_notices").count(),"notAttempted":results.iter().filter(|result|result.status=="not_attempted").count(),"recoveredNotices":results.iter().map(|result|result.recovered).sum::<usize>(),"updatedNotices":results.iter().map(|result|result.updated).sum::<usize>(),"sources":results}),
                );
            }
            let results = refresh_cfp_sources(config.auth_db_path(), &sources, &options)?;
            Ok(
                json!({"success":results.iter().filter(|result|result.status=="success").count(),"failed":results.iter().filter(|result|result.status=="failed").count(),"unsupported":results.iter().filter(|result|result.status=="unsupported").count(),"notAttempted":results.iter().filter(|result|result.status=="not_attempted").count(),"sources":results}),
            )
        }
        _ => Err(usage().into()),
    }
}

fn select_sources(
    config: &StorageConfig,
    database: Option<&str>,
    catalog_id: Option<&str>,
    is_all: bool,
) -> Result<Vec<CfpSourceConfig>, Box<dyn Error>> {
    if is_all {
        return Ok(cfp_source_registry().to_vec());
    }
    let mut identities = BTreeSet::new();
    let mut has_database = database.is_none();
    for catalog in config.list_provider_catalogs()? {
        if database.is_some_and(|database| database != format!("{}.sqlite", catalog.stem)) {
            continue;
        }
        let Some(filename) = catalog.csv_filename else {
            continue;
        };
        has_database = true;
        for journal in read_catalog_csv(config.meta_dir().join(filename))? {
            if catalog_id.is_some_and(|id| {
                id != journal.catalog_id && !journal.catalog_aliases.iter().any(|alias| alias == id)
            }) {
                continue;
            }
            identities.insert(journal.catalog_id);
            identities.extend(journal.catalog_aliases);
        }
    }
    if !has_database {
        return Err("CFP database catalog not found".into());
    }
    if identities.is_empty() {
        return Err("CFP journal catalog member not found".into());
    }
    let sources: Vec<_> = cfp_source_registry()
        .iter()
        .filter(|source| source.catalog_ids.iter().any(|id| identities.contains(id)))
        .cloned()
        .collect();
    if sources.is_empty() && catalog_id.is_some() {
        return Err("CFP journal is not yet adapted".into());
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfp_rejects_invalid_scope_and_budgets_before_creating_a_database() {
        let directory = tempfile::tempdir().unwrap();
        for tail in [
            vec!["refresh"],
            vec!["refresh", "--all", "--catalog-id", "issn-fixture"],
            vec!["refresh", "--all", "--source-timeout", "0"],
            vec!["refresh", "--all", "--timeout", "3601"],
            vec!["refresh", "--all", "--unexpected"],
        ] {
            let mut args = tail.into_iter().map(str::to_owned).collect::<Vec<_>>();
            args.extend([
                "--project-root".into(),
                directory.path().to_string_lossy().into_owned(),
            ]);
            assert!(run_command(args).is_err());
            assert!(!directory.path().join("data/auth.sqlite").exists());
        }
    }

    #[test]
    fn cfp_registry_and_help_keep_acquisition_backend_owned() {
        let registry = cfp_source_registry();
        assert_eq!(registry.len(), 467);
        assert_eq!(
            registry
                .iter()
                .filter(|source| source.can_refresh())
                .count(),
            106
        );
        assert!(usage().contains("--obscura-path"));
        assert!(usage().contains("--input"));
    }
}
