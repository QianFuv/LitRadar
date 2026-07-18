//! Shared Axum application state.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use litradar_provider::ProviderRegistry;
use litradar_storage::{SecretCodec, StorageConfig};
use tokio::sync::Semaphore;

const AUTH_USERNAME_ATTEMPT_LIMIT: u32 = 5;
const AUTH_USERNAME_WINDOW_SECONDS: u64 = 5 * 60;
const AUTH_GLOBAL_LOGIN_ATTEMPT_LIMIT: u32 = 100;
const AUTH_GLOBAL_REGISTER_ATTEMPT_LIMIT: u32 = 25;
const AUTH_GLOBAL_WINDOW_SECONDS: u64 = 60;
const AUTH_TRACKED_USERNAME_LIMIT: usize = 4_096;
const DEFAULT_BLOCKING_CONCURRENCY: usize = 8;
const DEFAULT_BLOCKING_TIMEOUT: Duration = Duration::from_secs(30);

/// State shared by API route handlers.
#[derive(Clone)]
pub struct ApiState {
    storage_config: StorageConfig,
    secret_codec: SecretCodec,
    are_session_cookies_secure: bool,
    auth_rate_limiter: Arc<Mutex<AuthRateLimiter>>,
    blocking_executor: BlockingExecutor,
    article_providers: Arc<ProviderRegistry>,
}

impl ApiState {
    /// Build API state from storage configuration.
    ///
    /// # Arguments
    ///
    /// * `storage_config` - Data path configuration.
    /// * `secret_codec` - Deployment secret codec.
    /// * `are_session_cookies_secure` - Whether session cookies include Secure.
    ///
    /// # Returns
    ///
    /// Shared API state.
    pub fn new(
        storage_config: StorageConfig,
        secret_codec: SecretCodec,
        are_session_cookies_secure: bool,
    ) -> Self {
        let article_providers = crate::article_access::build_article_provider_registry(
            storage_config.clone(),
            secret_codec.clone(),
        )
        .expect("built-in article provider registry should be valid");
        Self {
            storage_config,
            secret_codec,
            are_session_cookies_secure,
            auth_rate_limiter: Arc::new(Mutex::new(AuthRateLimiter::new(
                AuthRateLimitConfig::default(),
            ))),
            blocking_executor: BlockingExecutor::new(
                DEFAULT_BLOCKING_CONCURRENCY,
                DEFAULT_BLOCKING_TIMEOUT,
            ),
            article_providers: Arc::new(article_providers),
        }
    }

    /// Build API state with deterministic blocking limits for focused tests.
    ///
    /// # Arguments
    ///
    /// * `storage_config` - Data path configuration.
    /// * `secret_codec` - Deployment secret codec.
    /// * `are_session_cookies_secure` - Whether session cookies include Secure.
    /// * `concurrency` - Maximum simultaneously running blocking jobs.
    /// * `timeout` - Default permit-and-result deadline.
    ///
    /// # Returns
    ///
    /// Shared API state with test-specific executor settings.
    #[cfg(test)]
    pub(crate) fn new_with_blocking_limits(
        storage_config: StorageConfig,
        secret_codec: SecretCodec,
        are_session_cookies_secure: bool,
        concurrency: usize,
        timeout: Duration,
    ) -> Self {
        let article_providers = crate::article_access::build_article_provider_registry(
            storage_config.clone(),
            secret_codec.clone(),
        )
        .expect("built-in article provider registry should be valid");
        Self {
            storage_config,
            secret_codec,
            are_session_cookies_secure,
            auth_rate_limiter: Arc::new(Mutex::new(AuthRateLimiter::new(
                AuthRateLimitConfig::default(),
            ))),
            blocking_executor: BlockingExecutor::new(concurrency, timeout),
            article_providers: Arc::new(article_providers),
        }
    }

