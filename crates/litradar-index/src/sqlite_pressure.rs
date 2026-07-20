//! Release-mode pressure coverage for process-real fetch producers and one SQLite writer.

use std::env;
use std::io::{self, BufReader, BufWriter, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_domain::{
    ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft, JournalRankings, ProviderBatch,
};
use rusqlite::{Connection, ErrorCode};
use serde::Serialize;

use crate::control::{
    acquire_lease, open_control_db, release_lease, ContentCheckpointCommitError,
    ControlDatabaseError,
};
use crate::live::{
    run_worker_processes_with_launcher, LaunchedWorkerProcess, LiveIndexError, ParentWriterContext,
    WriterCommitObservation,
};
use crate::schema::{open_content_db, ContentDatabaseError};
use crate::stats::IndexRunMetrics;
use crate::worker_protocol::{
    read_message, write_message, ParentMessage, WorkerFailure, WorkerFailureClass,
    WorkerJournalAssignment, WorkerMessage, WorkerOperation, WorkerRequest, PROTOCOL_VERSION,
};

const REPORT_PATH_ENV: &str = "LITRADAR_SQLITE_PRESSURE_REPORT";
const CHILD_ROLE_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_CHILD_ROLE";
const CHILD_REQUEST_PATH_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_REQUEST_PATH";
const CHILD_ENDPOINT_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_ENDPOINT";
const CHILD_PAGES_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_PAGES";
const CHILD_ARTICLES_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_ARTICLES";
const CHILD_BEHAVIOR_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_BEHAVIOR";
const CHILD_TARGET_WORKER_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_TARGET_WORKER";
const CHILD_TARGET_PAGE_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_TARGET_PAGE";
const CHILD_START_DELAY_MS_ENV: &str = "LITRADAR_SINGLE_WRITER_PRESSURE_START_DELAY_MS";
const CHILD_ROLE: &str = "producer";
const CHILD_TEST_NAME: &str = "sqlite_pressure::single_writer_process_fixture";
const FULL_WORKER_COUNT: usize = 3;
const FULL_PAGES_PER_WORKER: usize = 200;
const FULL_ARTICLES_PER_PAGE: usize = 225;
const CHILD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROCESS_COMPLETION_BOUND: Duration = Duration::from_secs(2_400);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FixtureBehavior {
    Normal,
    FailedMessage,
    MalformedMessage,
    PrematureEof,
    NonzeroAfterSuccess,
    AckWriteFailure,
    OutOfOrderBatch,
}

impl FixtureBehavior {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::FailedMessage => "failed_message",
            Self::MalformedMessage => "malformed_message",
            Self::PrematureEof => "premature_eof",
            Self::NonzeroAfterSuccess => "nonzero_after_success",
            Self::AckWriteFailure => "ack_write_failure",
            Self::OutOfOrderBatch => "out_of_order_batch",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "failed_message" => Some(Self::FailedMessage),
            "malformed_message" => Some(Self::MalformedMessage),
            "premature_eof" => Some(Self::PrematureEof),
            "nonzero_after_success" => Some(Self::NonzeroAfterSuccess),
            "ack_write_failure" => Some(Self::AckWriteFailure),
            "out_of_order_batch" => Some(Self::OutOfOrderBatch),
            _ => None,
        }
    }

    fn failure_code(self) -> &'static str {
        match self {
            Self::Normal => "UnexpectedPressureFailure",
            Self::FailedMessage => "InjectedWorkerFailure",
            Self::MalformedMessage => "MalformedWorkerMessage",
            Self::PrematureEof => "PrematureWorkerEof",
            Self::NonzeroAfterSuccess => "NonzeroWorkerExit",
            Self::AckWriteFailure => "AckWriteFailure",
            Self::OutOfOrderBatch => "OutOfOrderBatch",
        }
    }
}

#[derive(Clone, Copy)]
struct PressureConfig {
    worker_count: usize,
    pages_per_worker: usize,
    articles_per_page: usize,
    behavior: FixtureBehavior,
    target_worker: usize,
    target_page: usize,
    heartbeat_interval: Duration,
    producer_start_delay: Duration,
}

impl PressureConfig {
    fn full() -> Self {
        Self {
            worker_count: FULL_WORKER_COUNT,
            pages_per_worker: FULL_PAGES_PER_WORKER,
            articles_per_page: FULL_ARTICLES_PER_PAGE,
            behavior: FixtureBehavior::Normal,
            target_worker: usize::MAX,
            target_page: usize::MAX,
            heartbeat_interval: Duration::from_secs(1),
            producer_start_delay: Duration::ZERO,
        }
    }

    fn smoke() -> Self {
        Self {
            worker_count: 3,
            pages_per_worker: 3,
            articles_per_page: 4,
            behavior: FixtureBehavior::Normal,
            target_worker: usize::MAX,
            target_page: usize::MAX,
            heartbeat_interval: Duration::from_millis(10),
            producer_start_delay: Duration::from_millis(100),
        }
    }

