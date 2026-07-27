//! Cross-platform supervision for complete child process trees.

use std::fmt;
use std::io;
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
use std::time::Duration;

#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Instant;

use command_group::{CommandGroup, GroupChild};

#[cfg(unix)]
use command_group::{Signal, UnixChildExt};

#[cfg(unix)]
const TERMINATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Fixed failure classes exposed by complete process-tree supervision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSupervisorErrorKind {
    /// The child could not be spawned or assigned to its process group or Job Object.
    SpawnOrAssign,
    /// The Unix process group could not receive its graceful termination signal.
    GracefulSignal,
    /// The process group or Job Object could not be forcefully terminated.
    ForceKill,
    /// The process group or Job Object could not be polled or reaped.
    Wait,
}

impl ProcessSupervisorErrorKind {
    /// Return the stable machine-readable failure class.
    ///
    /// # Returns
    ///
    /// Fixed ASCII classification suitable for metrics and structured logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SpawnOrAssign => "spawn_or_assign_failed",
            Self::GracefulSignal => "terminate_failed",
            Self::ForceKill => "kill_failed",
            Self::Wait => "wait_failed",
        }
    }
}

/// Redacted process supervisor failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessSupervisorError {
    kind: ProcessSupervisorErrorKind,
    io_kind: io::ErrorKind,
}

impl ProcessSupervisorError {
    fn new(kind: ProcessSupervisorErrorKind, error: &io::Error) -> Self {
        Self {
            kind,
            io_kind: error.kind(),
        }
    }

    /// Return the fixed process-supervision failure class.
    ///
    /// # Returns
    ///
    /// Stable error classification without command paths or operating-system messages.
    pub const fn kind(&self) -> ProcessSupervisorErrorKind {
        self.kind
    }

    /// Return the standard I/O error category without its sensitive free-form message.
    ///
    /// # Returns
    ///
    /// Operating-system-independent I/O error category.
    pub const fn io_kind(&self) -> io::ErrorKind {
        self.io_kind
    }
}

impl fmt::Display for ProcessSupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ProcessSupervisorErrorKind::SpawnOrAssign => "process group spawn or assignment failed",
            ProcessSupervisorErrorKind::GracefulSignal => {
                "process group graceful termination failed"
            }
            ProcessSupervisorErrorKind::ForceKill => "process group force termination failed",
            ProcessSupervisorErrorKind::Wait => "process group wait failed",
        })
    }
}

impl std::error::Error for ProcessSupervisorError {}

/// How a requested process-tree termination completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessTerminationMode {
    /// The complete process group had already exited before termination began.
    AlreadyExited,
    /// A Unix SIGTERM stopped the complete process group during the grace period.
    Graceful,
    /// SIGKILL or Windows Job Object termination was required.
    Forced,
}

/// Reaped process-tree termination result.
#[derive(Debug)]
pub struct ProcessTermination {
    mode: ProcessTerminationMode,
    status: ExitStatus,
}

impl ProcessTermination {
    /// Return how the process tree reached its terminal state.
    ///
    /// # Returns
    ///
    /// Stable termination mode.
    pub const fn mode(&self) -> ProcessTerminationMode {
        self.mode
    }

    /// Return the direct child's reaped exit status.
    ///
    /// # Returns
    ///
    /// Exit status retained after all supervised process-tree work ended.
    pub const fn status(&self) -> &ExitStatus {
        &self.status
    }
}

/// Child process isolated in a Unix process group or Windows Job Object.
#[derive(Debug)]
pub struct SupervisedChild {
    child: GroupChild,
    is_reaped: bool,
}

impl SupervisedChild {
    /// Spawn a command inside an independently terminable process tree.
    ///
    /// # Arguments
    ///
    /// * `command` - Fully configured typed command without a shell intermediary.
    ///
    /// # Returns
    ///
    /// Supervised child or a fixed spawn/assignment failure.
    pub fn spawn(command: &mut Command) -> Result<Self, ProcessSupervisorError> {
        let mut group = command.group();
        #[cfg(windows)]
        group.kill_on_drop(true);
        let child = group.spawn().map_err(|error| {
            ProcessSupervisorError::new(ProcessSupervisorErrorKind::SpawnOrAssign, &error)
        })?;
        Ok(Self {
            child,
            is_reaped: false,
        })
    }

