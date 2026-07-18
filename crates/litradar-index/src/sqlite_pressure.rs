//! Release-mode pressure coverage for concurrent canonical SQLite writers.

use std::env;
use std::path::Path;
#[cfg(target_os = "windows")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_domain::{
    ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft, JournalRankings, ProviderBatch,
};
use rusqlite::{Connection, ErrorCode};
use serde::Serialize;

use crate::control::{open_control_db, write_checkpoint, CheckpointScope, ControlDatabaseError};
use crate::schema::{open_content_db, write_content_batch, ContentDatabaseError};

const REPORT_PATH_ENV: &str = "LITRADAR_SQLITE_PRESSURE_REPORT";
const FULL_WORKER_COUNT: usize = 3;
const FULL_PAGES_PER_WORKER: usize = 200;
const FULL_ARTICLES_PER_PAGE: usize = 225;
const EVENT_TIMEOUT: Duration = Duration::from_secs(65);

#[derive(Clone, Copy)]
struct PressureConfig {
    worker_count: usize,
    pages_per_worker: usize,
    articles_per_page: usize,
    injected_failure: Option<InjectedFailure>,
}

impl PressureConfig {
    fn full() -> Self {
        Self {
            worker_count: FULL_WORKER_COUNT,
            pages_per_worker: FULL_PAGES_PER_WORKER,
            articles_per_page: FULL_ARTICLES_PER_PAGE,
            injected_failure: None,
        }
    }

    fn synthetic_failure() -> Self {
        Self {
            worker_count: 3,
            pages_per_worker: 2,
            articles_per_page: 3,
            injected_failure: Some(InjectedFailure {
                worker_slot: 1,
                page_index: 0,
            }),
        }
    }

    fn expected_pages(self) -> usize {
        self.worker_count * self.pages_per_worker
    }

    fn expected_articles(self) -> usize {
        self.expected_pages() * self.articles_per_page
    }
}

#[derive(Clone, Copy)]
struct InjectedFailure {
    worker_slot: usize,
    page_index: usize,
}

#[derive(Debug, Serialize)]
struct PressureReport {
    report_version: u32,
    generated_at_unix_ms: u64,
    status: &'static str,
    scenario: ScenarioReport,
    progress: ProgressReport,
    commit_latency_ms: LatencyReport,
    peak_rss_bytes: Option<u64>,
    terminal_count: usize,
    first_failure: Option<FailureReport>,
    database_counts: DatabaseCounts,
    integrity: IntegrityReport,
}

#[derive(Debug, Serialize)]
struct ScenarioReport {
    worker_count: usize,
    pages_per_worker: usize,
    articles_per_page: usize,
    expected_pages: usize,
    expected_articles: usize,
    uses_canonical_content_writer: bool,
    writes_content_before_checkpoint: bool,
}

#[derive(Debug, Serialize)]
struct ProgressReport {
    finished_workers: usize,
    committed_pages: usize,
    committed_articles: usize,
    elapsed_ms: u64,
    throughput_articles_per_second: f64,
}