    fn failure(behavior: FixtureBehavior) -> Self {
        Self {
            worker_count: 3,
            pages_per_worker: 2,
            articles_per_page: 3,
            behavior,
            target_worker: 0,
            target_page: 0,
            heartbeat_interval: Duration::from_millis(10),
            producer_start_delay: Duration::from_millis(50),
        }
    }

    fn ack_failure() -> Self {
        Self {
            worker_count: 1,
            pages_per_worker: 1,
            articles_per_page: 1,
            behavior: FixtureBehavior::AckWriteFailure,
            target_worker: 0,
            target_page: 0,
            heartbeat_interval: Duration::from_millis(10),
            producer_start_delay: Duration::from_millis(50),
        }
    }

    fn expected_pages(self) -> usize {
        self.worker_count * self.pages_per_worker
    }

    fn expected_articles(self) -> usize {
        self.expected_pages() * self.articles_per_page
    }
}

#[derive(Debug, Serialize)]
struct PressureReport {
    report_version: u32,
    generated_at_unix_ms: u64,
    status: &'static str,
    scenario: ScenarioReport,
    progress: ProgressReport,
    writer_service_ms: LatencyReport,
    peak_rss_bytes: Option<u64>,
    terminal_count: usize,
    first_failure: Option<FailureReport>,
    database_counts: DatabaseCounts,
    integrity: IntegrityReport,
    lifecycle: LifecycleReport,
}

#[derive(Debug, Serialize)]
struct ScenarioReport {
    producer_count: usize,
    pages_per_producer: usize,
    articles_per_page: usize,
    expected_pages: usize,
    expected_articles: usize,
    process_real_producers: bool,
    acknowledged_ipc: bool,
    single_parent_sqlite_writer: bool,
    sqlite_writer_count: usize,
    max_unacknowledged_batches_per_producer: usize,
    uses_canonical_content_writer: bool,
    writes_content_before_checkpoint: bool,
}

#[derive(Debug, Serialize)]
struct ProgressReport {
    finished_producers: usize,
    committed_pages: usize,
    committed_articles: usize,
    observed_worker_ids: Vec<usize>,
    attribution_complete: bool,
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
    worker_id: Option<usize>,
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

#[derive(Debug, Default, Serialize)]
struct LifecycleReport {
    request_files_removed: bool,
    process_supervision_returned: bool,
    completed_within_bound: bool,
    lease_heartbeat_observed: bool,
}

struct DatabaseInspection {
    counts: DatabaseCounts,
    integrity: IntegrityReport,
    failure: Option<FailureReport>,
}

struct FailingAcknowledgementWriter;

impl Write for FailingAcknowledgementWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "synthetic acknowledgement failure",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "synthetic acknowledgement failure",
        ))
    }
}

#[test]
#[ignore = "release-only 3x200x225 process-real single-writer pressure"]
fn single_writer_ingestion_emits_attributable_report() {
    let report_path = env::var_os(REPORT_PATH_ENV)
        .map(PathBuf::from)
        .expect("pressure report path environment variable should be set");
    let report = run_pressure(PressureConfig::full());
    write_report(&report_path, &report).expect("pressure report should write");
    assert_eq!(
        report.status, "passed",
        "single-writer pressure failed; inspect the redacted report"
    );
}

#[test]
fn single_writer_smoke_reports_acknowledged_process_topology() {
    let config = PressureConfig::smoke();
    let report = run_pressure(config);

    assert_eq!(report.status, "passed", "{report:#?}");
    assert_eq!(report.report_version, 3);
    assert_eq!(report.terminal_count, 0);
    assert!(report.first_failure.is_none());
    assert!(report.scenario.process_real_producers);
    assert!(report.scenario.acknowledged_ipc);
    assert!(report.scenario.single_parent_sqlite_writer);
    assert_eq!(report.scenario.sqlite_writer_count, 1);
    assert_eq!(report.scenario.max_unacknowledged_batches_per_producer, 1);
    assert_eq!(report.progress.committed_pages, config.expected_pages());
    assert_eq!(
        report.progress.committed_articles,
        config.expected_articles()
    );
    assert_eq!(
        report.writer_service_ms.sample_count,
        config.expected_pages()
    );
    assert!(report.progress.attribution_complete);
    assert!(report.integrity.row_alignment);
    assert!(report.integrity.checkpoint_alignment);
    assert!(report.lifecycle.request_files_removed);
    assert!(report.lifecycle.process_supervision_returned);
    assert!(report.lifecycle.completed_within_bound);
    assert!(report.lifecycle.lease_heartbeat_observed);
}