    /// Return the operating-system process identifier for the group leader.
    ///
    /// # Returns
    ///
    /// Direct child process identifier.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Take the configured standard-input pipe before process polling begins.
    ///
    /// # Returns
    ///
    /// Owned input pipe when the command requested one.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.inner().stdin.take()
    }

    /// Take the configured standard-output pipe before process polling begins.
    ///
    /// # Returns
    ///
    /// Owned output pipe when the command requested one.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.inner().stdout.take()
    }

    /// Take the configured standard-error pipe before process polling begins.
    ///
    /// # Returns
    ///
    /// Owned error pipe when the command requested one.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.inner().stderr.take()
    }

    /// Poll the complete process tree and reap it when it has exited.
    ///
    /// # Returns
    ///
    /// `None` while work remains, otherwise the direct child's cached exit status.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, ProcessSupervisorError> {
        let status = self.child.try_wait().map_err(|error| {
            ProcessSupervisorError::new(ProcessSupervisorErrorKind::Wait, &error)
        })?;
        if status.is_some() {
            self.is_reaped = true;
        }
        Ok(status)
    }

    /// Wait for and reap the complete process tree.
    ///
    /// # Returns
    ///
    /// Direct child's cached exit status after the process tree has ended.
    pub fn wait(&mut self) -> Result<ExitStatus, ProcessSupervisorError> {
        let status = self.child.wait().map_err(|error| {
            ProcessSupervisorError::new(ProcessSupervisorErrorKind::Wait, &error)
        })?;
        self.is_reaped = true;
        Ok(status)
    }

    /// Request graceful termination, enforce a bound, then reap the complete process tree.
    ///
    /// Unix sends SIGTERM to the process group, waits for the grace period, and sends SIGKILL
    /// only while group members remain. Windows terminates the assigned Job Object atomically.
    ///
    /// # Arguments
    ///
    /// * `grace_period` - Maximum Unix interval before forceful group termination.
    ///
    /// # Returns
    ///
    /// Reaped status and stable termination mode.
    pub fn terminate_tree(
        &mut self,
        grace_period: Duration,
    ) -> Result<ProcessTermination, ProcessSupervisorError> {
        if self.is_reaped {
            let status = self.wait()?;
            return Ok(ProcessTermination {
                mode: ProcessTerminationMode::AlreadyExited,
                status,
            });
        }
        terminate_platform(self, grace_period)
    }

    /// Immediately terminate and reap the complete process tree.
    ///
    /// # Returns
    ///
    /// Direct child's exit status after all group or Job Object members have ended.
    pub fn force_kill_and_wait(&mut self) -> Result<ExitStatus, ProcessSupervisorError> {
        if self.is_reaped {
            return self.wait();
        }
        if let Err(error) = self.child.kill() {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.is_reaped = true;
                    return Ok(status);
                }
                Ok(None) => {
                    return Err(ProcessSupervisorError::new(
                        ProcessSupervisorErrorKind::ForceKill,
                        &error,
                    ));
                }
                Err(wait_error) => {
                    return Err(ProcessSupervisorError::new(
                        ProcessSupervisorErrorKind::Wait,
                        &wait_error,
                    ));
                }
            }
        }
        self.wait()
    }
}

