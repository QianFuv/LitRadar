//! Coordinated HTTP and scheduler service runtime.

use std::error::Error;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_api::PreparedApiService;
use litradar_storage::{
    cleanup_security_audit_events, load_audit_retention_days,
    report_security_audit_persistence_failure, SecurityAuditRetentionResult,
};
use litradar_worker::scheduler::{
    prepare_scheduled_runs, run_scheduled_claim, scheduler_worker_id, ScheduledTaskExecution,
    SchedulerCancellation, SchedulerError, SchedulerExecutionResult,
};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing::Instrument;

use crate::config::ServeConfig;

const SCHEDULER_EXECUTION_LIMIT: usize = 4;

const AUDIT_BACKLOG_INTERVAL: Duration = Duration::from_secs(60);

const AUDIT_RETENTION_INTERVAL: Duration = Duration::from_secs(86_400);

/// Run HTTP and scheduling under one coordinated lifecycle.
///
/// # Arguments
///
/// * `config` - Validated service runtime configuration.
///
/// # Returns
///
/// Result indicating whether coordinated startup and shutdown completed successfully.
pub(crate) async fn run_service(config: ServeConfig) -> Result<(), Box<dyn Error>> {
    let started_at = Instant::now();
    tracing::info!(event = "service.starting", component = "runtime");
    let result = run_service_inner(config).await;
    let duration_ms = elapsed_millis(started_at);
    match &result {
        Ok(()) => tracing::info!(
            event = "service.stopped",
            component = "runtime",
            outcome = "success",
            duration_ms,
        ),
        Err(_) => tracing::error!(
            event = "service.failed",
            component = "runtime",
            outcome = "failure",
            error_kind = "service_failure",
            duration_ms,
        ),
    }
    result
}

async fn run_service_inner(config: ServeConfig) -> Result<(), Box<dyn Error>> {
    let api_service = PreparedApiService::prepare(config.api_config.clone()).await?;
    let cancellation = SchedulerCancellation::new();
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let api_future = api_service.run(wait_for_shutdown(shutdown_receiver.clone()));
    let scheduler_future = run_scheduler_loop(
        config.clone(),
        shutdown_receiver.clone(),
        cancellation.clone(),
    );
    let delivery_future = crate::manual_delivery::run_manual_delivery_dispatcher(
        config.clone(),
        shutdown_receiver.clone(),
    );
    let audit_retention_future =
        run_audit_retention_loop(config.auth_db_path.clone(), shutdown_receiver);
    tracing::info!(
        event = "service.ready",
        component = "runtime",
        component_count = 4,
    );
    coordinate_components(
        api_future,
        scheduler_future,
        audit_retention_future,
        delivery_future,
        termination_signal(),
        shutdown_sender,
        cancellation,
    )
    .await
}

async fn run_audit_retention_loop(
    auth_db_path: PathBuf,
    shutdown: watch::Receiver<bool>,
) -> Result<(), Box<dyn Error>> {
    run_audit_retention_loop_with(
        AUDIT_RETENTION_INTERVAL,
        shutdown,
        move || {
            let auth_db_path = auth_db_path.clone();
            async move { run_audit_retention_tick(auth_db_path).await }
        },
        tokio::time::sleep,
    )
    .await
}