#[test]
fn single_writer_injected_failure_emits_a_redacted_report() {
    let directory = tempfile::tempdir().expect("temporary report directory should create");
    let report_path = directory.path().join("synthetic-failure.json");
    let report = run_pressure(PressureConfig::failure(FixtureBehavior::FailedMessage));
    write_report(&report_path, &report).expect("synthetic failure report should write");
    let report_text = std::fs::read_to_string(&report_path)
        .expect("synthetic failure report should remain readable");

    assert_eq!(report.status, "failed");
    assert_eq!(report.terminal_count, 1);
    assert_eq!(report.report_version, 3);
    assert!(report.lifecycle.request_files_removed);
    let failure = report
        .first_failure
        .as_ref()
        .expect("synthetic failure should be captured");
    assert_eq!(failure.error_domain, "synthetic");
    assert_eq!(failure.error_code, "InjectedWorkerFailure");
    assert!(!failure.is_busy_or_locked);
    assert!(!report_text.contains(&directory.path().to_string_lossy().to_string()));
    assert!(!report_text.contains("10.9000/"));
    assert!(!report_text.contains("Pressure Article"));
}

#[test]
fn single_writer_process_lifecycle_fails_closed_and_removes_requests() {
    for behavior in [
        FixtureBehavior::MalformedMessage,
        FixtureBehavior::PrematureEof,
        FixtureBehavior::NonzeroAfterSuccess,
        FixtureBehavior::OutOfOrderBatch,
    ] {
        let report = run_pressure(PressureConfig::failure(behavior));
        assert_eq!(report.status, "failed");
        assert_eq!(report.terminal_count, 1);
        assert_eq!(
            report
                .first_failure
                .as_ref()
                .expect("fixture failure should be retained")
                .error_code,
            behavior.failure_code()
        );
        assert!(report.lifecycle.request_files_removed);
        assert!(report.lifecycle.process_supervision_returned);
        assert!(report.lifecycle.completed_within_bound);
        if behavior == FixtureBehavior::OutOfOrderBatch {
            assert_eq!(report.database_counts.articles, Some(0));
            assert_eq!(report.database_counts.provider_checkpoints, Some(0));
        }
    }
}

#[test]
fn single_writer_ack_failure_retains_durable_content_and_checkpoint() {
    let report = run_pressure(PressureConfig::ack_failure());

    assert_eq!(report.status, "failed");
    assert_eq!(report.terminal_count, 1);
    assert_eq!(
        report
            .first_failure
            .as_ref()
            .expect("acknowledgement failure should be retained")
            .error_code,
        "AckWriteFailure"
    );
    assert_eq!(report.writer_service_ms.sample_count, 0);
    assert_eq!(report.database_counts.articles, Some(1));
    assert_eq!(report.database_counts.provider_checkpoints, Some(1));
    assert!(report.integrity.row_alignment);
    assert!(report.integrity.checkpoint_alignment);
    assert!(report.lifecycle.request_files_removed);
}

#[test]
fn single_writer_process_fixture() {
    if env::var(CHILD_ROLE_ENV).as_deref() != Ok(CHILD_ROLE) {
        return;
    }
    run_process_fixture().expect("process-real pressure fixture should complete");
}