#[cfg(unix)]
fn terminate_platform(
    child: &mut SupervisedChild,
    grace_period: Duration,
) -> Result<ProcessTermination, ProcessSupervisorError> {
    if let Err(error) = child.child.signal(Signal::SIGTERM) {
        return match child.child.try_wait() {
            Ok(Some(status)) => {
                child.is_reaped = true;
                Ok(ProcessTermination {
                    mode: ProcessTerminationMode::AlreadyExited,
                    status,
                })
            }
            Ok(None) => Err(ProcessSupervisorError::new(
                ProcessSupervisorErrorKind::GracefulSignal,
                &error,
            )),
            Err(wait_error) => Err(ProcessSupervisorError::new(
                ProcessSupervisorErrorKind::Wait,
                &wait_error,
            )),
        };
    }

    let deadline = Instant::now() + grace_period;
    while Instant::now() < deadline {
        child.child.try_wait().map_err(|error| {
            ProcessSupervisorError::new(ProcessSupervisorErrorKind::Wait, &error)
        })?;
        thread::sleep(
            TERMINATION_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
        );
    }

    let mode = match child.child.kill() {
        Ok(()) => ProcessTerminationMode::Forced,
        Err(error) => match child.child.try_wait() {
            Ok(Some(_)) => ProcessTerminationMode::Graceful,
            Ok(None) => {
                return Err(ProcessSupervisorError::new(
                    ProcessSupervisorErrorKind::ForceKill,
                    &error,
                ));
            }
            Err(wait_error) => {
                return Err(ProcessSupervisorError::new(
                    ProcessSupervisorErrorKind::Wait,
                    &wait_error,
                ));
            }
        },
    };
    let status = child.wait()?;
    Ok(ProcessTermination { mode, status })
}

