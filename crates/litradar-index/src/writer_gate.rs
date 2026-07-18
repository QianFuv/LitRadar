//! Bounded catalog-scoped write admission for multi-process SQLite indexing.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

const WRITER_GATE_TIMEOUT: Duration = Duration::from_secs(180);
const WRITER_GATE_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Catalog-scoped operating-system lock used to serialize SQLite write phases.
#[derive(Debug)]
pub(crate) struct WriterGate {
    file: File,
}

impl WriterGate {
    /// Open or create one reusable empty writer-gate sidecar.
    pub(crate) fn open(path: &Path) -> Result<Self, WriterGateError> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map(|file| Self { file })
            .map_err(WriterGateError::from_io)
    }

    /// Wait for bounded exclusive write admission using production timing.
    pub(crate) fn acquire(&self) -> Result<WriterGateGuard<'_>, WriterGateError> {
        self.acquire_with(WRITER_GATE_TIMEOUT, WRITER_GATE_POLL_INTERVAL)
    }

    /// Wait for bounded exclusive write admission using explicit test timing.
    pub(crate) fn acquire_with(
        &self,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<WriterGateGuard<'_>, WriterGateError> {
        let started_at = Instant::now();
        loop {
            match self.file.try_lock() {
                Ok(()) => {
                    return Ok(WriterGateGuard {
                        file: &self.file,
                        waited_ms: elapsed_millis(started_at),
                    });
                }
                Err(TryLockError::WouldBlock) => {
                    let elapsed = started_at.elapsed();
                    if elapsed >= timeout {
                        return Err(WriterGateError::Timeout {
                            waited_ms: duration_millis(elapsed),
                        });
                    }
                    let remaining = timeout.saturating_sub(elapsed);
                    let sleep_for = poll_interval.min(remaining);
                    if sleep_for.is_zero() {
                        thread::yield_now();
                    } else {
                        thread::sleep(sleep_for);
                    }
                }
                Err(TryLockError::Error(error)) => {
                    return Err(WriterGateError::from_io(error));
                }
            }
        }
    }
}

/// Held catalog writer admission that unlocks when it leaves scope.
#[derive(Debug)]
pub(crate) struct WriterGateGuard<'gate> {
    file: &'gate File,
    waited_ms: u64,
}

impl WriterGateGuard<'_> {
    /// Return the measured time spent waiting for write admission.
    pub(crate) fn waited_ms(&self) -> u64 {
        self.waited_ms
    }
}

impl Drop for WriterGateGuard<'_> {
    /// Release writer admission when the guarded write phase ends.
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Safe writer-gate failure without a sidecar path or operating-system message.
#[derive(Debug)]
pub(crate) enum WriterGateError {
    /// Opening or locking the sidecar failed.
    Io(io::ErrorKind),
    /// The bounded acquisition deadline expired.
    Timeout {
        /// Milliseconds spent waiting before the timeout was returned.
        waited_ms: u64,
    },
}

impl WriterGateError {
    /// Discard one free-form I/O diagnostic while retaining its stable kind.
    pub(crate) fn from_io(error: io::Error) -> Self {
        Self::Io(error.kind())
    }

    /// Return the fixed event classification for this failure.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Timeout { .. } => "timeout",
        }
    }

    /// Return whether the bounded admission deadline expired.
    pub(crate) fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }

    /// Return the stable standard-library I/O kind when present.
    pub(crate) fn io_kind(&self) -> Option<&io::ErrorKind> {
        match self {
            Self::Io(kind) => Some(kind),
            Self::Timeout { .. } => None,
        }
    }

    /// Return measured wait time when the failure followed lock contention.
    pub(crate) fn waited_ms(&self) -> Option<u64> {
        match self {
            Self::Io(_) => None,
            Self::Timeout { waited_ms } => Some(*waited_ms),
        }
    }
}

impl fmt::Display for WriterGateError {
    /// Format one fixed writer-gate diagnostic without system error text.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("writer gate I/O failed"),
            Self::Timeout { .. } => formatter.write_str("writer gate acquisition timed out"),
        }
    }
}

impl std::error::Error for WriterGateError {}