fn run_pressure(config: PressureConfig) -> PressureReport {
    let started_at = Instant::now();
    let directory = match tempfile::tempdir() {
        Ok(directory) => directory,
        Err(_) => {
            return failed_setup_report(
                config,
                started_at,
                safe_failure("create_temp_directory", "io", "Io"),
            );
        }
    };
    let content_path = directory.path().join("content.sqlite");
    let control_path = directory.path().join("control.sqlite");
    let request_dir = directory.path().join("worker-requests");
    let content = match open_content_db(&content_path) {
        Ok(content) => content,
        Err(error) => {
            return failed_setup_report(
                config,
                started_at,
                classify_content_error("open_content", &error),
            );
        }
    };
    let control = match open_control_db(&control_path) {
        Ok(control) => control,
        Err(error) => {
            return failed_setup_report(
                config,
                started_at,
                classify_control_error("open_control", &error),
            );
        }
    };
    let run_id = format!("pressure-{}", unix_millis());
    let context = ParentWriterContext {
        catalog_name: "pressure-catalog".to_string(),
        provider_name: "pressure-provider".to_string(),
        run_id: run_id.clone(),
        timestamp: "2026-07-19T00:00:00Z".to_string(),
    };
    let acquired_at = unix_seconds().saturating_sub(10);
    if let Err(error) = acquire_lease(
        &control,
        &context.catalog_name,
        &context.provider_name,
        &context.run_id,
        acquired_at,
    ) {
        return failed_setup_report(
            config,
            started_at,
            classify_control_error("acquire_lease", &error),
        );
    }
    let requests = pressure_requests(config, &context);
    let mut observations = Vec::with_capacity(config.expected_pages());
    let execution = run_worker_processes_with_launcher(
        &request_dir,
        &content,
        &control,
        &context,
        requests,
        IndexRunMetrics {
            journals_total: config.worker_count,
            ..IndexRunMetrics::default()
        },
        config.heartbeat_interval,
        |request_path, worker_id| launch_process_fixture(config, request_path, worker_id),
        |observation| observations.push(observation),
    );
    let heartbeat_at = control
        .query_row(
            "SELECT heartbeat_at FROM provider_leases
             WHERE catalog_name = ?1 AND provider_name = ?2 AND run_id = ?3",
            [
                context.catalog_name.as_str(),
                context.provider_name.as_str(),
                context.run_id.as_str(),
            ],
            |row| row.get::<_, i64>(0),
        )
        .ok();
    let release = release_lease(
        &control,
        &context.catalog_name,
        &context.provider_name,
        &context.run_id,
    );
    let request_files_removed = request_dir_is_empty(&request_dir);
    let inspection = inspect_databases(&content, &control, config);
    let metrics = execution.as_ref().ok();
    let mut first_failure = execution
        .as_ref()
        .err()
        .map(|error| classify_execution_error(config, error));
    if first_failure.is_none() {
        first_failure = release
            .as_ref()
            .err()
            .map(|error| classify_control_error("release_lease", error));
    }
    if first_failure.is_none() {
        first_failure = inspection.failure.clone();
    }
    let terminal_count = usize::from(first_failure.is_some());
    let committed_pages = observations.len();
    let committed_articles = observations
        .iter()
        .map(|observation| observation.articles_seen)
        .sum::<usize>();
    let observed_worker_ids = observed_worker_ids(&observations);
    let attribution_complete = observations_are_complete(config, &observations);
    let finished_producers = metrics.map_or(0, |metrics| metrics.journals_succeeded);
    let elapsed_ms = elapsed_millis(started_at);
    let elapsed_seconds = started_at.elapsed().as_secs_f64();
    let throughput_articles_per_second = if elapsed_seconds > 0.0 {
        committed_articles as f64 / elapsed_seconds
    } else {
        0.0
    };
    let lifecycle = LifecycleReport {
        request_files_removed,
        process_supervision_returned: true,
        completed_within_bound: started_at.elapsed() <= PROCESS_COMPLETION_BOUND,
        lease_heartbeat_observed: heartbeat_at.is_some_and(|value| value > acquired_at),
    };
    let did_complete = execution.is_ok()
        && release.is_ok()
        && terminal_count == 0
        && finished_producers == config.worker_count
        && committed_pages == config.expected_pages()
        && committed_articles == config.expected_articles()
        && observations.len() == config.expected_pages()
        && attribution_complete
        && inspection.integrity.row_alignment
        && inspection.integrity.checkpoint_alignment
        && lifecycle.request_files_removed
        && lifecycle.completed_within_bound
        && lifecycle.lease_heartbeat_observed;

    PressureReport {
        report_version: 3,
        generated_at_unix_ms: unix_millis(),
        status: if did_complete { "passed" } else { "failed" },
        scenario: scenario_report(config),
        progress: ProgressReport {
            finished_producers,
            committed_pages,
            committed_articles,
            observed_worker_ids,
            attribution_complete,
            elapsed_ms,
            throughput_articles_per_second,
        },
        writer_service_ms: latency_report(
            observations
                .iter()
                .map(|observation| observation.service_ms)
                .collect(),
        ),
        peak_rss_bytes: peak_rss_bytes(),
        terminal_count,
        first_failure,
        database_counts: inspection.counts,
        integrity: inspection.integrity,
        lifecycle,
    }
}

fn pressure_requests(config: PressureConfig, context: &ParentWriterContext) -> Vec<WorkerRequest> {
    (0..config.worker_count)
        .map(|worker_id| WorkerRequest {
            protocol_version: PROTOCOL_VERSION,
            catalog_name: context.catalog_name.clone(),
            provider_name: context.provider_name.clone(),
            run_id: context.run_id.clone(),
            worker_id,
            process_count: config.worker_count,
            source_worker_count: 1,
            schedule_epoch_unix_millis: 0,
            timeout_seconds: 10,
            scholarly_config: litradar_sources::LiveScholarlyConfig::from_value_pools(
                10, "", "", "",
            ),
            assignments: vec![WorkerJournalAssignment {
                journal_ordinal: worker_id,
                entry: pressure_catalog(worker_id),
                initial_checkpoint: None,
            }],
        })
        .collect()
}