#[derive(Debug, Default, Serialize)]
struct LatencyReport {
    sample_count: usize,
    p50: Option<u64>,
    p95: Option<u64>,
    p99: Option<u64>,
    max: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct FailureReport {
    operation: &'static str,
    worker_slot: Option<usize>,
    page_index: Option<usize>,
    error_domain: &'static str,
    error_code: String,
    sqlite_extended_code: Option<i32>,
    is_busy_or_locked: bool,
}

#[derive(Debug, Default, Serialize)]
struct DatabaseCounts {
    journals: Option<i64>,
    issues: Option<i64>,
    articles: Option<i64>,
    article_listing: Option<i64>,
    article_search: Option<i64>,
    article_change_events: Option<i64>,
    provider_checkpoints: Option<i64>,
}

#[derive(Debug, Default, Serialize)]
struct IntegrityReport {
    content_integrity: Option<&'static str>,
    content_foreign_key_failures: Option<i64>,
    control_integrity: Option<&'static str>,
    control_foreign_key_failures: Option<i64>,
    row_alignment: bool,
    checkpoint_alignment: bool,
}

enum WorkerEvent {
    Committed { articles: usize, elapsed_ms: u64 },
    Failed(FailureReport),
    Finished,
}

#[test]
#[ignore = "release-only 3x200x225 canonical SQLite writer pressure"]
fn concurrent_canonical_writers_emit_attributable_report() {
    let report_path = env::var_os(REPORT_PATH_ENV)
        .map(std::path::PathBuf::from)
        .expect("pressure report path environment variable should be set");
    let report = run_pressure(PressureConfig::full());
    let write_result = write_report(&report_path, &report);
    assert!(write_result.is_ok(), "pressure report should write");
    assert_eq!(
        report.status, "passed",
        "canonical SQLite pressure failed; inspect the redacted report"
    );
}

#[test]
fn injected_worker_failure_always_emits_a_redacted_report() {
    let temp = tempfile::tempdir().expect("temporary report directory should create");
    let report_path = temp.path().join("synthetic-failure.json");
    let report = run_pressure(PressureConfig::synthetic_failure());
    write_report(&report_path, &report).expect("synthetic failure report should write");
    let report_text = std::fs::read_to_string(&report_path)
        .expect("synthetic failure report should remain readable");

    assert_eq!(report.status, "failed");
    assert_eq!(report.terminal_count, 1);
    let failure = report
        .first_failure
        .as_ref()
        .expect("synthetic failure should be captured");
    assert_eq!(failure.error_domain, "synthetic");
    assert_eq!(failure.error_code, "InjectedWorkerFailure");
    assert!(!failure.is_busy_or_locked);
    assert!(!report_text.contains(&temp.path().to_string_lossy().to_string()));
    assert!(!report_text.contains("10.9000/"));
    assert!(!report_text.contains("Pressure Article"));
}

fn run_pressure(config: PressureConfig) -> PressureReport {
    let started_at = Instant::now();
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(_) => {
            return failed_setup_report(
                config,
                started_at,
                safe_failure("create_temp_directory", "io", "Io"),
            );
        }
    };
    let content_path = temp.path().join("content.sqlite");
    let control_path = temp.path().join("control.sqlite");

    if let Err(error) = open_content_db(&content_path) {
        return failed_setup_report(
            config,
            started_at,
            classify_content_error("initialize_content", None, None, &error),
        );
    }
    if let Err(error) = open_control_db(&control_path) {
        return failed_setup_report(
            config,
            started_at,
            classify_control_error("initialize_control", None, None, &error),
        );
    }

    let mut connections = Vec::with_capacity(config.worker_count);
    for worker_slot in 0..config.worker_count {
        let content = match open_content_db(&content_path) {
            Ok(connection) => connection,
            Err(error) => {
                return failed_setup_report(
                    config,
                    started_at,
                    classify_content_error("open_content", Some(worker_slot), None, &error),
                );
            }
        };
        let control = match open_control_db(&control_path) {
            Ok(connection) => connection,
            Err(error) => {
                return failed_setup_report(
                    config,
                    started_at,
                    classify_control_error("open_control", Some(worker_slot), None, &error),
                );
            }
        };
        connections.push((worker_slot, content, control));
    }

