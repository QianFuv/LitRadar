//! Ownership checks for Unix fetch workers in independent process groups.

use std::io;
use std::process::Command;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::{getpgrp, getpid, getppid, Pid};

const PARENT_PID_ENV: &str = "LITRADAR_INDEX_PARENT_PID";
const PARENT_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// Bind a child to this launcher, overriding any inherited ownership value.
pub(crate) fn configure_worker_parent(command: &mut Command) {
    command.env(PARENT_PID_ENV, std::process::id().to_string());
}

/// Stop the worker's isolated process group when its launching process exits.
pub(crate) struct ParentProcessGuard {
    stop: Sender<()>,
    watcher: Option<JoinHandle<()>>,
}

impl ParentProcessGuard {
    /// Validate ownership before work begins and monitor it independently of I/O.
    pub(crate) fn from_environment() -> io::Result<Self> {
        let expected_parent = parse_parent_pid(std::env::var(PARENT_PID_ENV).ok().as_deref())?;
        let process_group = getpgrp();
        if process_group != getpid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "index worker requires an independent process group",
            ));
        }
        check_parent(expected_parent, process_group)?;
        let (stop, receiver) = mpsc::channel();
        let watcher = thread::Builder::new()
            .name("index-parent-watch".to_string())
            .spawn(move || loop {
                match receiver.recv_timeout(PARENT_CHECK_INTERVAL) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        if check_parent(expected_parent, process_group).is_err() {
                            std::process::exit(1);
                        }
                    }
                }
            })?;
        Ok(Self {
            stop,
            watcher: Some(watcher),
        })
    }
}

fn parse_parent_pid(value: Option<&str>) -> io::Result<Pid> {
    value
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value > 0)
        .map(Pid::from_raw)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid index worker parent"))
}

fn check_parent(expected_parent: Pid, process_group: Pid) -> io::Result<()> {
    if getppid() != expected_parent {
        killpg(process_group, Signal::SIGKILL).map_err(io::Error::from)?;
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "index worker parent exited",
        ));
    }
    Ok(())
}

impl Drop for ParentProcessGuard {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::time::Instant;

    use litradar_worker::process_supervisor::SupervisedChild;
    use tempfile::{tempdir, TempDir};

    use super::*;

    const FIXTURE_ROLE_ENV: &str = "LITRADAR_PARENT_FIXTURE_ROLE";
    const FIXTURE_DIRECTORY_ENV: &str = "LITRADAR_PARENT_FIXTURE_DIRECTORY";
    const FIXTURE_MODE_ENV: &str = "LITRADAR_PARENT_FIXTURE_MODE";
    const FIXTURE_LIFETIME: Duration = Duration::from_secs(15);

    struct Fixture {
        parent: SupervisedChild,
        directory: TempDir,
    }

    impl Fixture {
        fn spawn(mode: &str) -> Self {
            let directory = tempdir().unwrap();
            let parent =
                SupervisedChild::spawn(&mut fixture_command("parent", directory.path(), mode))
                    .unwrap();
            Self { parent, directory }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.path().join(name)
        }

        fn stop_parent(&mut self, signal: Signal) {
            killpg(Pid::from_raw(self.parent.id() as i32), signal).unwrap();
            wait_until(Duration::from_secs(2), || {
                self.parent.try_wait().unwrap().is_some()
            });
        }

        fn listeners(&self) -> (SocketAddr, SocketAddr) {
            let worker = read_identity(&self.path("worker"));
            let descendant = read_identity(&self.path("descendant"));
            assert_ne!(worker.0, self.parent.id() as i32);
            assert_eq!(worker.0, worker.1);
            assert_eq!(descendant.1, worker.1);
            assert_ne!(descendant.0, worker.0);
            assert!(is_listening(worker.2));
            assert!(is_listening(descendant.2));
            (worker.2, descendant.2)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(value) = fs::read_to_string(self.path("worker-pid")) {
                if let Ok(worker_pid) = value.parse::<i32>() {
                    if worker_pid > 0 && worker_pid != getpgrp().as_raw() {
                        let _ = killpg(Pid::from_raw(worker_pid), Signal::SIGKILL);
                    }
                }
            }
            let _ = self.parent.force_kill_and_wait();
        }
    }

    fn fixture_command(role: &str, directory: &Path, mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "parent_process::tests::parent_process_fixture",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FIXTURE_ROLE_ENV, role)
            .env(FIXTURE_DIRECTORY_ENV, directory)
            .env(FIXTURE_MODE_ENV, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "process fixture deadline exceeded"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn read_identity(path: &Path) -> (i32, i32, SocketAddr) {
        let mut identity = None;
        wait_until(Duration::from_secs(5), || {
            identity = fs::read_to_string(path).ok().and_then(|value| {
                let mut fields = value.split_whitespace();
                Some((
                    fields.next()?.parse().ok()?,
                    fields.next()?.parse().ok()?,
                    fields.next()?.parse().ok()?,
                ))
            });
            identity.is_some()
        });
        identity.unwrap()
    }

    fn is_listening(address: SocketAddr) -> bool {
        TcpStream::connect_timeout(&address, Duration::from_millis(20)).is_ok()
    }

    fn publish_listener(path: &Path) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        fs::write(
            path,
            format!(
                "{} {} {}",
                getpid(),
                getpgrp(),
                listener.local_addr().unwrap()
            ),
        )
        .unwrap();
        thread::spawn(move || {
            for connection in listener.incoming() {
                drop(connection.unwrap());
            }
        });
    }