fn launch_process_fixture(
    config: PressureConfig,
    request_path: &Path,
    worker_id: usize,
) -> Result<LaunchedWorkerProcess, LiveIndexError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| LiveIndexError::Worker("pressure worker listener failed".to_string()))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| LiveIndexError::Worker("pressure worker listener failed".to_string()))?;
    let endpoint = listener
        .local_addr()
        .map_err(|_| LiveIndexError::Worker("pressure worker listener failed".to_string()))?;
    let executable = env::current_exe()
        .map_err(|_| LiveIndexError::Worker("pressure worker executable failed".to_string()))?;
    let mut child = Command::new(executable)
        .arg("--exact")
        .arg(CHILD_TEST_NAME)
        .arg("--quiet")
        .env(CHILD_ROLE_ENV, CHILD_ROLE)
        .env(CHILD_REQUEST_PATH_ENV, request_path)
        .env(CHILD_ENDPOINT_ENV, endpoint.to_string())
        .env(CHILD_PAGES_ENV, config.pages_per_worker.to_string())
        .env(CHILD_ARTICLES_ENV, config.articles_per_page.to_string())
        .env(CHILD_BEHAVIOR_ENV, config.behavior.as_str())
        .env(CHILD_TARGET_WORKER_ENV, config.target_worker.to_string())
        .env(CHILD_TARGET_PAGE_ENV, config.target_page.to_string())
        .env(
            CHILD_START_DELAY_MS_ENV,
            duration_millis(config.producer_start_delay).to_string(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            LiveIndexError::Worker(format!("pressure worker {worker_id} could not start"))
        })?;
    let stream = match accept_fixture_stream(&listener, &mut child) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    stream
        .set_nodelay(true)
        .map_err(|_| LiveIndexError::Worker("pressure worker stream failed".to_string()))?;
    let reader = stream
        .try_clone()
        .map_err(|_| LiveIndexError::Worker("pressure worker stream failed".to_string()))?;
    if config.behavior == FixtureBehavior::AckWriteFailure && worker_id == config.target_worker {
        return Ok(LaunchedWorkerProcess::from_test_streams(
            child,
            reader,
            FailingAcknowledgementWriter,
        ));
    }
    Ok(LaunchedWorkerProcess::from_test_streams(
        child, reader, stream,
    ))
}

fn accept_fixture_stream(
    listener: &TcpListener,
    child: &mut Child,
) -> Result<TcpStream, LiveIndexError> {
    let deadline = Instant::now() + CHILD_CONNECT_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).map_err(|_| {
                    LiveIndexError::Worker("pressure worker stream failed".to_string())
                })?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                return Err(LiveIndexError::Worker(
                    "pressure worker connection failed".to_string(),
                ));
            }
        }
        if child
            .try_wait()
            .map_err(|_| LiveIndexError::Worker("pressure worker wait failed".to_string()))?
            .is_some()
        {
            return Err(LiveIndexError::Worker(
                "pressure worker exited before connecting".to_string(),
            ));
        }
        if Instant::now() >= deadline {
            return Err(LiveIndexError::Worker(
                "pressure worker connection timed out".to_string(),
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn run_process_fixture() -> Result<(), &'static str> {
    let request_path = required_path(CHILD_REQUEST_PATH_ENV)?;
    let request_bytes = std::fs::read(request_path).map_err(|_| "request read failed")?;
    let request: WorkerRequest =
        serde_json::from_slice(&request_bytes).map_err(|_| "request decode failed")?;
    let endpoint = required_string(CHILD_ENDPOINT_ENV)?
        .parse::<SocketAddr>()
        .map_err(|_| "endpoint decode failed")?;
    let pages_per_worker = required_usize(CHILD_PAGES_ENV)?;
    let articles_per_page = required_usize(CHILD_ARTICLES_ENV)?;
    let behavior = FixtureBehavior::parse(&required_string(CHILD_BEHAVIOR_ENV)?)
        .ok_or("behavior decode failed")?;
    let target_worker = required_usize(CHILD_TARGET_WORKER_ENV)?;
    let target_page = required_usize(CHILD_TARGET_PAGE_ENV)?;
    let start_delay = Duration::from_millis(
        required_string(CHILD_START_DELAY_MS_ENV)?
            .parse::<u64>()
            .map_err(|_| "delay decode failed")?,
    );
    let stream = TcpStream::connect(endpoint).map_err(|_| "fixture connection failed")?;
    stream
        .set_nodelay(true)
        .map_err(|_| "fixture stream setup failed")?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|_| "fixture reader clone failed")?,
    );
    let mut writer = BufWriter::new(stream);
    if !start_delay.is_zero() {
        thread::sleep(start_delay);
    }
    let assignment = request
        .assignments
        .first()
        .ok_or("fixture assignment missing")?;
    let mut sequence = 0_u64;
    for page_index in 0..pages_per_worker {
        if request.worker_id == target_worker && page_index == target_page {
            match behavior {
                FixtureBehavior::FailedMessage => {
                    write_message(
                        &mut writer,
                        &WorkerMessage::Failed {
                            protocol_version: PROTOCOL_VERSION,
                            worker_id: request.worker_id,
                            sequence,
                            failure: WorkerFailure::fixed(
                                WorkerFailureClass::Provider,
                                WorkerOperation::ProviderRequest,
                            ),
                        },
                    )
                    .map_err(|_| "failure message write failed")?;
                    return Ok(());
                }
                FixtureBehavior::MalformedMessage => {
                    writer
                        .write_all(b"{malformed-worker-message\n")
                        .map_err(|_| "malformed message write failed")?;
                    writer.flush().map_err(|_| "malformed flush failed")?;
                    return Ok(());
                }
                FixtureBehavior::PrematureEof => return Ok(()),
                FixtureBehavior::Normal
                | FixtureBehavior::NonzeroAfterSuccess
                | FixtureBehavior::AckWriteFailure
                | FixtureBehavior::OutOfOrderBatch => {}
            }
        }
        let batch = pressure_batch(
            request.worker_id,
            page_index,
            pages_per_worker,
            articles_per_page,
        );
        let is_complete = batch.is_complete;
        let emitted_sequence = if behavior == FixtureBehavior::OutOfOrderBatch
            && request.worker_id == target_worker
            && page_index == target_page
        {
            sequence.saturating_add(1)
        } else {
            sequence
        };
        write_message(
            &mut writer,
            &WorkerMessage::Batch {
                protocol_version: PROTOCOL_VERSION,
                worker_id: request.worker_id,
                sequence: emitted_sequence,
                journal_ordinal: assignment.journal_ordinal,
                page_index,
                batch,
            },
        )
        .map_err(|_| "batch write failed")?;
        let acknowledgement: ParentMessage =
            read_message(&mut reader).map_err(|_| "acknowledgement read failed")?;
        match acknowledgement {
            ParentMessage::Committed {
                protocol_version,
                worker_id,
                sequence: acknowledged_sequence,
                journal_ordinal,
                page_index: acknowledged_page_index,
                is_complete: acknowledged_complete,
            } if protocol_version == PROTOCOL_VERSION
                && worker_id == request.worker_id
                && acknowledged_sequence == sequence
                && journal_ordinal == assignment.journal_ordinal
                && acknowledged_page_index == page_index
                && acknowledged_complete == is_complete => {}
            ParentMessage::Committed { .. } => return Err("acknowledgement mismatch"),
        }
        sequence = sequence.checked_add(1).ok_or("fixture sequence overflow")?;
    }
    write_message(
        &mut writer,
        &WorkerMessage::Succeeded {
            protocol_version: PROTOCOL_VERSION,
            worker_id: request.worker_id,
            sequence,
        },
    )
    .map_err(|_| "success message write failed")?;
    if behavior == FixtureBehavior::NonzeroAfterSuccess && request.worker_id == target_worker {
        std::process::exit(17);
    }
    Ok(())
}

