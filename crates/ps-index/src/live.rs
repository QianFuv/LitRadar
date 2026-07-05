//! Live CSV index orchestration for the legacy `index` command.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ps_sources::{
    CnkiClient, CnkiSourceError, LiveCnkiConfig, LiveCnkiTransport, LiveScholarlyConfig,
    LiveScholarlyTransport, ScholarlyClient, SourceError,
};
use rusqlite::Connection;
use serde::Serialize;

use crate::cnki::{process_cnki_row, CnkiIndexConfig, CnkiIndexError};
use crate::manifest::{
    build_change_manifest_from_snapshots, collect_article_snapshot, write_change_manifest,
};
use crate::schema::{init_index_db, mark_journal_done, mark_year_done, persist_index_run_stats};
use crate::scholarly::{process_scholarly_row, ScholarlyIndexError};
use crate::stats::{IndexRunStats, PathCountIncrements};
use crate::transforms::{build_journal_id, journal_title_from_row, source_from_row, CsvRow};

const SCHOLARLY_SOURCE: &str = "scholarly";
const CNKI_SOURCE: &str = "cnki";

/// Live index run configuration.
#[derive(Debug, Clone)]
pub struct LiveIndexConfig {
    /// Project root containing the `data` directory.
    pub project_root: PathBuf,
    /// Optional CSV filename under `data/meta`.
    pub file: Option<String>,
    /// Number of issues processed together for CNKI.
    pub issue_batch_size: usize,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// Whether completed journals and years may be skipped.
    pub resume: bool,
    /// Whether to perform an update run and emit a change manifest.
    pub update: bool,
    /// Whether to run `notify` after an update manifest is written.
    pub notify: bool,
    /// Whether notify handoff should use dry-run mode.
    pub notify_dry_run: bool,
}

/// Live index command outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveIndexOutcome {
    /// Final run status.
    pub status: String,
    /// Human-readable message for skipped work.
    pub message: Option<String>,
    /// Per-CSV outcomes.
    pub csvs: Vec<LiveCsvIndexOutcome>,
}

/// Live index outcome for one CSV file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveCsvIndexOutcome {
    /// Source CSV path.
    pub csv_path: String,
    /// Output database path.
    pub db_path: String,
    /// Run identifier.
    pub run_id: String,
    /// Final run status.
    pub status: String,
    /// Indexed journal count.
    pub journal_count: usize,
    /// Written article ids.
    pub written_article_ids: Vec<i64>,
    /// Source attempt count.
    pub source_attempt_count: usize,
    /// Optional update manifest path.
    pub manifest_path: Option<String>,
    /// Optional notify process exit code.
    pub notify_exit_code: Option<i32>,
}

/// Live index workflow errors.
#[derive(Debug)]
pub enum LiveIndexError {
    /// IO operation failed.
    Io(std::io::Error),
    /// SQLite operation failed.
    Sqlite(rusqlite::Error),
    /// Scholarly source operation failed.
    Source(SourceError),
    /// CNKI source operation failed.
    CnkiSource(CnkiSourceError),
    /// Scholarly index row failed.
    Scholarly(ScholarlyIndexError),
    /// CNKI index row failed.
    Cnki(CnkiIndexError),
    /// A CSV row has an unsupported source.
    UnsupportedSource(String),
    /// Required runtime configuration is missing.
    MissingConfig(String),
    /// Notify handoff failed.
    Notify(String),
}

impl fmt::Display for LiveIndexError {
    /// Format the live index error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Source(error) => write!(formatter, "{error}"),
            Self::CnkiSource(error) => write!(formatter, "{error}"),
            Self::Scholarly(error) => write!(formatter, "{error}"),
            Self::Cnki(error) => write!(formatter, "{error}"),
            Self::UnsupportedSource(message) => formatter.write_str(message),
            Self::MissingConfig(message) => formatter.write_str(message),
            Self::Notify(message) => formatter.write_str(message),
        }
    }
}

impl Error for LiveIndexError {
    /// Return the underlying error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::Source(error) => Some(error),
            Self::CnkiSource(error) => Some(error),
            Self::Scholarly(error) => Some(error),
            Self::Cnki(error) => Some(error),
            Self::UnsupportedSource(_) | Self::MissingConfig(_) | Self::Notify(_) => None,
        }
    }
}