    let barrier = Arc::new(Barrier::new(config.worker_count));
    let should_stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    let mut handles = Vec::with_capacity(config.worker_count);
    for (worker_slot, content, control) in connections {
        let worker_barrier = Arc::clone(&barrier);
        let worker_stop = Arc::clone(&should_stop);
        let worker_sender = sender.clone();
        handles.push(thread::spawn(move || {
            worker_barrier.wait();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_worker(
                    config,
                    worker_slot,
                    &content,
                    &control,
                    &worker_stop,
                    &worker_sender,
                );
            }));
            if result.is_err() {
                publish_failure(
                    &worker_stop,
                    &worker_sender,
                    safe_worker_failure("worker_panic", worker_slot, None, "ThreadPanic"),
                );
            }
            let _ = worker_sender.send(WorkerEvent::Finished);
        }));
    }
    drop(sender);

    let mut finished_workers = 0;
    let mut committed_pages = 0;
    let mut committed_articles = 0;
    let mut terminal_count = 0;
    let mut first_failure = None;
    let mut commit_latencies = Vec::with_capacity(config.expected_pages());
    while finished_workers < config.worker_count {
        match receiver.recv_timeout(EVENT_TIMEOUT) {
            Ok(WorkerEvent::Committed {
                articles,
                elapsed_ms,
            }) => {
                committed_pages += 1;
                committed_articles += articles;
                commit_latencies.push(elapsed_ms);
            }
            Ok(WorkerEvent::Failed(failure)) => {
                terminal_count += 1;
                if first_failure.is_none() {
                    first_failure = Some(failure);
                }
            }
            Ok(WorkerEvent::Finished) => finished_workers += 1,
            Err(RecvTimeoutError::Timeout) => {
                should_stop.store(true, Ordering::Release);
                terminal_count += 1;
                if first_failure.is_none() {
                    first_failure = Some(safe_failure(
                        "supervise_workers",
                        "supervisor",
                        "WorkerEventTimeout",
                    ));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if finished_workers < config.worker_count {
                    terminal_count += 1;
                    if first_failure.is_none() {
                        first_failure = Some(safe_failure(
                            "supervise_workers",
                            "supervisor",
                            "WorkerChannelDisconnected",
                        ));
                    }
                }
                break;
            }
        }
    }

    for handle in handles {
        if handle.join().is_err() {
            terminal_count += 1;
            if first_failure.is_none() {
                first_failure = Some(safe_failure(
                    "join_worker",
                    "supervisor",
                    "ThreadJoinFailure",
                ));
            }
        }
    }

    let inspection = inspect_databases(&content_path, &control_path, config);
    if let Some(failure) = inspection.failure {
        terminal_count += 1;
        if first_failure.is_none() {
            first_failure = Some(failure);
        }
    }
    let did_complete = terminal_count == 0
        && committed_pages == config.expected_pages()
        && committed_articles == config.expected_articles()
        && inspection.integrity.row_alignment
        && inspection.integrity.checkpoint_alignment;

    build_report(
        config,
        started_at,
        if did_complete { "passed" } else { "failed" },
        finished_workers,
        committed_pages,
        committed_articles,
        terminal_count,
        first_failure,
        commit_latencies,
        inspection.counts,
        inspection.integrity,
    )
}

fn run_worker(
    config: PressureConfig,
    worker_slot: usize,
    content: &Connection,
    control: &Connection,
    should_stop: &AtomicBool,
    sender: &Sender<WorkerEvent>,
) {
    let catalog = pressure_catalog(worker_slot);
    let scope = CheckpointScope::Journal {
        catalog_id: catalog.catalog_id.clone(),
    };
    for page_index in 0..config.pages_per_worker {
        if should_stop.load(Ordering::Acquire) {
            break;
        }
        if config.injected_failure.is_some_and(|failure| {
            failure.worker_slot == worker_slot && failure.page_index == page_index
        }) {
            publish_failure(
                should_stop,
                sender,
                FailureReport {
                    operation: "injected_worker_failure",
                    worker_slot: Some(worker_slot),
                    page_index: Some(page_index),
                    error_domain: "synthetic",
                    error_code: "InjectedWorkerFailure".to_string(),
                    sqlite_extended_code: None,
                    is_busy_or_locked: false,
                },
            );
            break;
        }

        let batch = pressure_batch(worker_slot, page_index, config.articles_per_page);
        let revision = format!("pressure-w{worker_slot}-p{page_index}");
        let checkpoint = format!("page-{page_index}");
        let commit_started_at = Instant::now();
        let outcome =
            match write_content_batch(content, &catalog, &batch, &revision, "2026-07-18T00:00:00Z")
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    publish_failure(
                        should_stop,
                        sender,
                        classify_content_error(
                            "write_content_batch",
                            Some(worker_slot),
                            Some(page_index),
                            &error,
                        ),
                    );
                    break;
                }
            };
        if let Err(error) = write_checkpoint(
            control,
            "pressure-catalog",
            "pressure-provider",
            &scope,
            &checkpoint,
            "2026-07-18T00:00:00Z",
        ) {
            publish_failure(
                should_stop,
                sender,
                classify_control_error(
                    "write_checkpoint",
                    Some(worker_slot),
                    Some(page_index),
                    &error,
                ),
            );
            break;
        }
        let _ = sender.send(WorkerEvent::Committed {
            articles: outcome.articles_seen,
            elapsed_ms: elapsed_millis(commit_started_at),
        });
    }
}