fn pressure_catalog(worker_id: usize) -> JournalCatalogEntry {
    JournalCatalogEntry {
        catalog_id: format!("pressure-journal-{worker_id}"),
        catalog_aliases: Vec::new(),
        title: format!("Pressure Journal {worker_id}"),
        issn: None,
        eissn: None,
        all_issns: Vec::new(),
        title_aliases: Vec::new(),
        area: Some("Systems".to_string()),
        rankings: JournalRankings::default(),
    }
}

fn pressure_batch(
    worker_id: usize,
    page_index: usize,
    pages_per_worker: usize,
    articles_per_page: usize,
) -> ProviderBatch {
    let catalog = pressure_catalog(worker_id);
    let issue_number = (page_index + 1).to_string();
    let articles = (0..articles_per_page)
        .map(|article_index| {
            let ordinal = page_index * articles_per_page + article_index + 1;
            ArticleDraft {
                catalog_id: catalog.catalog_id.clone(),
                title: format!("Pressure Article {worker_id}-{ordinal}"),
                publication_year: Some(2026),
                date: None,
                issue_title: None,
                volume: Some((worker_id + 1).to_string()),
                issue_number: Some(issue_number.clone()),
                authors: Vec::new(),
                start_page: Some(ordinal.to_string()),
                end_page: None,
                abstract_text: Some("Canonical SQLite pressure fixture".to_string()),
                doi: Some(format!(
                    "10.9000/w{worker_id}.p{page_index}.a{article_index}"
                )),
                pmid: None,
                open_access: Some(true),
                in_press: Some(false),
                retraction_doi: None,
            }
        })
        .collect();
    let is_complete = page_index + 1 == pages_per_worker;

    ProviderBatch {
        catalog_id: catalog.catalog_id.clone(),
        journal: JournalDraft {
            catalog_id: catalog.catalog_id.clone(),
            observed_title: Some(catalog.title),
            observed_issns: Vec::new(),
            observed_title_aliases: Vec::new(),
        },
        issues: vec![IssueDraft {
            catalog_id: catalog.catalog_id,
            publication_year: Some(2026),
            title: None,
            volume: Some((worker_id + 1).to_string()),
            number: Some(issue_number),
            date: None,
        }],
        articles,
        is_complete,
        next_checkpoint: (!is_complete).then(|| format!("page-{}", page_index + 1)),
    }
}