impl From<std::io::Error> for LiveIndexError {
    /// Convert IO errors into live index errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for LiveIndexError {
    /// Convert SQLite errors into live index errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<SourceError> for LiveIndexError {
    /// Convert source errors into live index errors.
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

impl From<CnkiSourceError> for LiveIndexError {
    /// Convert CNKI source errors into live index errors.
    fn from(error: CnkiSourceError) -> Self {
        Self::CnkiSource(error)
    }
}

impl From<ScholarlyIndexError> for LiveIndexError {
    /// Convert Scholarly row errors into live index errors.
    fn from(error: ScholarlyIndexError) -> Self {
        Self::Scholarly(error)
    }
}

impl From<CnkiIndexError> for LiveIndexError {
    /// Convert CNKI row errors into live index errors.
    fn from(error: CnkiIndexError) -> Self {
        Self::Cnki(error)
    }
}

/// Run live indexing for the legacy `index` command.
///
/// # Arguments
///
/// * `config` - Live index configuration.
///
/// # Returns
///
/// Live index outcome.
pub fn run_live_index(config: &LiveIndexConfig) -> Result<LiveIndexOutcome, LiveIndexError> {
    let meta_dir = config.project_root.join("data").join("meta");
    let index_dir = config.project_root.join("data").join("index");
    if !meta_dir.exists() {
        return Ok(LiveIndexOutcome {
            status: "skipped".to_string(),
            message: Some(format!("Directory not found: {}", meta_dir.display())),
            csvs: Vec::new(),
        });
    }
    fs::create_dir_all(&index_dir)?;
    let csv_paths = csv_paths(&meta_dir, config.file.as_deref())?;
    if csv_paths.is_empty() {
        return Ok(LiveIndexOutcome {
            status: "skipped".to_string(),
            message: Some(format!("No CSV files found in {}", meta_dir.display())),
            csvs: Vec::new(),
        });
    }

    let mut outcomes = Vec::new();
    for csv_path in csv_paths {
        let stem = csv_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("index");
        let db_path = index_dir.join(format!("{stem}.sqlite"));
        outcomes.push(run_live_csv_index(config, &csv_path, &db_path)?);
    }
    Ok(LiveIndexOutcome {
        status: "succeeded".to_string(),
        message: None,
        csvs: outcomes,
    })
}

fn run_live_csv_index(
    config: &LiveIndexConfig,
    csv_path: &Path,
    db_path: &Path,
) -> Result<LiveCsvIndexOutcome, LiveIndexError> {
    let rows = read_csv_rows(csv_path)?;
    if rows.is_empty() {
        return Ok(LiveCsvIndexOutcome {
            csv_path: csv_path.display().to_string(),
            db_path: db_path.display().to_string(),
            run_id: String::new(),
            status: "skipped".to_string(),
            journal_count: 0,
            written_article_ids: Vec::new(),
            source_attempt_count: 0,
            manifest_path: None,
            notify_exit_code: None,
        });
    }
    validate_sources(&rows)?;
    let scholarly_config = LiveScholarlyConfig::from_environment(config.timeout_seconds);
    validate_required_source_config(&rows, &scholarly_config)?;
    let timestamp = default_timestamp();
    let csv_file = csv_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("journals.csv")
        .to_string();
    let db_name = db_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("index.sqlite")
        .to_string();
    let run_id = format!(
        "{}-{timestamp}",
        csv_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("index")
    );

    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(db_path)?;
    init_index_db(&connection)?;
    let before_snapshot = if config.update {
        Some(collect_article_snapshot(&connection)?)
    } else {
        None
    };

    let mut scholarly_client = ScholarlyClient::new(
        LiveScholarlyTransport::new(scholarly_config.clone())?,
        scholarly_config.has_semantic_scholar_key(),
    );
    let mut cnki_client = CnkiClient::new(LiveCnkiTransport::new(LiveCnkiConfig {
        timeout_seconds: config.timeout_seconds,
    })?);
    let cnki_config = CnkiIndexConfig {
        csv_path: csv_path.to_path_buf(),
        fixture_path: PathBuf::new(),
        output_db_path: db_path.to_path_buf(),
        manifest_path: None,
        run_id: run_id.clone(),
        timestamp: timestamp.clone(),
        resume: config.resume,
        update: config.update,
        issue_batch_size: config.issue_batch_size.max(1),
    };
    let mut stats = IndexRunStats::new(run_id.clone(), csv_file.clone(), timestamp.clone());
    let mut all_written_articles = Vec::new();

    for row in &rows {
        let source = source_from_row(row);
        let journal_id = build_journal_id(row).ok_or_else(|| {
            LiveIndexError::UnsupportedSource(format!(
                "Journal row missing id: {}",
                journal_title_from_row(row)
            ))
        })?;
        let journal_title = journal_title_from_row(row);
        let path_key = stats.start_path(
            &source,
            "journal",
            Some(journal_id),
            journal_title.clone(),
            timestamp.clone(),
        );
        match source.as_str() {
            SCHOLARLY_SOURCE => {
                let attempt_start = scholarly_client.attempts().len();
                let result = process_scholarly_row(
                    &connection,
                    &mut scholarly_client,
                    row,
                    &csv_file,
                    journal_id,
                    &timestamp,
                );
                let attempts = scholarly_client.attempts()[attempt_start..].to_vec();
                stats.record_source_attempts(&attempts, Some(journal_id), &journal_title);
                match result {
                    Ok(outcome) => {
                        stats.record_path_counts(
                            &path_key,
                            PathCountIncrements {
                                works_count: outcome.works_count,
                                issues_count: outcome.issues_count,
                                articles_written_count: outcome.written_articles.len() as i64,
                                articles_deleted_no_authors_count: outcome.deleted_article_count,
                                ..PathCountIncrements::default()
                            },
                        );
                        for year in outcome.years {
                            mark_year_done(&connection, journal_id, year, &timestamp)?;
                        }
                        mark_journal_done(&connection, journal_id, &timestamp)?;
                        stats.finish_path(&path_key, "succeeded", timestamp.clone(), None);
                        all_written_articles.extend(outcome.written_articles);
                    }
                    Err(error) => {
                        stats.finish_path(
                            &path_key,
                            "failed",
                            timestamp.clone(),
                            Some(&error.to_string()),
                        );
                        stats.finish("failed", timestamp.clone(), Some(error.to_string()));
                        persist_index_run_stats(&connection, &stats)?;
                        return Err(error.into());
                    }
                }
            }
            CNKI_SOURCE => {
                let attempt_start = cnki_client.attempts().len();
                let result = process_cnki_row(
                    &connection,
                    &mut cnki_client,
                    row,
                    &csv_file,
                    journal_id,
                    &cnki_config,
                );
                let attempts = cnki_client.attempts()[attempt_start..].to_vec();
                stats.record_source_attempts_for_source(
                    CNKI_SOURCE,
                    &attempts,
                    Some(journal_id),
                    &journal_title,
                );
                match result {
                    Ok(outcome) => {
                        stats.record_path_counts(
                            &path_key,
                            PathCountIncrements {
                                issues_count: outcome.issues_count,
                                article_summaries_count: outcome.article_summaries_count,
                                article_details_count: outcome.article_details_count,
                                articles_written_count: outcome.written_articles.len() as i64,
                                articles_deleted_no_authors_count: outcome.deleted_article_count,
                                ..PathCountIncrements::default()
                            },
                        );
                        stats.finish_path(&path_key, &outcome.status, timestamp.clone(), None);
                        all_written_articles.extend(outcome.written_articles);
                    }
                    Err(error) => {
                        stats.finish_path(
                            &path_key,
                            "failed",
                            timestamp.clone(),
                            Some(&error.to_string()),
                        );
                        stats.finish("failed", timestamp.clone(), Some(error.to_string()));
                        persist_index_run_stats(&connection, &stats)?;
                        return Err(error.into());
                    }
                }
            }
            other => {
                return Err(LiveIndexError::UnsupportedSource(format!(
                    "Unsupported source for {}: {other}",
                    journal_title_from_row(row)
                )));
            }
        }
    }

    stats.finish("succeeded", timestamp.clone(), None);
    persist_index_run_stats(&connection, &stats)?;
    all_written_articles.sort_by_key(|article| article.article_id);
    let mut manifest_path = None;
    if let Some(before_snapshot) = before_snapshot {
        let after_snapshot = collect_article_snapshot(&connection)?;
        let path = config
            .project_root
            .join("data")
            .join("push_state")
            .join(format!(
                "{}.changes.json",
                db_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("index")
            ));
        let manifest = build_change_manifest_from_snapshots(
            &db_name,
            db_path,
            &run_id,
            &timestamp,
            &before_snapshot,
            &after_snapshot,
        );
        write_change_manifest(&manifest, &path)?;
        manifest_path = Some(path);
    }
    let notify_exit_code = if config.notify {
        let Some(path) = manifest_path.as_ref() else {
            return Err(LiveIndexError::Notify(
                "--notify requires an update manifest".to_string(),
            ));
        };
        Some(run_notify_for_manifest(config, &db_name, path)?)
    } else {
        None
    };

    Ok(LiveCsvIndexOutcome {
        csv_path: csv_path.display().to_string(),
        db_path: db_path.display().to_string(),
        run_id,
        status: "succeeded".to_string(),
        journal_count: rows.len(),
        written_article_ids: all_written_articles
            .iter()
            .map(|article| article.article_id)
            .collect(),
        source_attempt_count: scholarly_client.attempts().len() + cnki_client.attempts().len(),
        manifest_path: manifest_path.map(|path| path.display().to_string()),
        notify_exit_code,
    })
}

fn csv_paths(meta_dir: &Path, file: Option<&str>) -> Result<Vec<PathBuf>, LiveIndexError> {
    if let Some(file) = file {
        let path = meta_dir.join(file);
        return if path.exists() {
            Ok(vec![path])
        } else {
            Ok(Vec::new())
        };
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(meta_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("csv") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_csv_rows(path: &Path) -> Result<Vec<CsvRow>, LiveIndexError> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return Ok(Vec::new());
    };
    let headers = parse_csv_line(header_line);
    let mut rows = Vec::new();
    for line in lines {
        let values = parse_csv_line(line);
        let mut row = CsvRow::new();
        for (index, header) in headers.iter().enumerate() {
            row.insert(
                header.clone(),
                values.get(index).cloned().unwrap_or_default(),
            );
        }
        let source = row
            .get("source")
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| SCHOLARLY_SOURCE.to_string());
        row.insert("source".to_string(), source);
        rows.push(row);
    }
    Ok(rows)
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut characters = line.chars().peekable();
    let mut inside_quotes = false;
    while let Some(character) = characters.next() {
        match character {
            '"' if inside_quotes && characters.peek() == Some(&'"') => {
                current.push('"');
                characters.next();
            }
            '"' => inside_quotes = !inside_quotes,
            ',' if !inside_quotes => {
                values.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    values.push(current.trim().to_string());
    values
}

fn validate_sources(rows: &[CsvRow]) -> Result<(), LiveIndexError> {
    for row in rows {
        let source = source_from_row(row);
        if source != SCHOLARLY_SOURCE && source != CNKI_SOURCE {
            return Err(LiveIndexError::UnsupportedSource(format!(
                "Unsupported source for {}: {source}",
                journal_title_from_row(row)
            )));
        }
    }
    Ok(())
}

fn validate_required_source_config(
    rows: &[CsvRow],
    config: &LiveScholarlyConfig,
) -> Result<(), LiveIndexError> {
    let has_scholarly_rows = rows
        .iter()
        .any(|row| source_from_row(row) == SCHOLARLY_SOURCE);
    if !has_scholarly_rows {
        return Ok(());
    }
    if config.openalex_api_keys.is_empty() {
        return Err(LiveIndexError::MissingConfig(
            "OpenAlex API key is required for scholarly indexing.".to_string(),
        ));
    }
    if !config.has_semantic_scholar_key() {
        return Err(LiveIndexError::MissingConfig(
            "Semantic Scholar API key is required for scholarly indexing.".to_string(),
        ));
    }
    Ok(())
}

fn run_notify_for_manifest(
    config: &LiveIndexConfig,
    db_name: &str,
    manifest_path: &Path,
) -> Result<i32, LiveIndexError> {
    run_notify_command_for_manifest(Path::new("notify"), config, db_name, manifest_path)
}

fn run_notify_command_for_manifest(
    command_path: &Path,
    config: &LiveIndexConfig,
    db_name: &str,
    manifest_path: &Path,
) -> Result<i32, LiveIndexError> {
    let state_dir = config.project_root.join("data").join("push_state");
    let mut command = Command::new(command_path);
    command
        .arg("--db")
        .arg(db_name)
        .arg("--changes-file")
        .arg(manifest_path)
        .arg("--state-dir")
        .arg(&state_dir)
        .env("PAPER_SCANNER_PROJECT_ROOT", &config.project_root);
    if config.notify_dry_run {
        command.arg("--dry-run");
    }
    let status = command
        .status()
        .map_err(|error| LiveIndexError::Notify(error.to_string()))?;
    Ok(status.code().unwrap_or(1))
}

fn default_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use ps_sources::LiveScholarlyConfig;
    use tempfile::tempdir;

    use super::{
        csv_paths, parse_csv_line, read_csv_rows, run_live_index, run_notify_command_for_manifest,
        validate_required_source_config, validate_sources, LiveIndexConfig, LiveIndexError,
    };
    use crate::transforms::CsvRow;

    #[test]
    fn csv_parser_handles_quotes() {
        assert_eq!(
            parse_csv_line(r#"source,title,issn"#),
            vec!["source", "title", "issn"]
        );
        assert_eq!(
            parse_csv_line(r#"scholarly,"A, B",1234-5678"#),
            vec!["scholarly", "A, B", "1234-5678"]
        );
    }

    #[test]
    fn source_validation_rejects_unknown_values() {
        let row = CsvRow::from([
            ("source".to_string(), "unknown".to_string()),
            ("title".to_string(), "Bad Source".to_string()),
        ]);

        assert!(validate_sources(&[row]).is_err());
    }

    #[test]
    fn csv_path_discovery_sorts_csvs_and_respects_explicit_file() {
        let root = tempdir().expect("temp root should be created");
        fs::write(root.path().join("b.csv"), "source,title\n").expect("b csv should be written");
        fs::write(root.path().join("notes.txt"), "ignored").expect("text file should be written");
        fs::write(root.path().join("a.csv"), "source,title\n").expect("a csv should be written");

        assert_eq!(
            csv_file_names(&csv_paths(root.path(), None).expect("csvs should be listed")),
            vec!["a.csv", "b.csv"]
        );
        assert_eq!(
            csv_file_names(&csv_paths(root.path(), Some("b.csv")).expect("csv should be selected")),
            vec!["b.csv"]
        );
        assert!(csv_paths(root.path(), Some("missing.csv"))
            .expect("missing explicit csv should not fail")
            .is_empty());
    }

    #[test]
    fn live_index_skips_missing_or_empty_meta_inputs() {
        let root = tempdir().expect("temp root should be created");
        let missing_meta = run_live_index(&live_config(root.path()))
            .expect("missing meta dir should return a skipped outcome");

        assert_eq!(missing_meta.status, "skipped");
        assert!(missing_meta
            .message
            .as_deref()
            .expect("missing meta should explain skip")
            .contains("Directory not found"));

        fs::create_dir_all(root.path().join("data").join("meta"))
            .expect("meta dir should be created");
        let empty_meta =
            run_live_index(&live_config(root.path())).expect("empty meta dir should skip");

        assert_eq!(empty_meta.status, "skipped");
        assert!(empty_meta
            .message
            .as_deref()
            .expect("empty meta should explain skip")
            .contains("No CSV files"));
    }

    #[test]
    fn live_index_reports_empty_csv_without_network_transports() {
        let root = tempdir().expect("temp root should be created");
        let meta_dir = root.path().join("data").join("meta");
        fs::create_dir_all(&meta_dir).expect("meta dir should be created");
        fs::write(meta_dir.join("journals.csv"), "source,title,issn\n")
            .expect("empty csv should be written");

        let outcome = run_live_index(&live_config(root.path()))
            .expect("empty csv should not construct live transports");

        assert_eq!(outcome.status, "succeeded");
        assert_eq!(outcome.csvs.len(), 1);
        assert_eq!(outcome.csvs[0].status, "skipped");
        assert!(outcome.csvs[0].written_article_ids.is_empty());
    }

    #[test]
    fn live_index_rejects_unsupported_source_before_live_transports() {
        let root = tempdir().expect("temp root should be created");
        let meta_dir = root.path().join("data").join("meta");
        fs::create_dir_all(&meta_dir).expect("meta dir should be created");
        fs::write(
            meta_dir.join("selected.csv"),
            "source,title,issn\nunknown,Bad Source,1234-5678\n",
        )
        .expect("csv should be written");

        let error = run_live_index(&LiveIndexConfig {
            file: Some("selected.csv".to_string()),
            ..live_config(root.path())
        })
        .expect_err("unsupported source should fail before transports");

        assert!(matches!(
            error,
            LiveIndexError::UnsupportedSource(message) if message.contains("Bad Source")
        ));
    }

    #[test]
    fn csv_reader_defaults_source_and_validates_required_scholarly_config() {
        let root = tempdir().expect("temp root should be created");
        let csv_path = root.path().join("journals.csv");
        fs::write(&csv_path, "title,issn\nJournal,1234-5678\n").expect("csv should be written");

        let rows = read_csv_rows(&csv_path).expect("csv should parse");
        let missing_config = validate_required_source_config(
            &rows,
            &LiveScholarlyConfig {
                timeout_seconds: 1,
                openalex_api_keys: Vec::new(),
                semantic_scholar_api_keys: Vec::new(),
                crossref_mailtos: Vec::new(),
            },
        )
        .expect_err("scholarly rows should require API configuration");

        assert_eq!(rows[0].get("source").map(String::as_str), Some("scholarly"));
        assert!(missing_config.to_string().contains("OpenAlex API key"));

        let semantic_missing = validate_required_source_config(
            &rows,
            &LiveScholarlyConfig {
                timeout_seconds: 1,
                openalex_api_keys: vec!["openalex".to_string()],
                semantic_scholar_api_keys: Vec::new(),
                crossref_mailtos: Vec::new(),
            },
        )
        .expect_err("scholarly rows should require Semantic Scholar configuration");
        assert!(semantic_missing
            .to_string()
            .contains("Semantic Scholar API key"));

        let cnki_only = CsvRow::from([
            ("source".to_string(), "cnki".to_string()),
            ("title".to_string(), "CNKI".to_string()),
        ]);
        validate_required_source_config(
            &[cnki_only],
            &LiveScholarlyConfig {
                timeout_seconds: 1,
                openalex_api_keys: Vec::new(),
                semantic_scholar_api_keys: Vec::new(),
                crossref_mailtos: Vec::new(),
            },
        )
        .expect("CNKI-only rows should not require scholarly configuration");
    }

    #[test]
    fn notify_command_helper_reports_exit_code_and_arguments() {
        let root = tempdir().expect("temp root should be created");
        let manifest_path = root
            .path()
            .join("data")
            .join("push_state")
            .join("fixture.changes.json");
        fs::create_dir_all(manifest_path.parent().expect("manifest should have parent"))
            .expect("manifest dir should be created");
        fs::write(&manifest_path, "{}").expect("manifest should be written");
        let command_path = write_notify_command(root.path());

        let exit_code = run_notify_command_for_manifest(
            &command_path,
            &live_config(root.path()),
            "fixture.sqlite",
            &manifest_path,
        )
        .expect("notify command should run");

        let args =
            fs::read_to_string(root.path().join("args.txt")).expect("args should be captured");
        assert_eq!(exit_code, 7);
        assert!(args.contains("--db"));
        assert!(args.contains("fixture.sqlite"));
        assert!(args.contains("--changes-file"));
        assert!(args.contains("fixture.changes.json"));
        assert!(args.contains("--state-dir"));
        assert!(args.contains("push_state"));
        assert!(args.contains("--dry-run"));
    }

    #[test]
    fn notify_command_helper_maps_spawn_failures() {
        let root = tempdir().expect("temp root should be created");
        let manifest_path = root.path().join("missing.changes.json");

        let error = run_notify_command_for_manifest(
            &root.path().join("missing-notify"),
            &live_config(root.path()),
            "fixture.sqlite",
            &manifest_path,
        )
        .expect_err("missing notify command should fail");

        assert!(matches!(error, LiveIndexError::Notify(message) if !message.is_empty()));
    }

    fn csv_file_names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .expect("csv path should have a UTF-8 filename")
                    .to_string()
            })
            .collect()
    }

    #[cfg(windows)]
    fn write_notify_command(root: &Path) -> PathBuf {
        let path = root.join("notify.cmd");
        fs::write(
            &path,
            "@echo off\r\necho %* > \"%~dp0args.txt\"\r\nexit /b 7\r\n",
        )
        .expect("notify command should be written");
        path
    }

    #[cfg(not(windows))]
    fn write_notify_command(root: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = root.join("notify");
        fs::write(
            &path,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$(dirname \"$0\")/args.txt\"\nexit 7\n",
        )
        .expect("notify command should be written");
        let mut permissions = fs::metadata(&path)
            .expect("notify command metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("notify command should be executable");
        path
    }

    fn live_config(root: &Path) -> LiveIndexConfig {
        LiveIndexConfig {
            project_root: root.to_path_buf(),
            file: None,
            issue_batch_size: 10,
            timeout_seconds: 1,
            resume: false,
            update: false,
            notify: false,
            notify_dry_run: true,
        }
    }
}