async fn run_audit_retention_tick(
    auth_db_path: PathBuf,
) -> Result<SecurityAuditRetentionResult, AuditRetentionTickError> {
    match tokio::task::spawn_blocking(move || {
        let retention_days = load_audit_retention_days(&auth_db_path).map_err(|_| {
            report_security_audit_persistence_failure("retention_setting");
            AuditRetentionTickError {
                error_kind: "setting_error",
            }
        })?;
        cleanup_security_audit_events(&auth_db_path, retention_days, unix_time()).map_err(|_| {
            AuditRetentionTickError {
                error_kind: "persistence_error",
            }
        })
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            report_security_audit_persistence_failure("retention_join");
            Err(AuditRetentionTickError {
                error_kind: "join_error",
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuditRetentionTickError {
    error_kind: &'static str,
}

async fn run_audit_retention_loop_with<Tick, TickFuture, Delay, DelayFuture>(
    interval: Duration,
    mut shutdown: watch::Receiver<bool>,
    mut run_tick: Tick,
    mut delay: Delay,
) -> Result<(), Box<dyn Error>>
where
    Tick: FnMut() -> TickFuture,
    TickFuture: Future<Output = Result<SecurityAuditRetentionResult, AuditRetentionTickError>>,
    Delay: FnMut(Duration) -> DelayFuture,
    DelayFuture: Future<Output = ()>,
{
    let mut has_backlog = false;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let started_at = Instant::now();
        match run_tick().await {
            Ok(result) => {
                has_backlog = result.has_more_expired;
                emit_audit_retention_completed(&result, started_at);
            }
            Err(error) => emit_audit_retention_failed(started_at, error.error_kind),
        }
        if *shutdown.borrow() {
            return Ok(());
        }
        let next_interval = if has_backlog {
            AUDIT_BACKLOG_INTERVAL
        } else {
            interval
        };
        tokio::select! {
            () = delay(next_interval) => {}
            changed = shutdown.changed() => {
                let _ = changed;
                return Ok(());
            }
        }
    }
}

fn emit_audit_retention_completed(result: &SecurityAuditRetentionResult, started_at: Instant) {
    if result.did_run {
        tracing::info!(
            event = "audit.retention.completed",
            component = "security",
            outcome = "success",
            deleted_count = result.deleted_count,
            has_more_expired = result.has_more_expired,
            cutoff = result.cutoff,
            duration_ms = elapsed_millis(started_at),
        );
    } else {
        tracing::debug!(
            event = "audit.retention.skipped",
            component = "security",
            outcome = "success",
            reason = "daily_window_not_due",
            cutoff = result.cutoff,
            duration_ms = elapsed_millis(started_at),
        );
    }
}

fn emit_audit_retention_failed(started_at: Instant, error_kind: &'static str) {
    tracing::error!(
        event = "audit.retention.failed",
        component = "security",
        outcome = "failure",
        error_kind,
        duration_ms = elapsed_millis(started_at),
    );
}

fn unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs_f64()
}

async fn run_scheduler_loop(
    config: ServeConfig,
    shutdown: watch::Receiver<bool>,
    cancellation: SchedulerCancellation,
) -> Result<(), Box<dyn Error>> {
    let worker_id = scheduler_worker_id();
    let scheduler_span = tracing::info_span!(
        "scheduler.loop",
        component = "scheduler",
        worker_id = %worker_id,
    );
    let scheduler_interval = config.scheduler_interval;
    let tick_config = config.clone();
    let tick_worker_id = worker_id.clone();
    let claim_cancellation = cancellation.clone();
    run_scheduler_loop_with(
        scheduler_interval,
        shutdown,
        cancellation,
        &worker_id,
        move |available_slots| {
            let auth_db_path = tick_config.auth_db_path.clone();
            let worker_id = tick_worker_id.clone();
            let span = tracing::Span::current();
            let subscriber = tracing::dispatcher::get_default(Clone::clone);
            async move {
                match tokio::task::spawn_blocking(move || {
                    tracing::dispatcher::with_default(&subscriber, || {
                        span.in_scope(|| {
                            prepare_scheduled_runs(auth_db_path, &worker_id, available_slots)
                        })
                    })
                })
                .await
                {
                    Ok(Ok(result)) => Ok(result),
                    Ok(Err(error)) => Err(SchedulerTickError {
                        source: error.into(),
                        error_kind: "scheduler_error",
                    }),
                    Err(error) => Err(SchedulerTickError {
                        source: error.into(),
                        error_kind: "join_error",
                    }),
                }
            }
        },
        move |claim| {
            let auth_db_path = config.auth_db_path.clone();
            let application_executable = config.application_executable.clone();
            let secret_key_file = config.api_config.secret_key_file.clone();
            let cancellation = claim_cancellation.clone();
            let span = tracing::Span::current();
            let subscriber = tracing::dispatcher::get_default(Clone::clone);
            async move {
                tokio::task::spawn_blocking(move || {
                    tracing::dispatcher::with_default(&subscriber, || {
                        span.in_scope(|| {
                            run_scheduled_claim(
                                auth_db_path,
                                application_executable,
                                secret_key_file,
                                claim,
                                cancellation,
                            )
                        })
                    })
                })
                .await
                .map_err(|_| SchedulerError::ExecutionThread)?
            }
        },
        tokio::time::sleep,
    )
    .instrument(scheduler_span)
    .await
}

struct SchedulerTickError {
    source: Box<dyn Error>,
    error_kind: &'static str,
}

async fn run_scheduler_loop_with<
    Work,
    Tick,
    TickFuture,
    Execute,
    ExecutionFuture,
    Delay,
    DelayFuture,
>(
    scheduler_interval: Duration,
    mut shutdown: watch::Receiver<bool>,
    cancellation: SchedulerCancellation,
    worker_id: &str,
    mut run_tick: Tick,
    mut execute: Execute,
    mut delay: Delay,
) -> Result<(), Box<dyn Error>>
where
    Tick: FnMut(usize) -> TickFuture,
    TickFuture: Future<Output = Result<(SchedulerExecutionResult, Vec<Work>), SchedulerTickError>>,
    Execute: FnMut(Work) -> ExecutionFuture,
    ExecutionFuture:
        Future<Output = Result<ScheduledTaskExecution, SchedulerError>> + Send + 'static,
    Delay: FnMut(Duration) -> DelayFuture,
    DelayFuture: Future<Output = ()>,
{
    let mut executions = JoinSet::new();
    let mut completed = Vec::new();
    let mut outcome: Result<(), Box<dyn Error>> = 'scheduler: loop {
        if cancellation.is_cancelled() || *shutdown.borrow() {
            break Ok(());
        }
        let tick_started_at = Instant::now();
        let (mut result, claims) =
            match run_tick(SCHEDULER_EXECUTION_LIMIT - executions.len()).await {
                Ok(result) => result,
                Err(error) => {
                    emit_scheduler_tick_failed(worker_id, tick_started_at, error.error_kind);
                    break Err(error.source);
                }
            };
        result.executed.append(&mut completed);
        for claim in claims {
            executions.spawn(execute(claim));
        }
        emit_scheduler_tick_completed(worker_id, &result, tick_started_at);
        let next_tick = delay(scheduler_interval);
        tokio::pin!(next_tick);
        loop {
            if cancellation.is_cancelled() || *shutdown.borrow() {
                break 'scheduler Ok(());
            }
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    let _ = changed;
                    break 'scheduler Ok(());
                }
                Some(result) = executions.join_next(), if !executions.is_empty() => {
                    match result {
                        Ok(Ok(execution)) => completed.push(execution),
                        Ok(Err(error)) => {
                            emit_scheduler_tick_failed(worker_id, tick_started_at, "execution_error");
                            break 'scheduler Err(error.into());
                        }
                        Err(error) => {
                            emit_scheduler_tick_failed(worker_id, tick_started_at, "join_error");
                            break 'scheduler Err(error.into());
                        }
                    }
                }
                () = &mut next_tick => break,
            }
        }
    };
    cancellation.cancel();
    while let Some(result) = executions.join_next().await {
        match result {
            Ok(Err(error)) if outcome.is_ok() => outcome = Err(error.into()),
            Err(error) if outcome.is_ok() => outcome = Err(error.into()),
            _ => {}
        }
    }
    outcome
}