    /// Replace request-time article providers for focused capability tests.
    ///
    /// # Arguments
    ///
    /// * `article_providers` - Validated test registry.
    ///
    /// # Returns
    ///
    /// API state using the supplied registry.
    #[cfg(test)]
    pub(crate) fn with_article_providers(mut self, article_providers: ProviderRegistry) -> Self {
        self.article_providers = Arc::new(article_providers);
        self
    }

    /// Return storage configuration.
    ///
    /// # Returns
    ///
    /// Storage configuration used by repositories.
    pub fn storage_config(&self) -> &StorageConfig {
        &self.storage_config
    }

    /// Return the deployment secret codec.
    ///
    /// # Returns
    ///
    /// Codec used for persisted integration credentials.
    pub fn secret_codec(&self) -> &SecretCodec {
        &self.secret_codec
    }

    /// Return the validated request-time article provider registry.
    ///
    /// # Returns
    ///
    /// Provider registry shared by all action handlers.
    pub(crate) fn article_providers(&self) -> &ProviderRegistry {
        &self.article_providers
    }

    /// Run synchronous work on Tokio's blocking pool behind the shared concurrency limit.
    ///
    /// # Arguments
    ///
    /// * `work` - Owned synchronous operation to execute.
    ///
    /// # Returns
    ///
    /// Completed output or a bounded-executor failure.
    pub(crate) async fn run_blocking<Work, Output>(
        &self,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let span = tracing::Span::current();
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        self.blocking_executor
            .run(move || tracing::dispatcher::with_default(&subscriber, || span.in_scope(work)))
            .await
    }

    /// Run synchronous work with an operation-specific total deadline.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum time spent waiting for a permit and task result.
    /// * `work` - Owned synchronous operation to execute.
    ///
    /// # Returns
    ///
    /// Completed output or a bounded-executor failure.
    pub(crate) async fn run_blocking_with_timeout<Work, Output>(
        &self,
        timeout: Duration,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let span = tracing::Span::current();
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        self.blocking_executor
            .run_with_timeout(timeout, move || {
                tracing::dispatcher::with_default(&subscriber, || span.in_scope(work))
            })
            .await
    }

    /// Run detached background work behind the concurrency limit without a request deadline.
    ///
    /// # Arguments
    ///
    /// * `work` - Owned synchronous background operation to execute.
    ///
    /// # Returns
    ///
    /// Completed output or an executor shutdown/join failure.
    pub(crate) async fn run_background_blocking<Work, Output>(
        &self,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let span = tracing::Span::current();
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        self.blocking_executor
            .run_without_timeout(move || {
                tracing::dispatcher::with_default(&subscriber, || span.in_scope(work))
            })
            .await
    }

    /// Stop accepting queued blocking work during server shutdown.
    pub(crate) fn close_blocking_executor(&self) {
        self.blocking_executor.close();
    }

    /// Return whether session cookies include the Secure attribute.
    ///
    /// # Returns
    ///
    /// True when session cookies should be marked Secure.
    pub fn are_session_cookies_secure(&self) -> bool {
        self.are_session_cookies_secure
    }

    /// Consume one authentication attempt or return the retry delay.
    ///
    /// # Arguments
    ///
    /// * `kind` - Login or registration global bucket.
    /// * `username` - Username used for the normalized per-account bucket.
    ///
    /// # Returns
    ///
    /// Empty result when allowed, or Retry-After seconds when limited.
    pub(crate) fn check_auth_attempt(
        &self,
        kind: AuthAttemptKind,
        username: &str,
    ) -> Result<(), u64> {
        let mut limiter = self
            .auth_rate_limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limiter.check(kind, username, current_unix_seconds())
    }

    /// Clear the per-username failure bucket after successful authentication.
    ///
    /// # Arguments
    ///
    /// * `username` - Username whose normalized bucket should be cleared.
    pub(crate) fn clear_auth_attempts(&self, username: &str) {
        let mut limiter = self
            .auth_rate_limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limiter.clear_username(username);
    }
}