#[cfg(windows)]
fn terminate_platform(
    child: &mut SupervisedChild,
    _grace_period: Duration,
) -> Result<ProcessTermination, ProcessSupervisorError> {
    if let Some(status) = child.try_wait()? {
        return Ok(ProcessTermination {
            mode: ProcessTerminationMode::AlreadyExited,
            status,
        });
    }
    let status = child.force_kill_and_wait()?;
    Ok(ProcessTermination {
        mode: ProcessTerminationMode::Forced,
        status,
    })
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        if self.is_reaped {
            return;
        }
        if self.child.kill().is_ok() {
            self.is_reaped = self.child.wait().is_ok();
        } else if self.child.try_wait().is_ok_and(|status| status.is_some()) {
            self.is_reaped = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use tempfile::tempdir;

    use super::{ProcessSupervisorErrorKind, SupervisedChild};

    #[cfg(windows)]
    use super::ProcessTerminationMode;

    const FIXTURE_PATH_ENV: &str = "LITRADAR_PROCESS_TREE_FIXTURE_PATH";

    #[derive(Debug)]
    struct ProcessTreeFixture {
        parent_pid: u32,
        parent_address: SocketAddr,
        child_pid: u32,
        child_address: SocketAddr,
    }

    #[test]
    fn process_tree_termination_reaps_every_listener() {
        let directory = tempdir().expect("temporary process directory should create");
        let fixture_path = directory.path().join("tree.txt");
        let mut child = spawn_process_tree(&fixture_path);
        let fixture = wait_for_process_tree(&fixture_path);

        assert!(listener_is_reachable(fixture.parent_address));
        assert!(listener_is_reachable(fixture.child_address));

        let termination = child
            .terminate_tree(Duration::from_millis(150))
            .expect("complete process tree should terminate");

        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;

            assert_eq!(termination.status().signal(), Some(15));
        }
        #[cfg(windows)]
        assert_eq!(termination.mode(), ProcessTerminationMode::Forced);
        assert_listeners_stopped(&fixture);
    }

    #[test]
    fn process_tree_drop_reaps_every_listener() {
        let directory = tempdir().expect("temporary process directory should create");
        let fixture_path = directory.path().join("tree.txt");
        let child = spawn_process_tree(&fixture_path);
        let fixture = wait_for_process_tree(&fixture_path);

        assert!(listener_is_reachable(fixture.parent_address));
        assert!(listener_is_reachable(fixture.child_address));
        drop(child);

        assert_listeners_stopped(&fixture);
    }

    #[test]
    fn process_tree_spawn_failure_has_fixed_classification() {
        let mut command = Command::new("missing-process-supervisor-fixture-executable");
        let error = SupervisedChild::spawn(&mut command)
            .expect_err("missing executable should fail before process work");

        assert_eq!(error.kind(), ProcessSupervisorErrorKind::SpawnOrAssign);
        assert_eq!(error.kind().as_str(), "spawn_or_assign_failed");
        assert_eq!(
            error.to_string(),
            "process group spawn or assignment failed"
        );
        assert!(!format!("{error:?}").contains("fixture-executable"));
    }

    #[test]
    #[ignore = "helper process for complete process-tree coverage"]
    fn process_tree_parent_fixture() {
        let fixture_path = fixture_path();
        let child_path = fixture_path.with_extension("child");
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("parent fixture listener should bind");
        let parent_address = listener
            .local_addr()
            .expect("parent fixture address should resolve");
        let mut grandchild_command =
            Command::new(env::current_exe().expect("current test executable should resolve"));
        grandchild_command
            .args([
                "--ignored",
                "--exact",
                "process_supervisor::tests::process_tree_grandchild_fixture",
                "--nocapture",
            ])
            .env(FIXTURE_PATH_ENV, &child_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut grandchild = grandchild_command
            .spawn()
            .expect("grandchild fixture should start");
        let (child_pid, child_address) = wait_for_child_fixture(&child_path);
        fs::write(
            &fixture_path,
            format!(
                "{} {} {} {}",
                std::process::id(),
                parent_address,
                child_pid,
                child_address
            ),
        )
        .expect("process tree fixture should publish");

        loop {
            assert!(listener.local_addr().is_ok());
            assert!(grandchild.try_wait().is_ok_and(|status| status.is_none()));
            thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    #[ignore = "helper process for complete process-tree coverage"]
    fn process_tree_grandchild_fixture() {
        let fixture_path = fixture_path();
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("grandchild fixture listener should bind");
        let address = listener
            .local_addr()
            .expect("grandchild fixture address should resolve");
        fs::write(&fixture_path, format!("{} {}", std::process::id(), address))
            .expect("grandchild fixture should publish");

        loop {
            assert!(listener.local_addr().is_ok());
            thread::sleep(Duration::from_secs(1));
        }
    }

    fn spawn_process_tree(fixture_path: &Path) -> SupervisedChild {
        let mut command =
            Command::new(env::current_exe().expect("current test executable should resolve"));
        command
            .args([
                "--ignored",
                "--exact",
                "process_supervisor::tests::process_tree_parent_fixture",
                "--nocapture",
            ])
            .env(FIXTURE_PATH_ENV, fixture_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        SupervisedChild::spawn(&mut command).expect("process tree fixture should start")
    }

    fn fixture_path() -> PathBuf {
        env::var_os(FIXTURE_PATH_ENV)
            .map(PathBuf::from)
            .expect("process fixture path should be provided")
    }

    fn wait_for_child_fixture(path: &Path) -> (u32, SocketAddr) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(contents) = fs::read_to_string(path) {
                let mut parts = contents.split_whitespace();
                if let (Some(pid), Some(address), None) = (parts.next(), parts.next(), parts.next())
                {
                    return (
                        pid.parse().expect("child fixture PID should parse"),
                        address.parse().expect("child fixture address should parse"),
                    );
                }
            }
            assert!(
                Instant::now() < deadline,
                "child fixture did not become ready"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_process_tree(path: &Path) -> ProcessTreeFixture {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(contents) = fs::read_to_string(path) {
                let parts = contents.split_whitespace().collect::<Vec<_>>();
                if parts.len() == 4 {
                    return ProcessTreeFixture {
                        parent_pid: parts[0].parse().expect("parent fixture PID should parse"),
                        parent_address: parts[1]
                            .parse()
                            .expect("parent fixture address should parse"),
                        child_pid: parts[2].parse().expect("child fixture PID should parse"),
                        child_address: parts[3]
                            .parse()
                            .expect("child fixture address should parse"),
                    };
                }
            }
            assert!(
                Instant::now() < deadline,
                "process tree did not become ready"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn listener_is_reachable(address: SocketAddr) -> bool {
        TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
    }

    fn assert_listeners_stopped(fixture: &ProcessTreeFixture) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if !listener_is_reachable(fixture.parent_address)
                && !listener_is_reachable(fixture.child_address)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "process tree listeners remained reachable for parent PID {} and child PID {}",
                fixture.parent_pid,
                fixture.child_pid
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}