fn emit_scheduler_tick_completed(
    worker_id: &str,
    result: &SchedulerExecutionResult,
    started_at: Instant,
) {
    let duration_ms = elapsed_millis(started_at);
    let skipped = result.skipped.len();
    let executed = result.executed.len();
    if result.due == 0 && skipped == 0 && result.claimed == 0 {
        tracing::debug!(
            event = "scheduler.tick.completed",
            component = "scheduler",
            worker_id,
            outcome = "success",
            minute_epoch = result.minute_epoch,
            jobs = result.jobs,
            skipped,
            due = result.due,
            already_executed = result.already_executed,
            queued = result.queued,
            claimed = result.claimed,
            executed,
            duration_ms,
        );
    } else {
        tracing::info!(
            event = "scheduler.tick.completed",
            component = "scheduler",
            worker_id,
            outcome = "success",
            minute_epoch = result.minute_epoch,
            jobs = result.jobs,
            skipped,
            due = result.due,
            already_executed = result.already_executed,
            queued = result.queued,
            claimed = result.claimed,
            executed,
            duration_ms,
        );
    }
}

fn emit_scheduler_tick_failed(worker_id: &str, started_at: Instant, error_kind: &'static str) {
    tracing::error!(
        event = "scheduler.tick.failed",
        component = "scheduler",
        worker_id,
        outcome = "failure",
        error_kind,
        duration_ms = elapsed_millis(started_at),
    );
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

async fn coordinate_components<
    ApiFuture,
    SchedulerFuture,
    AuditFuture,
    DeliveryFuture,
    SignalFuture,
>(
    api_future: ApiFuture,
    scheduler_future: SchedulerFuture,
    audit_future: AuditFuture,
    delivery_future: DeliveryFuture,
    signal_future: SignalFuture,
    shutdown_sender: watch::Sender<bool>,
    cancellation: SchedulerCancellation,
) -> Result<(), Box<dyn Error>>
where
    ApiFuture: Future<Output = Result<(), Box<dyn Error>>>,
    SchedulerFuture: Future<Output = Result<(), Box<dyn Error>>>,
    AuditFuture: Future<Output = Result<(), Box<dyn Error>>>,
    DeliveryFuture: Future<Output = Result<(), Box<dyn Error>>>,
    SignalFuture: Future<Output = ()>,
{
    tokio::pin!(api_future);
    tokio::pin!(scheduler_future);
    tokio::pin!(audit_future);
    tokio::pin!(delivery_future);
    tokio::pin!(signal_future);

    let first = tokio::select! {
        result = &mut api_future => FirstCompletion::Api(result),
        result = &mut scheduler_future => FirstCompletion::Scheduler(result),
        result = &mut audit_future => FirstCompletion::Audit(result),
        result = &mut delivery_future => FirstCompletion::Delivery(result),
        () = &mut signal_future => FirstCompletion::Signal,
    };
    match &first {
        FirstCompletion::Signal => tracing::info!(
            event = "service.shutdown.requested",
            component = "runtime",
            reason = "signal",
        ),
        FirstCompletion::Api(result) => tracing::error!(
            event = "service.component.failed",
            component = "api",
            outcome = if result.is_err() {
                "failure"
            } else {
                "unexpected_stop"
            },
            error_kind = if result.is_err() {
                "component_failure"
            } else {
                "unexpected_stop"
            },
        ),
        FirstCompletion::Scheduler(result) => tracing::error!(
            event = "service.component.failed",
            component = "scheduler",
            outcome = if result.is_err() {
                "failure"
            } else {
                "unexpected_stop"
            },
            error_kind = if result.is_err() {
                "component_failure"
            } else {
                "unexpected_stop"
            },
        ),
        FirstCompletion::Audit(result) => tracing::error!(
            event = "service.component.failed",
            component = "audit_retention",
            outcome = if result.is_err() {
                "failure"
            } else {
                "unexpected_stop"
            },
            error_kind = if result.is_err() {
                "component_failure"
            } else {
                "unexpected_stop"
            },
        ),
        FirstCompletion::Delivery(result) => tracing::error!(
            event = "service.component.failed",
            component = "delivery_dispatcher",
            outcome = if result.is_err() {
                "failure"
            } else {
                "unexpected_stop"
            },
            error_kind = if result.is_err() {
                "component_failure"
            } else {
                "unexpected_stop"
            },
        ),
    }
    cancellation.cancel();
    let _ = shutdown_sender.send(true);

    match first {
        FirstCompletion::Signal => {
            let (api_result, scheduler_result, audit_result, delivery_result) =
                tokio::join!(api_future, scheduler_future, audit_future, delivery_future);
            api_result?;
            scheduler_result?;
            audit_result?;
            delivery_result?;
            tracing::info!(
                event = "service.shutdown.completed",
                component = "runtime",
                outcome = "success",
            );
            Ok(())
        }
        FirstCompletion::Api(api_result) => {
            let (scheduler_result, audit_result, delivery_result) =
                tokio::join!(scheduler_future, audit_future, delivery_future);
            api_result?;
            scheduler_result?;
            audit_result?;
            delivery_result?;
            Err(io::Error::other("HTTP service stopped unexpectedly").into())
        }
        FirstCompletion::Scheduler(scheduler_result) => {
            let (api_result, audit_result, delivery_result) =
                tokio::join!(api_future, audit_future, delivery_future);
            scheduler_result?;
            api_result?;
            audit_result?;
            delivery_result?;
            Err(io::Error::other("scheduler stopped unexpectedly").into())
        }
        FirstCompletion::Audit(audit_result) => {
            let (api_result, scheduler_result, delivery_result) =
                tokio::join!(api_future, scheduler_future, delivery_future);
            audit_result?;
            api_result?;
            scheduler_result?;
            delivery_result?;
            Err(io::Error::other("audit retention stopped unexpectedly").into())
        }
        FirstCompletion::Delivery(delivery_result) => {
            let (api_result, scheduler_result, audit_result) =
                tokio::join!(api_future, scheduler_future, audit_future);
            delivery_result?;
            api_result?;
            scheduler_result?;
            audit_result?;
            Err(io::Error::other("delivery dispatcher stopped unexpectedly").into())
        }
    }
}

enum FirstCompletion {
    Api(Result<(), Box<dyn Error>>),
    Scheduler(Result<(), Box<dyn Error>>),
    Audit(Result<(), Box<dyn Error>>),
    Delivery(Result<(), Box<dyn Error>>),
    Signal,
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(unix)]
async fn termination_signal() {
    let mut terminate =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(_) => {
                tracing::error!(
                    event = "service.signal.failed",
                    component = "runtime",
                    signal = "sigterm",
                    error_kind = "handler_install_failed",
                );
                if tokio::signal::ctrl_c().await.is_ok() {
                    tracing::info!(
                        event = "service.signal.received",
                        component = "runtime",
                        signal = "sigint",
                    );
                }
                return;
            }
        };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if result.is_ok() {
                tracing::info!(
                    event = "service.signal.received",
                    component = "runtime",
                    signal = "sigint",
                );
            } else {
                tracing::error!(
                    event = "service.signal.failed",
                    component = "runtime",
                    signal = "sigint",
                    error_kind = "receive_failed",
                );
            }
        }
        received = terminate.recv() => {
            if received.is_some() {
                tracing::info!(
                    event = "service.signal.received",
                    component = "runtime",
                    signal = "sigterm",
                );
            } else {
                tracing::error!(
                    event = "service.signal.failed",
                    component = "runtime",
                    signal = "sigterm",
                    error_kind = "stream_closed",
                );
            }
        }
    }
}