impl fmt::Debug for ApiState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiState")
            .field("storage_config", &self.storage_config)
            .field("secret_codec", &"[REDACTED]")
            .field(
                "are_session_cookies_secure",
                &self.are_session_cookies_secure,
            )
            .field("article_providers", &"[REGISTERED]")
            .finish_non_exhaustive()
    }
}

/// Failure reported by the bounded blocking executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockingTaskError {
    /// The executor was closed during server shutdown.
    Closed,
    /// The permit wait or blocking task exceeded its request deadline.
    TimedOut,
    /// The blocking task panicked or was cancelled by the runtime.
    Join,
}

impl fmt::Display for BlockingTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("blocking executor is closed"),
            Self::TimedOut => formatter.write_str("blocking operation timed out"),
            Self::Join => formatter.write_str("blocking operation failed to join"),
        }
    }
}

impl std::error::Error for BlockingTaskError {}

#[derive(Debug, Clone)]
struct BlockingExecutor {
    semaphore: Arc<Semaphore>,
    default_timeout: Duration,
}

impl BlockingExecutor {
    fn new(concurrency: usize, default_timeout: Duration) -> Self {
        assert!(concurrency > 0, "blocking concurrency must be positive");
        Self {
            semaphore: Arc::new(Semaphore::new(concurrency)),
            default_timeout,
        }
    }

    async fn run<Work, Output>(&self, work: Work) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        self.run_with_timeout(self.default_timeout, work).await
    }

    async fn run_with_timeout<Work, Output>(
        &self,
        timeout: Duration,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        tokio::time::timeout(timeout, self.run_without_timeout(work))
            .await
            .map_err(|_| BlockingTaskError::TimedOut)?
    }

    async fn run_without_timeout<Work, Output>(
        &self,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| BlockingTaskError::Closed)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| BlockingTaskError::Join)
    }

    fn close(&self) {
        self.semaphore.close();
    }
}

/// Authentication operation with an independent global request bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthAttemptKind {
    /// Login attempt.
    Login,
    /// Registration attempt.
    Register,
}

#[derive(Debug, Clone, Copy)]
struct AuthRateLimitConfig {
    username_attempt_limit: u32,
    username_window_seconds: u64,
    global_login_attempt_limit: u32,
    global_register_attempt_limit: u32,
    global_window_seconds: u64,
    tracked_username_limit: usize,
}

impl Default for AuthRateLimitConfig {
    fn default() -> Self {
        Self {
            username_attempt_limit: AUTH_USERNAME_ATTEMPT_LIMIT,
            username_window_seconds: AUTH_USERNAME_WINDOW_SECONDS,
            global_login_attempt_limit: AUTH_GLOBAL_LOGIN_ATTEMPT_LIMIT,
            global_register_attempt_limit: AUTH_GLOBAL_REGISTER_ATTEMPT_LIMIT,
            global_window_seconds: AUTH_GLOBAL_WINDOW_SECONDS,
            tracked_username_limit: AUTH_TRACKED_USERNAME_LIMIT,
        }
    }
}

#[derive(Debug)]
struct AuthRateLimiter {
    config: AuthRateLimitConfig,
    login_attempts: AttemptWindow,
    register_attempts: AttemptWindow,
    username_attempts: BTreeMap<String, AttemptWindow>,
}

impl AuthRateLimiter {
    fn new(config: AuthRateLimitConfig) -> Self {
        Self {
            config,
            login_attempts: AttemptWindow::default(),
            register_attempts: AttemptWindow::default(),
            username_attempts: BTreeMap::new(),
        }
    }

