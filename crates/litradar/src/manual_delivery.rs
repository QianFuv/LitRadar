//! Bounded service dispatcher for durable manual delivery child processes.

use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_storage::{DeliveryRunRecord, DeliveryRunStatus};
use litradar_worker::process_supervisor::SupervisedChild;
use litradar_worker::scheduler::INTERNAL_PARENT_RUN_ID_ARGUMENT;
use tokio::sync::watch;

use crate::config::ServeConfig;

const DISPATCH_POLL_INTERVAL: Duration = Duration::from_millis(100);
const SHUTDOWN_COOPERATIVE_GRACE: Duration = Duration::from_millis(250);
const PROCESS_TERMINATION_GRACE: Duration = Duration::from_millis(250);

struct ActiveManualChild {
    delivery_run_id: i64,
    owner_id: String,
    child: SupervisedChild,
    stop: Option<ChildStop>,
}

#[derive(Clone, Copy)]
struct ChildStop {
    kind: ChildStopKind,
    started_at: Instant,
}

#[derive(Clone, Copy)]
enum ChildStopKind {
    Cancellation,
    Deadline,
    Shutdown,
    Terminal,
}

/// Run the durable manual-delivery dispatcher until service shutdown.
///
/// # Arguments
///
/// * `config` - Service paths and executable configuration.
/// * `shutdown` - Coordinated service shutdown receiver.
///
/// # Returns
///
/// Empty result after a clean drain, or a fixed dispatcher infrastructure failure.
pub(crate) async fn run_manual_delivery_dispatcher(
    config: ServeConfig,
    shutdown: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth_db_path = config.auth_db_path.clone();
    let concurrency = tokio::task::spawn_blocking(move || {
        litradar_storage::load_delivery_worker_concurrency(&auth_db_path)
    })
    .await
    .map_err(|_| io::Error::other("delivery dispatcher setting task failed"))?
    .map_err(|_| io::Error::other("delivery dispatcher setting load failed"))?;
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    let worker = tokio::task::spawn_blocking(move || {
        run_dispatcher_blocking(config, concurrency, worker_cancellation)
    });
    tokio::pin!(worker);

    tokio::select! {
        result = &mut worker => {
            result.map_err(|_| io::Error::other("delivery dispatcher task failed"))??;
            if !*shutdown.borrow() {
                return Err(io::Error::other("delivery dispatcher stopped unexpectedly").into());
            }
        }
        () = wait_for_shutdown(shutdown.clone()) => {
            cancellation.store(true, Ordering::SeqCst);
            worker
                .await
                .map_err(|_| io::Error::other("delivery dispatcher task failed"))??;
        }
    }
    Ok(())
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

fn run_dispatcher_blocking(
    config: ServeConfig,
    concurrency: usize,
    cancellation: Arc<AtomicBool>,
) -> Result<(), io::Error> {
    let mut children = Vec::with_capacity(concurrency);
    while !cancellation.load(Ordering::SeqCst) {
        reap_children(&mut children)?;
        enforce_child_stops(&config.auth_db_path, &mut children)?;
        dispatch_available(&config, concurrency, &mut children)?;
        thread::sleep(DISPATCH_POLL_INTERVAL);
    }
    cancel_and_reap_children(&config.auth_db_path, &mut children)
}

fn reap_children(children: &mut Vec<ActiveManualChild>) -> Result<(), io::Error> {
    let mut index = 0;
    while index < children.len() {
        let status = children[index]
            .child
            .try_wait()
            .map_err(|_| io::Error::other("delivery child wait failed"))?;
        let Some(status) = status else {
            index += 1;
            continue;
        };
        tracing::info!(
            event = "delivery.dispatcher.child_completed",
            component = "delivery_dispatcher",
            outcome = if status.success() {
                "success"
            } else {
                "failure"
            },
            delivery_run_id = children[index].delivery_run_id,
            exit_success = status.success(),
        );
        children.swap_remove(index);
    }
    Ok(())
}

fn dispatch_available(
    config: &ServeConfig,
    concurrency: usize,
    children: &mut Vec<ActiveManualChild>,
) -> Result<(), io::Error> {
    let available = concurrency.saturating_sub(children.len());
    if available == 0 {
        return Ok(());
    }
    let now = unix_time();
    let candidates = litradar_storage::list_dispatchable_manual_delivery_runs(
        &config.auth_db_path,
        now,
        available,
    )
    .map_err(|_| io::Error::other("delivery dispatch query failed"))?;
    for candidate in candidates {
        if children
            .iter()
            .any(|child| child.delivery_run_id == candidate.id)
        {
            continue;
        }
        if candidate
            .deadline_at
            .is_some_and(|deadline| deadline <= now)
            && candidate.status == DeliveryRunStatus::Queued
        {
            let _ = litradar_storage::finalize_queued_delivery_run(
                &config.auth_db_path,
                candidate.id,
                candidate.revision,
                DeliveryRunStatus::TimedOut,
                None,
                Some("deadline_exceeded"),
                now,
            );
            continue;
        }
        match spawn_delivery_child(config, &candidate) {
            Ok((child, owner_id)) => children.push(ActiveManualChild {
                delivery_run_id: candidate.id,
                owner_id,
                child,
                stop: None,
            }),
            Err(()) => {
                tracing::error!(
                    event = "delivery.dispatcher.spawn_failed",
                    component = "delivery_dispatcher",
                    outcome = "failure",
                    error_kind = "spawn_or_assign_failed",
                    delivery_run_id = candidate.id,
                );
                if candidate.status == DeliveryRunStatus::Queued {
                    let _ = litradar_storage::finalize_queued_delivery_run(
                        &config.auth_db_path,
                        candidate.id,
                        candidate.revision,
                        DeliveryRunStatus::Failed,
                        None,
                        Some("spawn_or_assign_failed"),
                        unix_time(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn spawn_delivery_child(
    config: &ServeConfig,
    run: &DeliveryRunRecord,
) -> Result<(SupervisedChild, String), ()> {
    let owner_id = format!(
        "manual-worker-{}",
        litradar_storage::random_hex(16).map_err(|_| ())?
    );
    let mut command = Command::new(&config.application_executable);
    command.args([
        "delivery-run",
        "--project-root",
        &path_argument(&config.api_config.project_root),
        "--auth-db",
        &path_argument(&config.auth_db_path),
        "--secret-key-file",
        &path_argument(&config.api_config.secret_key_file),
        "--run-id",
        &run.id.to_string(),
        "--owner-id",
        &owner_id,
        INTERNAL_PARENT_RUN_ID_ARGUMENT,
        &format!("manual-delivery-{}", run.id),
    ]);
    #[cfg(test)]
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    SupervisedChild::spawn(&mut command)
        .map(|child| (child, owner_id))
        .map_err(|_| ())
}

fn enforce_child_stops(
    auth_db_path: &PathBuf,
    children: &mut Vec<ActiveManualChild>,
) -> Result<(), io::Error> {
    let now = unix_time();
    let mut index = 0;
    while index < children.len() {
        let run =
            litradar_storage::load_delivery_run(auth_db_path, children[index].delivery_run_id)
                .map_err(|_| io::Error::other("delivery cancellation query failed"))?;
        if let Some(run) = run {
            let stop_kind = if run.deadline_at.is_some_and(|deadline| deadline <= now) {
                Some(ChildStopKind::Deadline)
            } else if run.cancellation_requested {
                Some(ChildStopKind::Cancellation)
            } else if run.status.is_terminal() {
                Some(ChildStopKind::Terminal)
            } else {
                None
            };
            if children[index].stop.is_none() {
                children[index].stop = stop_kind.map(|kind| ChildStop {
                    kind,
                    started_at: if matches!(kind, ChildStopKind::Deadline) {
                        Instant::now() - SHUTDOWN_COOPERATIVE_GRACE
                    } else {
                        Instant::now()
                    },
                });
            }
        }
        let should_force = children[index]
            .stop
            .is_some_and(|stop| stop.started_at.elapsed() >= SHUTDOWN_COOPERATIVE_GRACE);
        if !should_force {
            index += 1;
            continue;
        }
        let mut active = children.swap_remove(index);
        if matches!(
            active.stop.map(|stop| stop.kind),
            Some(ChildStopKind::Deadline)
        ) {
            active
                .child
                .force_kill_and_wait()
                .map_err(|_| io::Error::other("delivery child termination failed"))?;
        } else {
            active
                .child
                .terminate_tree(PROCESS_TERMINATION_GRACE)
                .map_err(|_| io::Error::other("delivery child termination failed"))?;
        }
        finalize_forced_child(auth_db_path, &active)?;
    }
    Ok(())
}

fn finalize_forced_child(
    auth_db_path: &PathBuf,
    active: &ActiveManualChild,
) -> Result<(), io::Error> {
    let Some(run) = litradar_storage::load_delivery_run(auth_db_path, active.delivery_run_id)
        .map_err(|_| io::Error::other("delivery forced-stop query failed"))?
    else {
        return Ok(());
    };
    if run.status.is_terminal() {
        return Ok(());
    }
    if run.status == DeliveryRunStatus::Queued {
        if matches!(
            active.stop.map(|stop| stop.kind),
            Some(ChildStopKind::Deadline)
        ) {
            let _ = litradar_storage::finalize_queued_delivery_run(
                auth_db_path,
                run.id,
                run.revision,
                DeliveryRunStatus::TimedOut,
                None,
                Some("deadline_exceeded"),
                unix_time(),
            );
        }
        return Ok(());
    }
    if run.owner_id.as_deref() != Some(active.owner_id.as_str()) {
        return Ok(());
    }
    let error_code = match active.stop.map(|stop| stop.kind) {
        Some(ChildStopKind::Cancellation) => "forced_cancellation_unknown",
        Some(ChildStopKind::Deadline) => "forced_deadline_unknown",
        Some(ChildStopKind::Shutdown) => "forced_shutdown_unknown",
        Some(ChildStopKind::Terminal) | None => "forced_termination_unknown",
    };
    let _ = litradar_storage::finalize_delivery_run(
        auth_db_path,
        run.id,
        &active.owner_id,
        run.revision,
        DeliveryRunStatus::Unknown,
        None,
        Some(error_code),
        unix_time(),
    );
    Ok(())
}

fn cancel_and_reap_children(
    auth_db_path: &PathBuf,
    children: &mut Vec<ActiveManualChild>,
) -> Result<(), io::Error> {
    for active in children.iter_mut() {
        let Ok(Some(run)) =
            litradar_storage::load_delivery_run(auth_db_path, active.delivery_run_id)
        else {
            continue;
        };
        if !run.status.is_terminal() && run.owner_id.as_deref() == Some(active.owner_id.as_str()) {
            let _ = litradar_storage::request_delivery_run_cancellation(
                auth_db_path,
                run.id,
                run.revision,
                unix_time(),
            );
        }
        active.stop = Some(ChildStop {
            kind: ChildStopKind::Shutdown,
            started_at: Instant::now(),
        });
    }

    let deadline = Instant::now() + SHUTDOWN_COOPERATIVE_GRACE;
    while !children.is_empty() && Instant::now() < deadline {
        reap_children(children)?;
        if !children.is_empty() {
            thread::sleep(Duration::from_millis(10));
        }
    }
    for active in children.iter_mut() {
        active
            .child
            .terminate_tree(PROCESS_TERMINATION_GRACE)
            .map_err(|_| io::Error::other("delivery child termination failed"))?;
        finalize_forced_child(auth_db_path, active)?;
    }
    children.clear();
    Ok(())
}

fn path_argument(path: &std::path::Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

fn unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use litradar_api::config::ApiConfig;
    use litradar_storage::{
        DeliveryRunAdmissionOutcome, DeliveryRunCreate, DeliveryRunMode, DeliveryRunStatus,
        DeliveryTriggerKind, DeliveryWorkflow, StorageConfig,
    };

    use litradar_worker::process_supervisor::SupervisedChild;

    use super::{
        cancel_and_reap_children, dispatch_available, enforce_child_stops, ActiveManualChild,
    };
    use crate::config::ServeConfig;

    #[test]
    fn dispatcher_enforces_the_instance_pool_bound_across_users() {
        let fixture =
            DispatcherFixture::new(std::env::current_exe().expect("test path should load"));
        let runs = (0..3)
            .map(|index| fixture.admit_user_run(index))
            .collect::<Vec<_>>();
        let mut children = Vec::new();

        dispatch_available(&fixture.config, 2, &mut children)
            .expect("dispatcher should spawn up to its configured bound");

        assert_eq!(children.len(), 2);
        assert_eq!(
            litradar_storage::load_delivery_run(&fixture.config.auth_db_path, runs[2].id)
                .expect("third run should load")
                .expect("third run should remain")
                .status,
            DeliveryRunStatus::Queued
        );
        cancel_and_reap_children(&fixture.config.auth_db_path, &mut children)
            .expect("fixture children should be reaped");
        assert!(children.is_empty());
    }

    #[test]
    fn dispatcher_persists_a_fixed_spawn_failure_for_a_queued_run() {
        let fixture = DispatcherFixture::new(std::path::PathBuf::from(
            "missing-litradar-delivery-child-executable",
        ));
        let run = fixture.admit_user_run(0);
        let mut children = Vec::new();

        dispatch_available(&fixture.config, 1, &mut children)
            .expect("one spawn failure should not stop the dispatcher");

        let terminal = litradar_storage::load_delivery_run(&fixture.config.auth_db_path, run.id)
            .expect("failed run should load")
            .expect("failed run should exist");
        assert_eq!(terminal.status, DeliveryRunStatus::Failed);
        assert_eq!(
            terminal.error_code.as_deref(),
            Some("spawn_or_assign_failed")
        );
        assert!(children.is_empty());
    }

    #[test]
    fn manual_push_deadline_reaps_a_hung_process_tree() {
        let fixture =
            DispatcherFixture::new(std::env::current_exe().expect("test path should load"));
        let run = fixture.admit_user_run_with_deadline(0, fixture.now + 0.2);
        let process_directory = tempfile::tempdir().expect("process fixture should create");
        let (child, parent_address, child_address) =
            spawn_hang_process_tree(process_directory.path());
        assert!(listener_is_reachable(parent_address));
        assert!(listener_is_reachable(child_address));
        let mut children = vec![ActiveManualChild {
            delivery_run_id: run.id,
            owner_id: "deadline-owner".to_string(),
            child,
            stop: None,
        }];
        while super::unix_time() < run.deadline_at.expect("fixture deadline should exist") {
            thread::sleep(Duration::from_millis(5));
        }

        let started_at = Instant::now();
        enforce_child_stops(&fixture.config.auth_db_path, &mut children)
            .expect("deadline should force the process tree to stop");

        assert!(started_at.elapsed() < Duration::from_secs(2));
        assert!(children.is_empty());
        assert_listeners_stopped(parent_address, child_address);
        let terminal = litradar_storage::load_delivery_run(&fixture.config.auth_db_path, run.id)
            .expect("deadline run should load")
            .expect("deadline run should exist");
        assert_eq!(terminal.status, DeliveryRunStatus::TimedOut);
        assert_eq!(terminal.error_code.as_deref(), Some("deadline_exceeded"));
    }

    #[test]
    fn manual_push_cancellation_reaps_a_hung_process_tree_and_marks_unknown() {
        let fixture =
            DispatcherFixture::new(std::env::current_exe().expect("test path should load"));
        let run = fixture.admit_user_run(0);
        let claimed = match litradar_storage::claim_delivery_run(
            &fixture.config.auth_db_path,
            run.id,
            "cancellation-owner",
            run.revision,
            fixture.now,
            60.0,
        )
        .expect("fixture run should claim")
        {
            litradar_storage::DeliveryRunClaimOutcome::Claimed(run) => run,
            other => panic!("unexpected fixture claim: {other:?}"),
        };
        let running = litradar_storage::start_delivery_run(
            &fixture.config.auth_db_path,
            claimed.id,
            "cancellation-owner",
            claimed.revision,
            fixture.now,
        )
        .expect("fixture run should start");
        let process_directory = tempfile::tempdir().expect("process fixture should create");
        let (child, parent_address, child_address) =
            spawn_hang_process_tree(process_directory.path());
        let mut children = vec![ActiveManualChild {
            delivery_run_id: running.id,
            owner_id: "cancellation-owner".to_string(),
            child,
            stop: None,
        }];
        litradar_storage::request_delivery_run_cancellation(
            &fixture.config.auth_db_path,
            running.id,
            running.revision,
            super::unix_time(),
        )
        .expect("fixture cancellation should persist");

        enforce_child_stops(&fixture.config.auth_db_path, &mut children)
            .expect("cancellation should begin a cooperative grace period");
        thread::sleep(super::SHUTDOWN_COOPERATIVE_GRACE + Duration::from_millis(20));
        enforce_child_stops(&fixture.config.auth_db_path, &mut children)
            .expect("expired cancellation grace should force termination");

        assert!(children.is_empty());
        assert_listeners_stopped(parent_address, child_address);
        let terminal = litradar_storage::load_delivery_run(&fixture.config.auth_db_path, run.id)
            .expect("cancelled run should load")
            .expect("cancelled run should exist");
        assert_eq!(terminal.status, DeliveryRunStatus::Unknown);
        assert_eq!(
            terminal.error_code.as_deref(),
            Some("forced_cancellation_unknown")
        );
    }

    #[test]
    #[ignore = "private helper process for durable manual deadline tests"]
    fn manual_dispatcher_hanging_fixture() {
        let Some(directory) = std::env::var_os("LITRADAR_MANUAL_HANG_FIXTURE") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let mode = std::env::var("LITRADAR_MANUAL_HANG_MODE").unwrap_or_default();
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener should bind");
        let address = listener
            .local_addr()
            .expect("fixture listener address should load");
        if mode == "grandchild" {
            std::fs::write(directory.join("grandchild.txt"), address.to_string())
                .expect("grandchild address should publish");
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        let mut child_command =
            Command::new(std::env::current_exe().expect("test path should load"));
        child_command
            .args([
                "--ignored",
                "--exact",
                "manual_delivery::tests::manual_dispatcher_hanging_fixture",
                "--nocapture",
            ])
            .env("LITRADAR_MANUAL_HANG_FIXTURE", &directory)
            .env("LITRADAR_MANUAL_HANG_MODE", "grandchild")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut grandchild = child_command
            .spawn()
            .expect("fixture grandchild should spawn");
        let _grandchild_waiter = thread::spawn(move || {
            let _ = grandchild.wait();
        });
        let child_address = wait_for_address(&directory.join("grandchild.txt"));
        std::fs::write(
            directory.join("ready.txt"),
            format!("{address}\n{child_address}\n"),
        )
        .expect("fixture addresses should publish");
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }

    struct DispatcherFixture {
        _directory: tempfile::TempDir,
        config: ServeConfig,
        now: f64,
    }

    impl DispatcherFixture {
        fn new(application_executable: std::path::PathBuf) -> Self {
            let directory = tempfile::tempdir().expect("dispatcher fixture should create");
            let storage = StorageConfig::from_project_root(directory.path());
            litradar_storage::migrate_storage(&storage).expect("fixture storage should migrate");
            let now = super::unix_time();
            litradar_storage::bootstrap_admin(
                storage.auth_db_path(),
                "dispatcher-bootstrap",
                "hash",
                "salt",
                now,
            )
            .expect("fixture administrator should bootstrap");
            let secret_key_file = directory.path().join("secret.key");
            std::fs::write(&secret_key_file, [83_u8; 32]).expect("fixture secret key should write");
            let api_config = ApiConfig::new(
                directory.path().to_path_buf(),
                "127.0.0.1".to_string(),
                0,
                secret_key_file,
            );
            let config = ServeConfig {
                api_config,
                application_executable,
                auth_db_path: storage.auth_db_path().to_path_buf(),
                scheduler_interval: Duration::from_secs(30),
            };
            Self {
                _directory: directory,
                config,
                now,
            }
        }

        fn admit_user_run(&self, index: usize) -> litradar_storage::DeliveryRunRecord {
            self.admit_user_run_with_deadline(index, self.now + 60.0)
        }

        fn admit_user_run_with_deadline(
            &self,
            index: usize,
            deadline_at: f64,
        ) -> litradar_storage::DeliveryRunRecord {
            let invite = litradar_storage::admin_create_invite_code(&self.config.auth_db_path)
                .expect("fixture invite should create");
            let user = litradar_storage::register_user_with_invite(
                &self.config.auth_db_path,
                &format!("dispatcher-user-{index}"),
                "hash",
                "salt",
                Some(&invite.code),
                self.now,
            )
            .expect("fixture user should register");
            match litradar_storage::admit_delivery_run(
                &self.config.auth_db_path,
                &DeliveryRunCreate {
                    external_id: format!("dispatcher-run-{index}"),
                    workflow: DeliveryWorkflow::Push,
                    scope_key: format!("manual-user-{}", user.id.value()),
                    db_name: None,
                    trigger_kind: DeliveryTriggerKind::Manual,
                    mode: DeliveryRunMode::Execute,
                    user_id: Some(user.id.value()),
                    deadline_at: Some(deadline_at),
                    created_at: self.now + index as f64 / 10.0,
                },
            )
            .expect("fixture run should admit")
            {
                DeliveryRunAdmissionOutcome::Enqueued(run) => run,
                _ => panic!("fixture admission should enqueue a new run"),
            }
        }
    }

    fn wait_for_hang_fixture(directory: &Path) -> (SocketAddr, SocketAddr) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(content) = std::fs::read_to_string(directory.join("ready.txt")) {
                let addresses = content
                    .lines()
                    .map(|line| {
                        line.parse::<SocketAddr>()
                            .expect("fixture address should parse")
                    })
                    .collect::<Vec<_>>();
                if addresses.len() == 2 {
                    return (addresses[0], addresses[1]);
                }
            }
            assert!(
                Instant::now() < deadline,
                "hung process fixture did not start"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn spawn_hang_process_tree(directory: &Path) -> (SupervisedChild, SocketAddr, SocketAddr) {
        let mut command = Command::new(std::env::current_exe().expect("test path should load"));
        command
            .args([
                "--ignored",
                "--exact",
                "manual_delivery::tests::manual_dispatcher_hanging_fixture",
                "--nocapture",
            ])
            .env("LITRADAR_MANUAL_HANG_FIXTURE", directory)
            .env("LITRADAR_MANUAL_HANG_MODE", "parent")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = SupervisedChild::spawn(&mut command).expect("hung process tree should spawn");
        let (parent_address, child_address) = wait_for_hang_fixture(directory);
        assert!(listener_is_reachable(parent_address));
        assert!(listener_is_reachable(child_address));
        (child, parent_address, child_address)
    }

    fn wait_for_address(path: &Path) -> SocketAddr {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(content) = std::fs::read_to_string(path) {
                return content
                    .trim()
                    .parse()
                    .expect("fixture child address should parse");
            }
            assert!(Instant::now() < deadline, "fixture child did not start");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn listener_is_reachable(address: SocketAddr) -> bool {
        TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
    }

    fn assert_listeners_stopped(parent_address: SocketAddr, child_address: SocketAddr) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !listener_is_reachable(parent_address) && !listener_is_reachable(child_address) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(!listener_is_reachable(parent_address));
        assert!(!listener_is_reachable(child_address));
    }
}