fn publish_failure(should_stop: &AtomicBool, sender: &Sender<WorkerEvent>, failure: FailureReport) {
    should_stop.store(true, Ordering::Release);
    let _ = sender.send(WorkerEvent::Failed(failure));
}

struct DatabaseInspection {
    counts: DatabaseCounts,
    integrity: IntegrityReport,
    failure: Option<FailureReport>,
}

fn inspect_databases(
    content_path: &Path,
    control_path: &Path,
    config: PressureConfig,
) -> DatabaseInspection {
    let mut counts = DatabaseCounts::default();
    let mut integrity = IntegrityReport::default();
    let mut failure = None;

    match Connection::open(content_path).and_then(|connection| {
        counts.journals = Some(table_count(&connection, "journals")?);
        counts.issues = Some(table_count(&connection, "issues")?);
        counts.articles = Some(table_count(&connection, "articles")?);
        counts.article_listing = Some(table_count(&connection, "article_listing")?);
        counts.article_search = Some(table_count(&connection, "article_search")?);
        counts.article_change_events = Some(table_count(&connection, "article_change_events")?);
        let integrity_value =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
        integrity.content_integrity = Some(if integrity_value == "ok" {
            "ok"
        } else {
            "failed"
        });
        integrity.content_foreign_key_failures = Some(connection.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_check",
            [],
            |row| row.get::<_, i64>(0),
        )?);
        Ok(())
    }) {
        Ok(()) => {}
        Err(error) => {
            failure = Some(classify_sqlite_error("inspect_content", None, None, &error));
        }
    }

    match Connection::open(control_path).and_then(|connection| {
        counts.provider_checkpoints = Some(table_count(&connection, "provider_checkpoints")?);
        let integrity_value =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
        integrity.control_integrity = Some(if integrity_value == "ok" {
            "ok"
        } else {
            "failed"
        });
        integrity.control_foreign_key_failures = Some(connection.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_check",
            [],
            |row| row.get::<_, i64>(0),
        )?);
        let final_checkpoint = format!("page-{}", config.pages_per_worker - 1);
        let aligned_checkpoint_count = connection.query_row(
            "SELECT COUNT(*) FROM provider_checkpoints
             WHERE catalog_name = 'pressure-catalog'
               AND provider_name = 'pressure-provider'
               AND scope_kind = 'journal'
               AND checkpoint = ?1",
            [&final_checkpoint],
            |row| row.get::<_, i64>(0),
        )?;
        integrity.checkpoint_alignment = aligned_checkpoint_count == config.worker_count as i64;
        Ok(())
    }) {
        Ok(()) => {}
        Err(error) => {
            if failure.is_none() {
                failure = Some(classify_sqlite_error("inspect_control", None, None, &error));
            }
        }
    }

    let expected_pages = config.expected_pages() as i64;
    let expected_articles = config.expected_articles() as i64;
    integrity.row_alignment = counts.journals == Some(config.worker_count as i64)
        && counts.issues == Some(expected_pages)
        && counts.articles == Some(expected_articles)
        && counts.article_listing == Some(expected_articles)
        && counts.article_search == Some(expected_articles)
        && counts.article_change_events == Some(expected_articles)
        && integrity.content_integrity == Some("ok")
        && integrity.content_foreign_key_failures == Some(0)
        && integrity.control_integrity == Some("ok")
        && integrity.control_foreign_key_failures == Some(0);

    DatabaseInspection {
        counts,
        integrity,
        failure,
    }
}

fn table_count(connection: &Connection, table_name: &str) -> rusqlite::Result<i64> {
    connection.query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
        row.get(0)
    })
}

fn pressure_catalog(worker_slot: usize) -> JournalCatalogEntry {
    JournalCatalogEntry {
        catalog_id: format!("pressure-journal-{worker_slot}"),
        title: format!("Pressure Journal {worker_slot}"),
        issn: None,
        eissn: None,
        all_issns: Vec::new(),
        title_aliases: Vec::new(),
        area: Some("Systems".to_string()),
        rankings: JournalRankings::default(),
    }
}