    fn assert_owner_loss(signal: Signal) {
        let mut fixture = Fixture::spawn("guarded");
        wait_until(Duration::from_secs(5), || {
            fixture.path("guard-ready").exists()
        });
        let (worker, descendant) = fixture.listeners();
        let deadline = Instant::now() + Duration::from_secs(2);
        fixture.stop_parent(signal);
        wait_until(deadline.saturating_duration_since(Instant::now()), || {
            !is_listening(worker) && !is_listening(descendant)
        });
    }

    #[test]
    fn parent_process_sigterm_stops_blocked_worker_and_descendant() {
        assert_owner_loss(Signal::SIGTERM);
    }

    #[test]
    fn parent_process_sigkill_stops_blocked_worker_and_descendant() {
        assert_owner_loss(Signal::SIGKILL);
    }

    #[test]
    fn parent_process_loss_before_guard_stops_group_before_work() {
        let mut fixture = Fixture::spawn("startup-race");
        let (worker, descendant) = fixture.listeners();
        fixture.stop_parent(Signal::SIGKILL);
        fs::write(fixture.path("release-startup"), "ready").unwrap();
        wait_until(Duration::from_secs(2), || {
            !is_listening(worker) && !is_listening(descendant)
        });
        assert!(!fixture.path("guard-ready").exists());
    }

    #[test]
    fn parent_process_normal_drop_stops_watcher_without_killing_group() {
        let mut fixture = Fixture::spawn("drop");
        wait_until(Duration::from_secs(5), || {
            fixture.path("guard-ready").exists()
        });
        let (worker, descendant) = fixture.listeners();
        fixture.stop_parent(Signal::SIGKILL);
        thread::sleep(Duration::from_millis(350));
        assert!(is_listening(worker));
        assert!(is_listening(descendant));
    }

    #[test]
    fn parent_process_rejects_invalid_owner_and_overrides_inherited_owner() {
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("-1"),
            Some("wrong"),
            Some("2147483648"),
        ] {
            assert_eq!(
                parse_parent_pid(value).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let mut command = Command::new("unused");
        command.env(PARENT_PID_ENV, "1");
        configure_worker_parent(&mut command);
        let owner = command
            .get_envs()
            .find(|(name, _)| *name == PARENT_PID_ENV)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(owner, std::process::id().to_string().as_str());
    }

    #[test]
    fn parent_process_rejects_shared_group_before_reading_worker_request() {
        let directory = tempdir().unwrap();
        let mut command = fixture_command("shared-group", directory.path(), "guarded");
        configure_worker_parent(&mut command);
        let mut child = command.spawn().unwrap();
        wait_until(Duration::from_secs(5), || {
            child.try_wait().unwrap().is_some()
        });
        assert!(child.wait().unwrap().success());
    }

    #[test]
    #[ignore = "subprocess fixture invoked by the parent-process tests"]
    fn parent_process_fixture() {
        let role = std::env::var(FIXTURE_ROLE_ENV).unwrap();
        let directory = PathBuf::from(std::env::var_os(FIXTURE_DIRECTORY_ENV).unwrap());
        let mode = std::env::var(FIXTURE_MODE_ENV).unwrap();
        thread::spawn(|| {
            thread::sleep(FIXTURE_LIFETIME);
            std::process::exit(99);
        });
        match role.as_str() {
            "parent" => {
                assert_eq!(getpid(), getpgrp());
                let mut command = fixture_command("worker", &directory, &mode);
                configure_worker_parent(&mut command);
                let mut worker = SupervisedChild::spawn(&mut command).unwrap();
                fs::write(directory.join("worker-pid"), worker.id().to_string()).unwrap();
                worker.wait().unwrap();
            }
            "worker" => {
                assert_eq!(getpid(), getpgrp());
                publish_listener(&directory.join("worker"));
                let mut descendant = fixture_command("descendant", &directory, &mode)
                    .spawn()
                    .unwrap();
                read_identity(&directory.join("descendant"));
                if mode == "startup-race" {
                    wait_until(Duration::from_secs(5), || {
                        directory.join("release-startup").exists()
                    });
                }
                let guard = ParentProcessGuard::from_environment().unwrap();
                if mode == "drop" {
                    let started = Instant::now();
                    drop(guard);
                    assert!(started.elapsed() < Duration::from_millis(500));
                    fs::write(directory.join("guard-ready"), "dropped").unwrap();
                    thread::sleep(FIXTURE_LIFETIME);
                } else {
                    fs::write(directory.join("guard-ready"), "started").unwrap();
                    thread::sleep(FIXTURE_LIFETIME);
                    drop(guard);
                }
                descendant.kill().unwrap();
                descendant.wait().unwrap();
            }
            "descendant" => {
                publish_listener(&directory.join("descendant"));
                thread::sleep(FIXTURE_LIFETIME);
            }
            "shared-group" => {
                assert_ne!(getpid(), getpgrp());
                let result = crate::run_live_index_worker_from_file_path(
                    directory.join("missing-request.json"),
                );
                assert!(
                    matches!(result, Err(crate::live::LiveIndexError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput)
                );
            }
            _ => panic!("unexpected fixture role"),
        }
    }
}