fn inspect_databases(
    content: &Connection,
    control: &Connection,
    config: PressureConfig,
) -> DatabaseInspection {
    let mut counts = DatabaseCounts::default();
    let mut integrity = IntegrityReport::default();
    let mut failure = None;
    let content_result = (|| -> rusqlite::Result<()> {
        counts.journals = Some(table_count(content, "journals")?);
        counts.issues = Some(table_count(content, "issues")?);
        counts.articles = Some(table_count(content, "articles")?);
        counts.article_listing = Some(table_count(content, "article_listing")?);
        counts.article_search = Some(table_count(content, "article_search")?);
        counts.article_change_events = Some(table_count(content, "article_change_events")?);
        let integrity_value =
            content.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
        integrity.content_integrity = Some(if integrity_value == "ok" {
            "ok"
        } else {
            "failed"
        });
        integrity.content_foreign_key_failures = Some(content.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_check",
            [],
            |row| row.get::<_, i64>(0),
        )?);
        Ok(())
    })();
    if let Err(error) = content_result {
        failure = Some(classify_sqlite_error("inspect_content", &error));
    }
    let control_result = (|| -> rusqlite::Result<()> {
        counts.provider_checkpoints = Some(table_count(control, "provider_checkpoints")?);
        let integrity_value =
            control.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
        integrity.control_integrity = Some(if integrity_value == "ok" {
            "ok"
        } else {
            "failed"
        });
        integrity.control_foreign_key_failures = Some(control.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_check",
            [],
            |row| row.get::<_, i64>(0),
        )?);
        let completed = (0..config.worker_count)
            .map(|worker_id| {
                let catalog_id = format!("pressure-journal-{worker_id}");
                control
                    .query_row(
                        "SELECT checkpoint FROM provider_checkpoints
                         WHERE catalog_name = 'pressure-catalog'
                           AND provider_name = 'pressure-provider'
                           AND scope_kind = 'journal'
                           AND scope_key = ?1",
                        [&catalog_id],
                        |row| row.get::<_, String>(0),
                    )
                    .map(|checkpoint| {
                        serde_json::from_str::<serde_json::Value>(&checkpoint)
                            .ok()
                            .is_some_and(|value| value["state"] == "complete")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        integrity.checkpoint_alignment = completed.into_iter().all(|value| value);
        Ok(())
    })();
    if let Err(error) = control_result {
        if failure.is_none() {
            failure = Some(classify_sqlite_error("inspect_control", &error));
        }
    }
    let expected_pages = i64::try_from(config.expected_pages()).unwrap_or(i64::MAX);
    let expected_articles = i64::try_from(config.expected_articles()).unwrap_or(i64::MAX);
    let expected_workers = i64::try_from(config.worker_count).unwrap_or(i64::MAX);
    integrity.row_alignment = counts.journals == Some(expected_workers)
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

fn observations_are_complete(
    config: PressureConfig,
    observations: &[WriterCommitObservation],
) -> bool {
    (0..config.worker_count).all(|worker_id| {
        let worker = observations
            .iter()
            .filter(|observation| observation.worker_id == worker_id)
            .collect::<Vec<_>>();
        worker.len() == config.pages_per_worker
            && worker.iter().enumerate().all(|(index, observation)| {
                observation.sequence == index as u64 && observation.page_index == index
            })
    })
}

fn observed_worker_ids(observations: &[WriterCommitObservation]) -> Vec<usize> {
    let mut worker_ids = observations
        .iter()
        .map(|observation| observation.worker_id)
        .collect::<Vec<_>>();
    worker_ids.sort_unstable();
    worker_ids.dedup();
    worker_ids
}

fn table_count(connection: &Connection, table_name: &str) -> rusqlite::Result<i64> {
    connection.query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
        row.get(0)
    })
}

fn classify_execution_error(config: PressureConfig, error: &LiveIndexError) -> FailureReport {
    if config.behavior != FixtureBehavior::Normal {
        return FailureReport {
            operation: "supervise_worker_processes",
            worker_id: Some(config.target_worker),
            page_index: Some(config.target_page),
            error_domain: "synthetic",
            error_code: config.behavior.failure_code().to_string(),
            sqlite_extended_code: None,
            is_busy_or_locked: false,
        };
    }
    match error {
        LiveIndexError::Commit(ContentCheckpointCommitError::Content(error))
        | LiveIndexError::ContentDatabase { source: error, .. } => {
            classify_content_error("parent_content_commit", error)
        }
        LiveIndexError::Commit(ContentCheckpointCommitError::Control(error))
        | LiveIndexError::Control(error) => {
            classify_control_error("parent_checkpoint_commit", error)
        }
        LiveIndexError::Heartbeat(_) => {
            safe_failure("heartbeat_lease", "control", "HeartbeatFailure")
        }
        LiveIndexError::Worker(_) => {
            safe_failure("supervise_worker_processes", "worker", "WorkerFailure")
        }
        _ => safe_failure("run_pressure", "index", "LiveIndexFailure"),
    }
}

fn classify_content_error(operation: &'static str, error: &ContentDatabaseError) -> FailureReport {
    match error {
        ContentDatabaseError::Sqlite(error) => classify_sqlite_error(operation, error),
        ContentDatabaseError::Json(_) => safe_failure(operation, "content", "Json"),
        ContentDatabaseError::Contract(_) => safe_failure(operation, "content", "Contract"),
        ContentDatabaseError::Identity(_) => safe_failure(operation, "content", "Identity"),
        ContentDatabaseError::Merge(_) => safe_failure(operation, "content", "Merge"),
        ContentDatabaseError::RebuildRequired { .. } => {
            safe_failure(operation, "content", "RebuildRequired")
        }
        ContentDatabaseError::InvalidCurrentSchema(_) => {
            safe_failure(operation, "content", "InvalidCurrentSchema")
        }
        ContentDatabaseError::ArticleIdCollision { .. } => {
            safe_failure(operation, "content", "ArticleIdCollision")
        }
    }
}

fn classify_control_error(operation: &'static str, error: &ControlDatabaseError) -> FailureReport {
    match error {
        ControlDatabaseError::Sqlite(error) => classify_sqlite_error(operation, error),
        ControlDatabaseError::Io(_) => safe_failure(operation, "control", "Io"),
        ControlDatabaseError::UnsupportedVersion { .. } => {
            safe_failure(operation, "control", "UnsupportedVersion")
        }
        ControlDatabaseError::ActiveLease { .. } => {
            safe_failure(operation, "control", "ActiveLease")
        }
        ControlDatabaseError::OwnershipLost { .. } => {
            safe_failure(operation, "control", "OwnershipLost")
        }
    }
}

fn classify_sqlite_error(operation: &'static str, error: &rusqlite::Error) -> FailureReport {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => FailureReport {
            operation,
            worker_id: None,
            page_index: None,
            error_domain: "sqlite",
            error_code: format!("{:?}", failure.code),
            sqlite_extended_code: Some(failure.extended_code),
            is_busy_or_locked: matches!(
                failure.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ),
        },
        _ => safe_failure(operation, "sqlite", "Other"),
    }
}

