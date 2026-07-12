//! Coordinated HTTP and scheduler service runtime.

use std::error::Error;
use std::future::Future;
use std::io;

use litradar_api::PreparedApiService;
use litradar_worker::scheduler::{
    run_due_scheduler_once, scheduler_worker_id, SchedulerCancellation,
};
use serde_json::json;
use tokio::sync::watch;

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
    let api_service = PreparedApiService::prepare(config.api_config.clone()).await?;
    let cancellation = SchedulerCancellation::new();
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let api_future = api_service.run(wait_for_shutdown(shutdown_receiver.clone()));
    let scheduler_future = run_scheduler_loop(config, shutdown_receiver, cancellation.clone());
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
    loop {
        if cancellation.is_cancelled() || *shutdown.borrow() {
            return Ok(());
        }
        let auth_db_path = config.auth_db_path.clone();
        let application_executable = config.application_executable.clone();
        let secret_key_file = config.api_config.secret_key_file.clone();
        let tick_worker_id = worker_id.clone();
        let tick_cancellation = cancellation.clone();
        let result = tokio::task::spawn_blocking(move || {
            run_due_scheduler_once(
                auth_db_path,
                application_executable,
                secret_key_file,
                &tick_worker_id,
                tick_cancellation,
            )
        })
        .await??;
        println!(
            "{}",
            serde_json::to_string(&json!({
                "component": "scheduler",
                "worker_id": worker_id,
                "status": result.status,
                "minute_epoch": result.minute_epoch,
                "jobs": result.jobs,
                "skipped": result.skipped.len(),
                "due": result.due,
                "already_executed": result.already_executed,
                "queued": result.queued,
                "claimed": result.claimed,
                "executed": result.executed.len(),
                "executions": result.executed,
            }))?
        );
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
    cancellation.cancel();
    let _ = shutdown_sender.send(true);

    match first {
        FirstCompletion::Signal => {
            let (api_result, scheduler_result) = tokio::join!(api_future, scheduler_future);
            api_result?;
            scheduler_result?;
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
            Err(error) => {
                eprintln!("failed to install SIGTERM handler: {error}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                eprintln!("failed to receive SIGINT: {error}");
            }
        }
        _ = terminate.recv() => {}
    }
}

#[cfg(not(unix))]
async fn termination_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to receive interrupt signal: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::future::pending;
    use std::io;

    use litradar_worker::scheduler::SchedulerCancellation;
    use tokio::sync::watch;

    use super::{coordinate_components, wait_for_shutdown};

    #[tokio::test]
    async fn component_failure_cancels_and_drains_its_sibling() {
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
            .await
            .expect_err("component failure should fail the service");

        assert_eq!(error.to_string(), "fixture API failure");
        assert!(assertion_handle.is_cancelled());
    }

    #[tokio::test]
    async fn termination_drains_both_components_successfully() {
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
            .await
            .expect("termination should drain cleanly");

        assert!(assertion_handle.is_cancelled());
    }
}