fn pressure_batch(
    worker_slot: usize,
    page_index: usize,
    articles_per_page: usize,
) -> ProviderBatch {
    let catalog = pressure_catalog(worker_slot);
    let issue_number = (page_index + 1).to_string();
    let articles = (0..articles_per_page)
        .map(|article_index| {
            let ordinal = page_index * articles_per_page + article_index + 1;
            ArticleDraft {
                catalog_id: catalog.catalog_id.clone(),
                title: format!("Pressure Article {worker_slot}-{ordinal}"),
                publication_year: Some(2026),
                date: None,
                issue_title: None,
                volume: Some((worker_slot + 1).to_string()),
                issue_number: Some(issue_number.clone()),
                authors: Vec::new(),
                start_page: Some(ordinal.to_string()),
                end_page: None,
                abstract_text: Some("Canonical SQLite pressure fixture".to_string()),
                doi: Some(format!(
                    "10.9000/w{worker_slot}.p{page_index}.a{article_index}"
                )),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_doi: None,
            }
        })
        .collect();

    ProviderBatch {
        catalog_id: catalog.catalog_id.clone(),
        journal: JournalDraft {
            catalog_id: catalog.catalog_id.clone(),
            observed_title: Some(catalog.title),
            observed_issns: Vec::new(),
            observed_title_aliases: Vec::new(),
        },
        issues: vec![IssueDraft {
            catalog_id: catalog.catalog_id.clone(),
            publication_year: Some(2026),
            title: None,
            volume: Some((worker_slot + 1).to_string()),
            number: Some(issue_number),
            date: None,
        }],
        articles,
        is_complete: false,
        next_checkpoint: Some(format!("page-{page_index}")),
    }
}

fn classify_content_error(
    operation: &'static str,
    worker_slot: Option<usize>,
    page_index: Option<usize>,
    error: &ContentDatabaseError,
) -> FailureReport {
    match error {
        ContentDatabaseError::Sqlite(error) => {
            classify_sqlite_error(operation, worker_slot, page_index, error)
        }
        ContentDatabaseError::Json(_) => {
            safe_scoped_failure(operation, worker_slot, page_index, "content", "Json")
        }
        ContentDatabaseError::Contract(_) => {
            safe_scoped_failure(operation, worker_slot, page_index, "content", "Contract")
        }
        ContentDatabaseError::Identity(_) => {
            safe_scoped_failure(operation, worker_slot, page_index, "content", "Identity")
        }
        ContentDatabaseError::Merge(_) => {
            safe_scoped_failure(operation, worker_slot, page_index, "content", "Merge")
        }
        ContentDatabaseError::RebuildRequired { .. } => safe_scoped_failure(
            operation,
            worker_slot,
            page_index,
            "content",
            "RebuildRequired",
        ),
        ContentDatabaseError::InvalidCurrentSchema(_) => safe_scoped_failure(
            operation,
            worker_slot,
            page_index,
            "content",
            "InvalidCurrentSchema",
        ),
        ContentDatabaseError::ArticleIdCollision { .. } => safe_scoped_failure(
            operation,
            worker_slot,
            page_index,
            "content",
            "ArticleIdCollision",
        ),
    }
}

fn classify_control_error(
    operation: &'static str,
    worker_slot: Option<usize>,
    page_index: Option<usize>,
    error: &ControlDatabaseError,
) -> FailureReport {
    match error {
        ControlDatabaseError::Sqlite(error) => {
            classify_sqlite_error(operation, worker_slot, page_index, error)
        }
        ControlDatabaseError::Io(_) => {
            safe_scoped_failure(operation, worker_slot, page_index, "control", "Io")
        }
        ControlDatabaseError::UnsupportedVersion { .. } => safe_scoped_failure(
            operation,
            worker_slot,
            page_index,
            "control",
            "UnsupportedVersion",
        ),
        ControlDatabaseError::ActiveLease { .. } => {
            safe_scoped_failure(operation, worker_slot, page_index, "control", "ActiveLease")
        }
        ControlDatabaseError::OwnershipLost { .. } => safe_scoped_failure(
            operation,
            worker_slot,
            page_index,
            "control",
            "OwnershipLost",
        ),
    }
}