fn safe_failure(
    operation: &'static str,
    error_domain: &'static str,
    error_code: &'static str,
) -> FailureReport {
    FailureReport {
        operation,
        worker_id: None,
        page_index: None,
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
    PressureReport {
        report_version: 3,
        generated_at_unix_ms: unix_millis(),
        status: "failed",
        scenario: scenario_report(config),
        progress: ProgressReport {
            finished_producers: 0,
            committed_pages: 0,
            committed_articles: 0,
            observed_worker_ids: Vec::new(),
            attribution_complete: false,
            elapsed_ms: elapsed_millis(started_at),
            throughput_articles_per_second: 0.0,
        },
        writer_service_ms: LatencyReport::default(),
        peak_rss_bytes: peak_rss_bytes(),
        terminal_count: 1,
        first_failure: Some(failure),
        database_counts: DatabaseCounts::default(),
        integrity: IntegrityReport::default(),
        lifecycle: LifecycleReport::default(),
    }
}

fn scenario_report(config: PressureConfig) -> ScenarioReport {
    ScenarioReport {
        producer_count: config.worker_count,
        pages_per_producer: config.pages_per_worker,
        articles_per_page: config.articles_per_page,
        expected_pages: config.expected_pages(),
        expected_articles: config.expected_articles(),
        process_real_producers: true,
        acknowledged_ipc: true,
        single_parent_sqlite_writer: true,
        sqlite_writer_count: 1,
        max_unacknowledged_batches_per_producer: 1,
        uses_canonical_content_writer: true,
        writes_content_before_checkpoint: true,
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

fn request_dir_is_empty(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    std::fs::read_dir(path)
        .ok()
        .is_some_and(|mut entries| entries.next().is_none())
}

fn required_path(name: &str) -> Result<PathBuf, &'static str> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or("required path is missing")
}

fn required_string(name: &str) -> Result<String, &'static str> {
    env::var(name).map_err(|_| "required value is missing")
}

fn required_usize(name: &str) -> Result<usize, &'static str> {
    required_string(name)?
        .parse::<usize>()
        .map_err(|_| "required number is invalid")
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn elapsed_millis(started_at: Instant) -> u64 {
    duration_millis(started_at.elapsed())
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn write_report(path: &Path, report: &PressureReport) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(report)?)?;
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