fn elapsed_millis(started_at: Instant) -> u64 {
    duration_millis(started_at.elapsed())
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use rusqlite::{Connection, ErrorCode};
    use tempfile::tempdir;

    use super::{WriterGate, WriterGateError};

    const CHILD_ROLE_ENV: &str = "LITRADAR_WRITER_GATE_TEST_CHILD_ROLE";
    const LOCK_PATH_ENV: &str = "LITRADAR_WRITER_GATE_TEST_LOCK_PATH";
    const DATABASE_PATH_ENV: &str = "LITRADAR_WRITER_GATE_TEST_DATABASE_PATH";
    const READY_PATH_ENV: &str = "LITRADAR_WRITER_GATE_TEST_READY_PATH";
    const INTERVAL_PATH_ENV: &str = "LITRADAR_WRITER_GATE_TEST_INTERVAL_PATH";
    const PROCESS_TEST_NAME: &str =
        "writer_gate::tests::process_real_writer_gate_covers_contention_timeout_serialization_and_exit";

    /// Exercise the writer gate through independent test processes.
    #[test]
    fn process_real_writer_gate_covers_contention_timeout_serialization_and_exit() {
        if let Some(role) = env::var_os(CHILD_ROLE_ENV) {
            run_child_role(&role.to_string_lossy());
            return;
        }

        let directory = tempdir().expect("writer gate test directory should create");
        let lock_path = directory.path().join("catalog.writer.lock");
        let database_path = directory.path().join("contention.sqlite");
        let setup = Connection::open(&database_path).expect("contention database should open");
        setup
            .execute_batch("CREATE TABLE writes (value INTEGER NOT NULL);")
            .expect("contention table should create");
        drop(setup);

        let ready_path = directory.path().join("holder.ready");
        let mut holder = spawn_child(
            "sqlite_holder",
            &lock_path,
            Some(&database_path),
            Some(&ready_path),
            None,
        );
        wait_for_file(&ready_path);

        let contender = Connection::open(&database_path).expect("contender should open");
        contender
            .busy_timeout(Duration::from_millis(20))
            .expect("short busy timeout should configure");
        let uncoordinated_error = contender
            .execute("INSERT INTO writes VALUES (2)", [])
            .expect_err("ungated contender should hit SQLite writer contention");
        assert!(matches!(
            uncoordinated_error,
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked,
                    ..
                },
                _
            )
        ));

        let gate = WriterGate::open(&lock_path).expect("contender gate should open");
        assert!(matches!(
            gate.acquire_with(Duration::from_millis(50), Duration::from_millis(5)),
            Err(WriterGateError::Timeout { .. })
        ));
        let guard = gate
            .acquire_with(Duration::from_secs(2), Duration::from_millis(5))
            .expect("contender should wait outside SQLite and acquire");
        contender
            .execute("INSERT INTO writes VALUES (2)", [])
            .expect("gated contender should write after holder releases");
        assert!(guard.waited_ms() <= 2_000);
        drop(guard);
        assert_child_success(&mut holder);

        let interval_dir = directory.path().join("intervals");
        fs::create_dir_all(&interval_dir).expect("interval directory should create");
        let mut contenders = (0..3)
            .map(|slot| {
                spawn_child(
                    "interval",
                    &lock_path,
                    None,
                    None,
                    Some(&interval_dir.join(format!("{slot}.txt"))),
                )
            })
            .collect::<Vec<_>>();
        for contender in &mut contenders {
            assert_child_success(contender);
        }
        let mut intervals = (0..3)
            .map(|slot| read_interval(&interval_dir.join(format!("{slot}.txt"))))
            .collect::<Vec<_>>();
        intervals.sort_unstable_by_key(|interval| interval.0);
        assert!(intervals.windows(2).all(|pair| pair[0].1 <= pair[1].0));

        let crash_ready_path = directory.path().join("crash.ready");
        let mut crash_holder = spawn_child(
            "crash_holder",
            &lock_path,
            None,
            Some(&crash_ready_path),
            None,
        );
        wait_for_file(&crash_ready_path);
        crash_holder.kill().expect("crash holder should terminate");
        crash_holder.wait().expect("crash holder should reap");
        let guard = gate
            .acquire_with(Duration::from_secs(2), Duration::from_millis(5))
            .expect("process exit should release the writer gate");
        drop(guard);
        assert_eq!(
            fs::metadata(&lock_path)
                .expect("sidecar should exist")
                .len(),
            0
        );
    }

    /// Execute one child-only writer gate role.
    fn run_child_role(role: &str) {
        let lock_path = required_path(LOCK_PATH_ENV);
        let gate = WriterGate::open(&lock_path).expect("child gate should open");
        match role {
            "sqlite_holder" => {
                let database_path = required_path(DATABASE_PATH_ENV);
                let ready_path = required_path(READY_PATH_ENV);
                let guard = gate
                    .acquire_with(Duration::from_secs(2), Duration::from_millis(5))
                    .expect("SQLite holder should acquire gate");
                let connection =
                    Connection::open(database_path).expect("holder database should open");
                connection
                    .execute_batch("BEGIN IMMEDIATE; INSERT INTO writes VALUES (1);")
                    .expect("holder should own SQLite writer transaction");
                fs::write(ready_path, b"ready").expect("holder ready file should write");
                thread::sleep(Duration::from_millis(350));
                connection
                    .execute_batch("COMMIT;")
                    .expect("holder transaction should commit");
                drop(guard);
            }
            "interval" => {
                let interval_path = required_path(INTERVAL_PATH_ENV);
                let guard = gate
                    .acquire_with(Duration::from_secs(5), Duration::from_millis(5))
                    .expect("interval child should acquire gate");
                let started_at = epoch_nanos();
                thread::sleep(Duration::from_millis(100));
                let completed_at = epoch_nanos();
                fs::write(interval_path, format!("{started_at},{completed_at}"))
                    .expect("interval should write");
                drop(guard);
            }
            "crash_holder" => {
                let ready_path = required_path(READY_PATH_ENV);
                let _guard = gate
                    .acquire_with(Duration::from_secs(2), Duration::from_millis(5))
                    .expect("crash holder should acquire gate");
                fs::write(ready_path, b"ready").expect("crash ready file should write");
                thread::sleep(Duration::from_secs(60));
            }
            _ => panic!("unknown writer gate child role"),
        }
    }

    /// Spawn this exact test as one isolated child role.
    fn spawn_child(
        role: &str,
        lock_path: &Path,
        database_path: Option<&Path>,
        ready_path: Option<&Path>,
        interval_path: Option<&Path>,
    ) -> Child {
        let mut command = Command::new(env::current_exe().expect("test executable should resolve"));
        command
            .arg("--exact")
            .arg(PROCESS_TEST_NAME)
            .arg("--nocapture")
            .env(CHILD_ROLE_ENV, role)
            .env(LOCK_PATH_ENV, lock_path);
        if let Some(path) = database_path {
            command.env(DATABASE_PATH_ENV, path);
        }
        if let Some(path) = ready_path {
            command.env(READY_PATH_ENV, path);
        }
        if let Some(path) = interval_path {
            command.env(INTERVAL_PATH_ENV, path);
        }
        command.spawn().expect("writer gate child should spawn")
    }

    /// Wait until a child publishes its bounded readiness marker.
    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(Instant::now() < deadline, "child readiness timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Require one child test process to exit successfully.
    fn assert_child_success(child: &mut Child) {
        let status = child.wait().expect("writer gate child should wait");
        assert!(status.success(), "writer gate child failed: {status}");
    }

    /// Load one child critical-section interval.
    fn read_interval(path: &Path) -> (u128, u128) {
        let text = fs::read_to_string(path).expect("interval should read");
        let (start, end) = text
            .split_once(',')
            .expect("interval should contain a comma");
        (
            start.parse().expect("interval start should parse"),
            end.parse().expect("interval end should parse"),
        )
    }

    /// Read one required child path from its task-specific environment variable.
    fn required_path(name: &str) -> PathBuf {
        env::var_os(name)
            .map(PathBuf::from)
            .expect("writer gate child path should be configured")
    }

    /// Return a monotonic-enough shared epoch timestamp for cross-process interval comparison.
    fn epoch_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow the Unix epoch")
            .as_nanos()
    }
}