fn classify_sqlite_error(
    operation: &'static str,
    worker_slot: Option<usize>,
    page_index: Option<usize>,
    error: &rusqlite::Error,
) -> FailureReport {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => FailureReport {
            operation,
            worker_slot,
            page_index,
            error_domain: "sqlite",
            error_code: format!("{:?}", failure.code),
            sqlite_extended_code: Some(failure.extended_code),
            is_busy_or_locked: matches!(
                failure.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ),
        },
        _ => safe_scoped_failure(
            operation,
            worker_slot,
            page_index,
            "rusqlite",
            "NonSqliteFailure",
        ),
    }
}

fn safe_failure(
    operation: &'static str,
    error_domain: &'static str,
    error_code: &'static str,
) -> FailureReport {
    safe_scoped_failure(operation, None, None, error_domain, error_code)
}

fn safe_worker_failure(
    operation: &'static str,
    worker_slot: usize,
    page_index: Option<usize>,
    error_code: &'static str,
) -> FailureReport {
    safe_scoped_failure(
        operation,
        Some(worker_slot),
        page_index,
        "worker",
        error_code,
    )
}

fn safe_scoped_failure(
    operation: &'static str,
    worker_slot: Option<usize>,
    page_index: Option<usize>,
    error_domain: &'static str,
    error_code: &'static str,
) -> FailureReport {
    FailureReport {
        operation,
        worker_slot,
        page_index,
        error_domain,
        error_code: error_code.to_string(),
        sqlite_extended_code: None,
        is_busy_or_locked: false,
    }
}

fn failed_setup_report(
    config: PressureConfig,
    started_at: Instant,
    failure: FailureReport,
) -> PressureReport {
    build_report(
        config,
        started_at,
        "failed",
        0,
        0,
        0,
        1,
        Some(failure),
        Vec::new(),
        DatabaseCounts::default(),
        IntegrityReport::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    config: PressureConfig,
    started_at: Instant,
    status: &'static str,
    finished_workers: usize,
    committed_pages: usize,
    committed_articles: usize,
    terminal_count: usize,
    first_failure: Option<FailureReport>,
    commit_latencies: Vec<u64>,
    database_counts: DatabaseCounts,
    integrity: IntegrityReport,
) -> PressureReport {
    let elapsed_ms = elapsed_millis(started_at);
    let elapsed_seconds = (elapsed_ms.max(1) as f64) / 1_000.0;
    PressureReport {
        report_version: 1,
        generated_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        status,
        scenario: ScenarioReport {
            worker_count: config.worker_count,
            pages_per_worker: config.pages_per_worker,
            articles_per_page: config.articles_per_page,
            expected_pages: config.expected_pages(),
            expected_articles: config.expected_articles(),
            uses_canonical_content_writer: true,
            writes_content_before_checkpoint: true,
        },
        progress: ProgressReport {
            finished_workers,
            committed_pages,
            committed_articles,
            elapsed_ms,
            throughput_articles_per_second: committed_articles as f64 / elapsed_seconds,
        },
        commit_latency_ms: latency_report(commit_latencies),
        peak_rss_bytes: peak_rss_bytes(),
        terminal_count,
        first_failure,
        database_counts,
        integrity,
    }
}

fn latency_report(mut samples: Vec<u64>) -> LatencyReport {
    if samples.is_empty() {
        return LatencyReport::default();
    }
    samples.sort_unstable();
    LatencyReport {
        sample_count: samples.len(),
        p50: percentile(&samples, 50),
        p95: percentile(&samples, 95),
        p99: percentile(&samples, 99),
        max: samples.last().copied(),
    }
}

fn percentile(samples: &[u64], percentile: usize) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let rank = (percentile * samples.len()).div_ceil(100);
    samples.get(rank.saturating_sub(1)).copied()
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn write_report(path: &Path, report: &PressureReport) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(report)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn peak_rss_bytes() -> Option<u64> {
    let command = format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id());
    let output = Command::new("powershell")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .arg(command)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

#[cfg(target_os = "linux")]
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1_024)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn peak_rss_bytes() -> Option<u64> {
    None
}