    fn check(&mut self, kind: AuthAttemptKind, username: &str, now: u64) -> Result<(), u64> {
        let global_limit = match kind {
            AuthAttemptKind::Login => self.config.global_login_attempt_limit,
            AuthAttemptKind::Register => self.config.global_register_attempt_limit,
        };
        let global_attempts = match kind {
            AuthAttemptKind::Login => &mut self.login_attempts,
            AuthAttemptKind::Register => &mut self.register_attempts,
        };
        global_attempts.try_acquire(now, global_limit, self.config.global_window_seconds)?;

        self.prune_usernames(now);
        let normalized_username = normalize_username(username);
        if !self.username_attempts.contains_key(&normalized_username)
            && self.username_attempts.len() >= self.config.tracked_username_limit
        {
            self.evict_oldest_username();
        }
        self.username_attempts
            .entry(normalized_username)
            .or_default()
            .try_acquire(
                now,
                self.config.username_attempt_limit,
                self.config.username_window_seconds,
            )
    }

    fn clear_username(&mut self, username: &str) {
        self.username_attempts.remove(&normalize_username(username));
    }

    fn prune_usernames(&mut self, now: u64) {
        let window_seconds = self.config.username_window_seconds;
        self.username_attempts.retain(|_, attempts| {
            attempts.count > 0 && now.saturating_sub(attempts.started_at) < window_seconds
        });
    }

    fn evict_oldest_username(&mut self) {
        let oldest = self
            .username_attempts
            .iter()
            .min_by(|(left_key, left), (right_key, right)| {
                left.started_at
                    .cmp(&right.started_at)
                    .then_with(|| left_key.cmp(right_key))
            })
            .map(|(username, _)| username.clone());
        if let Some(username) = oldest {
            self.username_attempts.remove(&username);
        }
    }
}

#[derive(Debug, Default)]
struct AttemptWindow {
    started_at: u64,
    count: u32,
}

impl AttemptWindow {
    fn try_acquire(&mut self, now: u64, limit: u32, window_seconds: u64) -> Result<(), u64> {
        let elapsed = now.saturating_sub(self.started_at);
        if self.count == 0 || elapsed >= window_seconds {
            self.started_at = now;
            self.count = 0;
        }
        if self.count >= limit {
            return Err(window_seconds
                .saturating_sub(now.saturating_sub(self.started_at))
                .max(1));
        }
        self.count += 1;
        Ok(())
    }
}

fn normalize_username(username: &str) -> String {
    username.trim().to_ascii_lowercase()
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs()
}