#[cfg(not(unix))]
async fn termination_signal() {
    if tokio::signal::ctrl_c().await.is_ok() {
        tracing::info!(
            event = "service.signal.received",
            component = "runtime",
            signal = "interrupt",
        );
    } else {
        tracing::error!(
            event = "service.signal.failed",
            component = "runtime",
            signal = "interrupt",
            error_kind = "receive_failed",
        );
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::future::pending;
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use litradar_worker::scheduler::{
        ScheduledTaskExecution, SchedulerCancellation, SchedulerError, SchedulerExecutionResult,
        SchedulerMode,
    };
    use serde_json::Value;
    use tokio::sync::{watch, Notify};
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::fmt::MakeWriter;

    use super::{
        coordinate_components, run_audit_retention_loop_with, run_scheduler_loop_with,
        wait_for_shutdown, AuditRetentionTickError, SchedulerTickError,
    };

    #[tokio::test]
    async fn audit_retention_continues_known_backlog_after_transient_failure() {
        let delays = Arc::new(Mutex::new(Vec::new()));
        let observed_delays = Arc::clone(&delays);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut tick_count = 0;
        run_audit_retention_loop_with(
            Duration::from_secs(86_400),
            shutdown_receiver,
            move || {
                tick_count += 1;
                if tick_count == 4 {
                    shutdown_sender
                        .send(true)
                        .expect("loop should remain active");
                }
                let tick = tick_count;
                async move {
                    if tick == 2 {
                        return Err(AuditRetentionTickError {
                            error_kind: "persistence_error",
                        });
                    }
                    Ok(litradar_storage::SecurityAuditRetentionResult {
                        did_run: tick < 4,
                        deleted_count: 1,
                        has_more_expired: tick == 1,
                        cutoff: 1000.0,
                    })
                }
            },
            move |duration| {
                observed_delays
                    .lock()
                    .expect("delays should lock")
                    .push(duration);
                async {}
            },
        )
        .await
        .expect("bounded continuation should finish");
        assert_eq!(
            *delays.lock().expect("delays should lock"),
            vec![
                Duration::from_secs(60),
                Duration::from_secs(60),
                Duration::from_secs(86_400)
            ]
        );
    }

    #[tokio::test]
    async fn audit_retention_runs_immediately_and_records_observability() {
        let logs = CapturedLogs::default();
        let tick_count = Arc::new(AtomicUsize::new(0));
        let tick_count_for_work = Arc::clone(&tick_count);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);

        run_audit_retention_loop_with(
            Duration::from_secs(86_400),
            shutdown_receiver,
            move || {
                tick_count_for_work.fetch_add(1, Ordering::SeqCst);
                shutdown_sender
                    .send(true)
                    .expect("retention receiver should remain open");
                async {
                    Ok(litradar_storage::SecurityAuditRetentionResult {
                        did_run: true,
                        deleted_count: 3,
                        has_more_expired: false,
                        cutoff: 1_000.0,
                    })
                }
            },
            |_| pending(),
        )
        .with_subscriber(logs.subscriber())
        .await
        .expect("retention loop should stop after immediate cleanup");

        assert_eq!(tick_count.load(Ordering::SeqCst), 1);
        let events = logs.events();
        let completed = events
            .iter()
            .find(|event| event["event"] == "audit.retention.completed")
            .expect("completed retention event should be emitted");
        assert_eq!(completed["deleted_count"], 3);
        assert_eq!(completed["outcome"], "success");
    }

    #[tokio::test]
    async fn audit_retention_failure_observability_does_not_stop_the_loop() {
        let logs = CapturedLogs::default();
        let tick_count = Arc::new(AtomicUsize::new(0));
        let tick_count_for_work = Arc::clone(&tick_count);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);

        run_audit_retention_loop_with(
            Duration::from_secs(1),
            shutdown_receiver,
            move || {
                let sequence = tick_count_for_work.fetch_add(1, Ordering::SeqCst);
                if sequence == 0 {
                    return std::future::ready(Err(AuditRetentionTickError {
                        error_kind: "persistence_error",
                    }));
                }
                shutdown_sender
                    .send(true)
                    .expect("retention receiver should remain open");
                std::future::ready(Ok(litradar_storage::SecurityAuditRetentionResult {
                    did_run: false,
                    deleted_count: 0,
                    has_more_expired: false,
                    cutoff: 2_000.0,
                }))
            },
            |_| async {},
        )
        .with_subscriber(logs.subscriber())
        .await
        .expect("retention loop should continue after a failed tick");

        assert_eq!(tick_count.load(Ordering::SeqCst), 2);
        let failures = logs
            .events()
            .into_iter()
            .filter(|event| event["event"] == "audit.retention.failed")
            .collect::<Vec<_>>();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["error_kind"], "persistence_error");
        assert!(!logs.text().contains("password_sentinel"));
        assert!(!logs.text().contains("token_sentinel"));
    }

    #[tokio::test]
    async fn component_failure_cancels_and_drains_its_sibling() {
        let logs = CapturedLogs::default();
        let cancellation = SchedulerCancellation::new();
        let assertion_handle = cancellation.clone();
        let did_drain_scheduler = Arc::new(AtomicBool::new(false));
        let scheduler_drain_assertion = Arc::clone(&did_drain_scheduler);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let audit_receiver = shutdown_receiver.clone();
        let delivery_receiver = shutdown_receiver.clone();
        let api =
            async { Err::<(), Box<dyn Error>>(io::Error::other("fixture API failure").into()) };
        let scheduler = async move {
            wait_for_shutdown(shutdown_receiver).await;
            scheduler_drain_assertion.store(true, Ordering::SeqCst);
            Ok::<(), Box<dyn Error>>(())
        };
        let audit = async move {
            wait_for_shutdown(audit_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };
        let delivery = async move {
            wait_for_shutdown(delivery_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };

        let error = coordinate_components(
            api,
            scheduler,
            audit,
            delivery,
            pending(),
            shutdown_sender,
            cancellation,
        )
        .with_subscriber(logs.subscriber())
        .await
        .expect_err("component failure should fail the service");

        assert_eq!(error.to_string(), "fixture API failure");
        assert!(assertion_handle.is_cancelled());
        assert!(did_drain_scheduler.load(Ordering::SeqCst));
        assert!(!logs.text().contains("fixture API failure"));
        let failures = logs
            .events()
            .into_iter()
            .filter(|event| event["event"] == "service.component.failed")
            .collect::<Vec<_>>();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["component"], "api");
        assert_eq!(failures[0]["outcome"], "failure");
    }

    #[tokio::test]
    async fn scheduler_failure_cancels_and_drains_the_api_sibling() {
        let logs = CapturedLogs::default();
        let cancellation = SchedulerCancellation::new();
        let assertion_handle = cancellation.clone();
        let did_drain_api = Arc::new(AtomicBool::new(false));
        let api_drain_assertion = Arc::clone(&did_drain_api);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let audit_receiver = shutdown_receiver.clone();
        let delivery_receiver = shutdown_receiver.clone();
        let api = async move {
            wait_for_shutdown(shutdown_receiver).await;
            api_drain_assertion.store(true, Ordering::SeqCst);
            Ok::<(), Box<dyn Error>>(())
        };
        let scheduler = async {
            Err::<(), Box<dyn Error>>(io::Error::other("fixture scheduler failure").into())
        };
        let audit = async move {
            wait_for_shutdown(audit_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };
        let delivery = async move {
            wait_for_shutdown(delivery_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };

        let error = coordinate_components(
            api,
            scheduler,
            audit,
            delivery,
            pending(),
            shutdown_sender,
            cancellation,
        )
        .with_subscriber(logs.subscriber())
        .await
        .expect_err("scheduler failure should fail the service");

        assert_eq!(error.to_string(), "fixture scheduler failure");
        assert!(assertion_handle.is_cancelled());
        assert!(did_drain_api.load(Ordering::SeqCst));
        assert!(!logs.text().contains("fixture scheduler failure"));
        let failures = logs
            .events()
            .into_iter()
            .filter(|event| event["event"] == "service.component.failed")
            .collect::<Vec<_>>();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["component"], "scheduler");
        assert_eq!(failures[0]["outcome"], "failure");
    }

    #[tokio::test]
    async fn scheduler_loop_scans_again_before_active_work_finishes() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let next_started = Arc::new(Notify::new());
        let clock = Arc::new(Notify::new());
        let slots = Arc::new(Mutex::new(Vec::new()));
        let slots_for_tick = Arc::clone(&slots);
        let finished = Arc::new(AtomicUsize::new(0));
        let finished_for_job = Arc::clone(&finished);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let work_started = Arc::clone(&started);
        let work_release = Arc::clone(&release);
        let next_work_started = Arc::clone(&next_started);
        let tick_clock = Arc::clone(&clock);
        let mut tick_count = 0;
        let scheduler = run_scheduler_loop_with(
            Duration::from_secs(60),
            shutdown_receiver,
            SchedulerCancellation::new(),
            "liveness-fixture",
            move |available_slots| {
                slots_for_tick
                    .lock()
                    .expect("slots should lock")
                    .push(available_slots);
                let index = tick_count;
                tick_count += 1;
                async move { Ok::<_, SchedulerTickError>((fixture_scheduler_result(), vec![index])) }
            },
            move |index| {
                let started = Arc::clone(&work_started);
                let release = Arc::clone(&work_release);
                let next_started = Arc::clone(&next_work_started);
                let finished = Arc::clone(&finished_for_job);
                async move {
                    if index == 0 {
                        started.notify_one();
                        release.notified().await;
                    } else {
                        next_started.notify_one();
                    }
                    finished.fetch_add(1, Ordering::SeqCst);
                    Ok(fixture_scheduler_execution(index))
                }
            },
            move |_| {
                let clock = Arc::clone(&tick_clock);
                async move { clock.notified().await }
            },
        );
        let observer = async move {
            started.notified().await;
            clock.notify_one();
            let did_start = tokio::time::timeout(Duration::from_secs(2), next_started.notified())
                .await
                .is_ok();
            shutdown_sender
                .send(true)
                .expect("scheduler should still be listening");
            release.notify_one();
            did_start
        };
        let (result, did_start) = tokio::join!(scheduler, observer);
        result.expect("scheduler should shut down");
        assert!(
            did_start,
            "later work must start before the earlier task finishes"
        );
        assert_eq!(*slots.lock().expect("slots should lock"), vec![4, 3]);
        assert_eq!(
            finished.load(Ordering::SeqCst),
            2,
            "shutdown must drain both jobs"
        );
    }

    #[tokio::test]
    async fn scheduler_execution_failure_cancels_and_drains_other_jobs() {
        let cancellation = SchedulerCancellation::new();
        let job_cancellation = cancellation.clone();
        let did_finish = Arc::new(AtomicBool::new(false));
        let job_finished = Arc::clone(&did_finish);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let scheduler = run_scheduler_loop_with(
            Duration::from_secs(60),
            shutdown_receiver,
            cancellation.clone(),
            "failure-fixture",
            |_| async {
                Ok::<_, SchedulerTickError>((fixture_scheduler_result(), vec![false, true]))
            },
            move |should_fail| {
                let cancellation = job_cancellation.clone();
                let did_finish = Arc::clone(&job_finished);
                async move {
                    if should_fail {
                        return Err(SchedulerError::HeartbeatLost);
                    }
                    while !cancellation.is_cancelled() {
                        tokio::task::yield_now().await;
                    }
                    did_finish.store(true, Ordering::SeqCst);
                    Ok(fixture_scheduler_execution(1))
                }
            },
            |_| pending(),
        );
        let result = tokio::time::timeout(Duration::from_secs(2), scheduler)
            .await
            .expect("failure must not leave a running task behind");
        assert!(result.is_err());
        assert!(cancellation.is_cancelled());
        assert!(did_finish.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn scheduler_loop_runs_the_first_tick_before_waiting() {
        let cancellation = SchedulerCancellation::new();
        let tick_cancellation = cancellation.clone();
        let tick_count = Arc::new(AtomicUsize::new(0));
        let tick_count_assertion = Arc::clone(&tick_count);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);

        run_scheduler_loop_with(
            Duration::from_secs(60),
            shutdown_receiver,
            cancellation,
            "fixture-worker",
            move |_| {
                tick_count_assertion.fetch_add(1, Ordering::SeqCst);
                tick_cancellation.cancel();
                async {
                    Ok::<_, SchedulerTickError>((fixture_scheduler_result(), Vec::<()>::new()))
                }
            },
            |_: ()| pending::<Result<ScheduledTaskExecution, SchedulerError>>(),
            |_| pending(),
        )
        .await
        .expect("cancelled loop should stop after its first tick");

        assert_eq!(tick_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scheduler_loop_stops_while_waiting_without_an_interval_sleep() {
        let cancellation = SchedulerCancellation::new();
        let tick_count = Arc::new(AtomicUsize::new(0));
        let tick_count_assertion = Arc::clone(&tick_count);
        let wait_started = Arc::new(Notify::new());
        let wait_started_for_delay = Arc::clone(&wait_started);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let scheduler = run_scheduler_loop_with(
            Duration::from_secs(3_600),
            shutdown_receiver,
            cancellation,
            "fixture-worker",
            move |_| {
                tick_count_assertion.fetch_add(1, Ordering::SeqCst);
                async {
                    Ok::<_, SchedulerTickError>((fixture_scheduler_result(), Vec::<()>::new()))
                }
            },
            |_: ()| pending::<Result<ScheduledTaskExecution, SchedulerError>>(),
            move |_| {
                let wait_started = Arc::clone(&wait_started_for_delay);
                async move {
                    wait_started.notify_one();
                    pending::<()>().await;
                }
            },
        );
        let shutdown = async move {
            wait_started.notified().await;
            shutdown_sender
                .send(true)
                .expect("scheduler shutdown receiver should remain open");
        };

        let (result, ()) = tokio::join!(scheduler, shutdown);

        result.expect("shutdown should stop the waiting scheduler loop");
        assert_eq!(tick_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn termination_during_scheduler_work_cancels_and_drains_both_components() {
        let cancellation = SchedulerCancellation::new();
        let tick_cancellation = cancellation.clone();
        let assertion_handle = cancellation.clone();
        let tick_started = Arc::new(Notify::new());
        let tick_started_for_work = Arc::clone(&tick_started);
        let did_finish_tick = Arc::new(AtomicBool::new(false));
        let did_finish_tick_assertion = Arc::clone(&did_finish_tick);
        let did_drain_api = Arc::new(AtomicBool::new(false));
        let did_drain_api_assertion = Arc::clone(&did_drain_api);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let api_receiver = shutdown_receiver.clone();
        let audit_receiver = shutdown_receiver.clone();
        let delivery_receiver = shutdown_receiver.clone();
        let api = async move {
            wait_for_shutdown(api_receiver).await;
            did_drain_api_assertion.store(true, Ordering::SeqCst);
            Ok::<(), Box<dyn Error>>(())
        };
        let scheduler = run_scheduler_loop_with(
            Duration::from_secs(3_600),
            shutdown_receiver,
            cancellation.clone(),
            "fixture-worker",
            |_| async { Ok::<_, SchedulerTickError>((fixture_scheduler_result(), vec![()])) },
            move |()| {
                let tick_started = Arc::clone(&tick_started_for_work);
                let cancellation = tick_cancellation.clone();
                let did_finish_tick = Arc::clone(&did_finish_tick_assertion);
                async move {
                    tick_started.notify_one();
                    while !cancellation.is_cancelled() {
                        tokio::task::yield_now().await;
                    }
                    did_finish_tick.store(true, Ordering::SeqCst);
                    Ok::<_, SchedulerError>(fixture_scheduler_execution(1))
                }
            },
            |_| pending(),
        );
        let signal = async move {
            tick_started.notified().await;
        };
        let audit = async move {
            wait_for_shutdown(audit_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };
        let delivery = async move {
            wait_for_shutdown(delivery_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };

        coordinate_components(
            api,
            scheduler,
            audit,
            delivery,
            signal,
            shutdown_sender,
            cancellation,
        )
        .await
        .expect("termination should drain work in progress");

        assert!(assertion_handle.is_cancelled());
        assert!(did_finish_tick.load(Ordering::SeqCst));
        assert!(did_drain_api.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn termination_drains_all_components_successfully() {
        let logs = CapturedLogs::default();
        let cancellation = SchedulerCancellation::new();
        let assertion_handle = cancellation.clone();
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let api_receiver = shutdown_receiver.clone();
        let audit_receiver = shutdown_receiver.clone();
        let delivery_receiver = shutdown_receiver.clone();
        let api = async move {
            wait_for_shutdown(api_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };
        let scheduler = async move {
            wait_for_shutdown(shutdown_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };
        let audit = async move {
            wait_for_shutdown(audit_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };
        let delivery = async move {
            wait_for_shutdown(delivery_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };

        coordinate_components(
            api,
            scheduler,
            audit,
            delivery,
            async {},
            shutdown_sender,
            cancellation,
        )
        .with_subscriber(logs.subscriber())
        .await
        .expect("termination should drain cleanly");

        assert!(assertion_handle.is_cancelled());
        let events = logs.events();
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "service.shutdown.requested")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "service.shutdown.completed")
                .count(),
            1
        );
    }

    fn fixture_scheduler_execution(task_id: i64) -> ScheduledTaskExecution {
        ScheduledTaskExecution {
            task_id,
            job_id: format!("fixture-{task_id}"),
            name: "Fixture task".into(),
            status: litradar_domain::SchedulerRunState::Success,
        }
    }

    fn fixture_scheduler_result() -> SchedulerExecutionResult {
        SchedulerExecutionResult {
            mode: SchedulerMode::Execute,
            status: litradar_domain::SchedulerRunState::Success,
            minute_epoch: 0,
            checked_from: 0.0,
            checked_to: 0.0,
            jobs: 0,
            skipped: Vec::new(),
            due: 0,
            already_executed: 0,
            queued: 0,
            claimed: 0,
            executed: Vec::new(),
        }
    }

    #[derive(Clone, Default)]
    struct CapturedLogs {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturedLogs {
        fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_writer(self.clone())
                .json()
                .flatten_event(true)
                .finish()
        }

        fn text(&self) -> String {
            String::from_utf8(
                self.bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            )
            .expect("captured logs should be UTF-8")
        }

        fn events(&self) -> Vec<Value> {
            self.text()
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_str(line).expect("captured log should be JSON"))
                .collect()
        }
    }

    struct CapturedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for CapturedLogs {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }
}
