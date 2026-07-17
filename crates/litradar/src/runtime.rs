//! Coordinated HTTP and scheduler service runtime.

use std::error::Error;
use std::future::Future;
use std::io;
use std::time::Instant;

use litradar_api::PreparedApiService;
use litradar_worker::scheduler::{
    run_due_scheduler_once, scheduler_worker_id, SchedulerCancellation, SchedulerExecutionResult,
};
use tokio::sync::watch;
use tracing::Instrument;

use crate::config::ServeConfig;

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
    let scheduler_future = run_scheduler_loop(config, shutdown_receiver, cancellation.clone());
    tracing::info!(
        event = "service.ready",
        component = "runtime",
        component_count = 2,
    );
    coordinate_components(
        api_future,
        scheduler_future,
        termination_signal(),
        shutdown_sender,
        cancellation,
    )
    .await
}

async fn run_scheduler_loop(
    config: ServeConfig,
    mut shutdown: watch::Receiver<bool>,
    cancellation: SchedulerCancellation,
) -> Result<(), Box<dyn Error>> {
    let worker_id = scheduler_worker_id();
    let scheduler_span = tracing::info_span!(
        "scheduler.loop",
        component = "scheduler",
        worker_id = %worker_id,
    );
    async move {
        loop {
            if cancellation.is_cancelled() || *shutdown.borrow() {
                return Ok(());
            }
            let tick_started_at = Instant::now();
            let auth_db_path = config.auth_db_path.clone();
            let application_executable = config.application_executable.clone();
            let secret_key_file = config.api_config.secret_key_file.clone();
            let tick_worker_id = worker_id.clone();
            let tick_cancellation = cancellation.clone();
            let span = tracing::Span::current();
            let subscriber = tracing::dispatcher::get_default(Clone::clone);
            let result = match tokio::task::spawn_blocking(move || {
                tracing::dispatcher::with_default(&subscriber, || {
                    span.in_scope(|| {
                        run_due_scheduler_once(
                            auth_db_path,
                            application_executable,
                            secret_key_file,
                            &tick_worker_id,
                            tick_cancellation,
                        )
                    })
                })
            })
            .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => {
                    emit_scheduler_tick_failed(&worker_id, tick_started_at, "scheduler_error");
                    return Err(error.into());
                }
                Err(error) => {
                    emit_scheduler_tick_failed(&worker_id, tick_started_at, "join_error");
                    return Err(error.into());
                }
            };
            emit_scheduler_tick_completed(&worker_id, &result, tick_started_at);
            if cancellation.is_cancelled() || *shutdown.borrow() {
                return Ok(());
            }
            tokio::select! {
                () = tokio::time::sleep(config.scheduler_interval) => {}
                changed = shutdown.changed() => {
                    let _ = changed;
                    return Ok(());
                }
            }
        }
    }
    .instrument(scheduler_span)
    .await
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

async fn coordinate_components<ApiFuture, SchedulerFuture, SignalFuture>(
    api_future: ApiFuture,
    scheduler_future: SchedulerFuture,
    signal_future: SignalFuture,
    shutdown_sender: watch::Sender<bool>,
    cancellation: SchedulerCancellation,
) -> Result<(), Box<dyn Error>>
where
    ApiFuture: Future<Output = Result<(), Box<dyn Error>>>,
    SchedulerFuture: Future<Output = Result<(), Box<dyn Error>>>,
    SignalFuture: Future<Output = ()>,
{
    tokio::pin!(api_future);
    tokio::pin!(scheduler_future);
    tokio::pin!(signal_future);

    let first = tokio::select! {
        result = &mut api_future => FirstCompletion::Api(result),
        result = &mut scheduler_future => FirstCompletion::Scheduler(result),
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
    }
    cancellation.cancel();
    let _ = shutdown_sender.send(true);

    match first {
        FirstCompletion::Signal => {
            let (api_result, scheduler_result) = tokio::join!(api_future, scheduler_future);
            api_result?;
            scheduler_result?;
            tracing::info!(
                event = "service.shutdown.completed",
                component = "runtime",
                outcome = "success",
            );
            Ok(())
        }
        FirstCompletion::Api(api_result) => {
            let scheduler_result = scheduler_future.await;
            api_result?;
            scheduler_result?;
            Err(io::Error::other("HTTP service stopped unexpectedly").into())
        }
        FirstCompletion::Scheduler(scheduler_result) => {
            let api_result = api_future.await;
            scheduler_result?;
            api_result?;
            Err(io::Error::other("scheduler stopped unexpectedly").into())
        }
    }
}

enum FirstCompletion {
    Api(Result<(), Box<dyn Error>>),
    Scheduler(Result<(), Box<dyn Error>>),
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
    use std::sync::{Arc, Mutex};

    use litradar_worker::scheduler::SchedulerCancellation;
    use serde_json::Value;
    use tokio::sync::watch;
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::fmt::MakeWriter;

    use super::{coordinate_components, wait_for_shutdown};

    #[tokio::test]
    async fn component_failure_cancels_and_drains_its_sibling() {
        let logs = CapturedLogs::default();
        let cancellation = SchedulerCancellation::new();
        let assertion_handle = cancellation.clone();
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let api =
            async { Err::<(), Box<dyn Error>>(io::Error::other("fixture API failure").into()) };
        let scheduler = async move {
            wait_for_shutdown(shutdown_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };

        let error = coordinate_components(api, scheduler, pending(), shutdown_sender, cancellation)
            .with_subscriber(logs.subscriber())
            .await
            .expect_err("component failure should fail the service");

        assert_eq!(error.to_string(), "fixture API failure");
        assert!(assertion_handle.is_cancelled());
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
    async fn termination_drains_both_components_successfully() {
        let logs = CapturedLogs::default();
        let cancellation = SchedulerCancellation::new();
        let assertion_handle = cancellation.clone();
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let api_receiver = shutdown_receiver.clone();
        let api = async move {
            wait_for_shutdown(api_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };
        let scheduler = async move {
            wait_for_shutdown(shutdown_receiver).await;
            Ok::<(), Box<dyn Error>>(())
        };

        coordinate_components(api, scheduler, async {}, shutdown_sender, cancellation)
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