#[cfg(test)]
/// Shared structured-log capture helpers for API module tests.
pub(crate) mod tracing_test_support {
    use std::future::Future;
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, Once, OnceLock};

    use serde_json::Value;
    use tracing::Instrument;
    use tracing_subscriber::fmt::MakeWriter;

    static CAPTURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static CAPTURE_BYTES: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    static CAPTURE_SUBSCRIBER: Once = Once::new();
    static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);

    /// Thread-safe byte buffer used as a tracing test writer.
    #[derive(Clone)]
    pub(crate) struct CapturedLogs {
        bytes: Arc<Mutex<Vec<u8>>>,
        capture_id: u64,
    }

    impl Default for CapturedLogs {
        fn default() -> Self {
            let bytes = Arc::clone(CAPTURE_BYTES.get_or_init(|| Arc::new(Mutex::new(Vec::new()))));
            CAPTURE_SUBSCRIBER.call_once(|| {
                let subscriber = tracing_subscriber::fmt()
                    .with_ansi(false)
                    .with_max_level(tracing::Level::TRACE)
                    .with_writer(CapturedSink {
                        bytes: Arc::clone(&bytes),
                    })
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .finish();
                tracing::subscriber::set_global_default(subscriber)
                    .expect("API tests should install one global tracing subscriber");
            });
            Self {
                bytes,
                capture_id: NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed),
            }
        }
    }

    impl CapturedLogs {
        /// Run an asynchronous operation inside a uniquely identifiable capture span.
        ///
        /// # Arguments
        ///
        /// * `future` - Future whose structured events should be captured.
        ///
        /// # Returns
        ///
        /// Future output after structured event capture.
        pub(crate) async fn capture_async<CapturedFuture>(
            &self,
            future: CapturedFuture,
        ) -> CapturedFuture::Output
        where
            CapturedFuture: Future,
        {
            let _capture_guard = CAPTURE_LOCK.lock().await;
            let capture_span = tracing::info_span!(
                "test.capture",
                component = "test",
                capture_id = self.capture_id,
            );
            future.instrument(capture_span).await
        }

        /// Return all captured bytes as UTF-8 text.
        ///
        /// # Returns
        ///
        /// Captured JSON Lines text.
        pub(crate) fn text(&self) -> String {
            self.events()
                .into_iter()
                .map(|event| serde_json::to_string(&event).expect("event should serialize"))
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// Parse captured JSON Lines into event values.
        ///
        /// # Returns
        ///
        /// Parsed event objects in emission order.
        pub(crate) fn events(&self) -> Vec<Value> {
            let text = String::from_utf8(
                self.bytes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            )
            .expect("captured logs should be UTF-8");
            text.lines()
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_str(line).expect("captured log should be JSON"))
                .filter(|event: &Value| {
                    event["spans"].as_array().is_some_and(|spans| {
                        spans
                            .iter()
                            .any(|span| span["capture_id"].as_u64() == Some(self.capture_id))
                    })
                })
                .collect()
        }
    }

    #[derive(Clone)]
    struct CapturedSink {
        bytes: Arc<Mutex<Vec<u8>>>,
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

    impl<'writer> MakeWriter<'writer> for CapturedSink {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use litradar_storage::{SecretCodec, StorageConfig};
    use tower::ServiceExt;
    use tracing::Instrument;

    use super::tracing_test_support::CapturedLogs;
    use super::{
        ApiState, AuthAttemptKind, AuthRateLimitConfig, AuthRateLimiter, BlockingExecutor,
        BlockingTaskError,
    };

    fn test_config() -> AuthRateLimitConfig {
        AuthRateLimitConfig {
            username_attempt_limit: 2,
            username_window_seconds: 10,
            global_login_attempt_limit: 3,
            global_register_attempt_limit: 2,
            global_window_seconds: 5,
            tracked_username_limit: 2,
        }
    }

    #[test]
    fn auth_rate_limit_normalizes_usernames_and_returns_retry_delay() {
        let mut limiter = AuthRateLimiter::new(test_config());

        assert_eq!(
            limiter.check(AuthAttemptKind::Login, " Alice ", 100),
            Ok(())
        );
        assert_eq!(limiter.check(AuthAttemptKind::Login, "alice", 101), Ok(()));
        assert_eq!(limiter.check(AuthAttemptKind::Login, "ALICE", 102), Err(8));

        limiter.clear_username("ALIce");
        assert_eq!(limiter.check(AuthAttemptKind::Login, "alice", 106), Ok(()));
    }

    #[test]
    fn auth_rate_limit_separates_global_buckets_and_prunes_bounded_keys() {
        let mut limiter = AuthRateLimiter::new(test_config());

        assert_eq!(
            limiter.check(AuthAttemptKind::Register, "alpha", 10),
            Ok(())
        );
        assert_eq!(limiter.check(AuthAttemptKind::Register, "beta", 10), Ok(()));
        assert_eq!(
            limiter.check(AuthAttemptKind::Register, "gamma", 10),
            Err(5)
        );
        assert_eq!(limiter.check(AuthAttemptKind::Login, "gamma", 10), Ok(()));
        assert!(limiter.username_attempts.len() <= 2);

        assert_eq!(
            limiter.check(AuthAttemptKind::Register, "gamma", 16),
            Ok(())
        );
        assert!(limiter.username_attempts.len() <= 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_executor_bounds_concurrency_and_keeps_runtime_responsive() {
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root("blocking-test-root"),
            SecretCodec::from_key([1_u8; 32]),
            false,
            1,
            Duration::from_millis(50),
        );
        let should_release = Arc::new(AtomicBool::new(false));
        let worker_release = Arc::clone(&should_release);
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let first_state = state.clone();
        let first = tokio::spawn(async move {
            first_state
                .run_blocking_with_timeout(Duration::from_secs(2), move || {
                    let _ = started_sender.send(());
                    while !worker_release.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    "released"
                })
                .await
        });
        started_receiver
            .await
            .expect("first blocking job should start");

        let queued_state = state.clone();
        let queued = tokio::spawn(async move { queued_state.run_blocking(|| "queued").await });
        let router = crate::routes::public_routes()
            .merge(crate::routes::health_routes())
            .with_state(state.clone());
        let health_result = tokio::time::timeout(
            Duration::from_millis(250),
            router.clone().oneshot(
                Request::get("/health/live")
                    .body(Body::empty())
                    .expect("request"),
            ),
        )
        .await;
        let saturated_result = tokio::time::timeout(
            Duration::from_millis(250),
            router.oneshot(
                Request::get("/announcements")
                    .body(Body::empty())
                    .expect("request"),
            ),
        )
        .await;
        let queued_result = queued.await.expect("queued future should join");

        should_release.store(true, Ordering::Release);
        assert_eq!(
            first.await.expect("first future should join"),
            Ok("released")
        );
        let health_response = health_result
            .expect("lightweight health request should remain responsive")
            .expect("health route should respond");
        assert_eq!(health_response.status(), StatusCode::OK);
        let saturated_response = saturated_result
            .expect("saturated storage request should honor its deadline")
            .expect("announcement route should respond");
        assert_eq!(saturated_response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let saturated_payload: serde_json::Value = serde_json::from_slice(
            &to_bytes(saturated_response.into_body(), 1_024)
                .await
                .expect("saturated response body should load"),
        )
        .expect("saturated response should be JSON");
        assert_eq!(
            saturated_payload["detail"],
            "Service temporarily unavailable"
        );
        assert_eq!(queued_result, Err(BlockingTaskError::TimedOut));
        assert_eq!(state.run_blocking(|| "available").await, Ok("available"));
    }

    #[tokio::test]
    async fn blocking_executor_close_rejects_new_work() {
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root("blocking-test-root"),
            SecretCodec::from_key([1_u8; 32]),
            false,
            1,
            Duration::from_secs(1),
        );

        state.close_blocking_executor();

        assert_eq!(
            state.run_blocking(|| "unused").await,
            Err(BlockingTaskError::Closed)
        );
    }

    #[tokio::test]
    async fn blocking_work_preserves_request_span_for_security_events() {
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root("blocking-security-test-root"),
            SecretCodec::from_key([1_u8; 32]),
            false,
            1,
            Duration::from_secs(1),
        );
        let logs = CapturedLogs::default();

        logs.capture_async(async {
            let request_span = tracing::info_span!(
                "http.request",
                component = "http",
                request_id = "request-security-blocking",
            );
            async {
                state
                    .run_blocking(|| {
                        tracing::info!(event = "security.blocking.test", component = "security",);
                    })
                    .await
                    .expect("blocking security work should complete");
            }
            .instrument(request_span)
            .await;
        })
        .await;

        let event = logs
            .events()
            .into_iter()
            .find(|event| event["event"] == "security.blocking.test")
            .expect("blocking security event should be captured");
        assert!(event["spans"].as_array().is_some_and(|spans| {
            spans
                .iter()
                .any(|span| span["request_id"] == "request-security-blocking")
        }));
    }

    #[test]
    fn blocking_executor_close_marks_the_semaphore_closed() {
        let executor = BlockingExecutor::new(1, Duration::from_secs(1));
        assert_eq!(executor.semaphore.available_permits(), 1);

        executor.close();

        assert!(executor.semaphore.is_closed());
    }
}
