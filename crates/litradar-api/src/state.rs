//! Shared Axum application state.

use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use litradar_provider::ProviderRegistry;
use litradar_storage::{
    AuthRateLimitPolicy, SecretCodec, StorageConfig, TokenBucketPolicy, TrustedProxyCidr,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const DEFAULT_BLOCKING_CONCURRENCY: usize = 8;
const DEFAULT_BLOCKING_QUEUE_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_KDF_CONCURRENCY: usize = 2;

/// State shared by API route handlers.
#[derive(Clone)]
pub struct ApiState {
    storage_config: StorageConfig,
    secret_codec: SecretCodec,
    are_session_cookies_secure: bool,
    auth_rate_limiter: Arc<Mutex<AuthRateLimiter>>,
    trusted_proxy_cidrs: Arc<[TrustedProxyCidr]>,
    blocking_executor: BlockingExecutor,
    kdf_executor: BlockingExecutor,
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
        Self::build(
            storage_config,
            secret_codec,
            are_session_cookies_secure,
            Vec::new(),
            AuthRateLimitPolicy::default(),
            DEFAULT_BLOCKING_CONCURRENCY,
            DEFAULT_BLOCKING_QUEUE_TIMEOUT,
        )
    }

    /// Build API state with startup-validated authentication network policy.
    ///
    /// # Arguments
    ///
    /// * `storage_config` - Data path configuration.
    /// * `secret_codec` - Deployment secret codec.
    /// * `are_session_cookies_secure` - Whether session cookies include Secure.
    /// * `trusted_proxy_cidrs` - Direct peer networks allowed to supply forwarding chains.
    /// * `auth_rate_limit_policy` - Process-local token-bucket policy.
    ///
    /// # Returns
    ///
    /// Shared API state using the validated authentication policy.
    pub(crate) fn new_with_auth_policy(
        storage_config: StorageConfig,
        secret_codec: SecretCodec,
        are_session_cookies_secure: bool,
        trusted_proxy_cidrs: Vec<TrustedProxyCidr>,
        auth_rate_limit_policy: AuthRateLimitPolicy,
    ) -> Self {
        Self::build(
            storage_config,
            secret_codec,
            are_session_cookies_secure,
            trusted_proxy_cidrs,
            auth_rate_limit_policy,
            DEFAULT_BLOCKING_CONCURRENCY,
            DEFAULT_BLOCKING_QUEUE_TIMEOUT,
        )
    }

    fn build(
        storage_config: StorageConfig,
        secret_codec: SecretCodec,
        are_session_cookies_secure: bool,
        trusted_proxy_cidrs: Vec<TrustedProxyCidr>,
        auth_rate_limit_policy: AuthRateLimitPolicy,
        blocking_concurrency: usize,
        blocking_queue_timeout: Duration,
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
            auth_rate_limiter: Arc::new(Mutex::new(AuthRateLimiter::new(auth_rate_limit_policy))),
            trusted_proxy_cidrs: trusted_proxy_cidrs.into(),
            blocking_executor: BlockingExecutor::new(blocking_concurrency, blocking_queue_timeout),
            kdf_executor: BlockingExecutor::new(DEFAULT_KDF_CONCURRENCY, blocking_queue_timeout),
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
    /// * `queue_timeout` - Default permit acquisition deadline.
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
        queue_timeout: Duration,
    ) -> Self {
        Self::build(
            storage_config,
            secret_codec,
            are_session_cookies_secure,
            Vec::new(),
            AuthRateLimitPolicy::default(),
            concurrency,
            queue_timeout,
        )
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

    /// Run one password KDF operation behind the dedicated concurrency-two gate.
    ///
    /// # Arguments
    ///
    /// * `work` - Owned synchronous password operation to execute.
    ///
    /// # Returns
    ///
    /// Completed output or a bounded-executor failure.
    pub(crate) async fn run_kdf_blocking<Work, Output>(
        &self,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let span = tracing::Span::current();
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        self.kdf_executor
            .run(move || tracing::dispatcher::with_default(&subscriber, || span.in_scope(work)))
            .await
    }

    /// Run synchronous work with an operation-specific queue deadline.
    ///
    /// # Arguments
    ///
    /// * `queue_timeout` - Maximum time spent waiting for an executor permit.
    /// * `work` - Owned synchronous operation to execute.
    ///
    /// # Returns
    ///
    /// Completed output or a bounded-executor failure.
    pub(crate) async fn run_blocking_with_queue_timeout<Work, Output>(
        &self,
        queue_timeout: Duration,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let span = tracing::Span::current();
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        self.blocking_executor
            .run_with_queue_timeout(queue_timeout, move || {
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
    #[cfg(test)]
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
            .run_without_queue_timeout(move || {
                tracing::dispatcher::with_default(&subscriber, || span.in_scope(work))
            })
            .await
    }

    /// Stop accepting queued blocking work during server shutdown.
    pub(crate) fn close_blocking_executor(&self) {
        self.blocking_executor.close();
        self.kdf_executor.close();
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
    /// * `peer_address` - Direct TCP peer address when connection metadata is available.
    /// * `headers` - Request headers containing an optional trusted forwarding chain.
    ///
    /// # Returns
    ///
    /// Empty result when allowed, or structured rejection metadata when limited.
    pub(crate) fn check_auth_attempt(
        &self,
        kind: AuthAttemptKind,
        username: &str,
        peer_address: Option<SocketAddr>,
        headers: &HeaderMap,
    ) -> Result<(), AuthRateLimitRejection> {
        let client_source =
            resolve_auth_client_source(peer_address, headers, self.trusted_proxy_cidrs.as_ref());
        let mut limiter = self
            .auth_rate_limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limiter.check(kind, client_source, username)
    }

    /// Clear the per-username failure bucket after successful authentication.
    ///
    /// # Arguments
    ///
    /// * `kind` - Authentication operation whose username bucket succeeded.
    /// * `username` - Username whose normalized bucket should be cleared.
    pub(crate) fn clear_auth_attempts(&self, kind: AuthAttemptKind, username: &str) {
        let mut limiter = self
            .auth_rate_limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        limiter.clear_username(kind, username);
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
            .field("trusted_proxy_count", &self.trusted_proxy_cidrs.len())
            .field("article_providers", &"[REGISTERED]")
            .finish_non_exhaustive()
    }
}

/// Failure reported by the bounded blocking executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockingTaskError {
    /// The executor was closed during server shutdown.
    Closed,
    /// The executor permit could not be acquired before the queue deadline.
    QueueTimedOut,
    /// The blocking task panicked or was cancelled by the runtime.
    Join,
}

impl fmt::Display for BlockingTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("blocking executor is closed"),
            Self::QueueTimedOut => formatter.write_str("blocking executor queue timed out"),
            Self::Join => formatter.write_str("blocking operation failed to join"),
        }
    }
}

impl std::error::Error for BlockingTaskError {}

#[derive(Debug, Clone)]
struct BlockingExecutor {
    semaphore: Arc<Semaphore>,
    default_queue_timeout: Duration,
}

impl BlockingExecutor {
    fn new(concurrency: usize, default_queue_timeout: Duration) -> Self {
        assert!(concurrency > 0, "blocking concurrency must be positive");
        Self {
            semaphore: Arc::new(Semaphore::new(concurrency)),
            default_queue_timeout,
        }
    }

    async fn run<Work, Output>(&self, work: Work) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        self.run_with_queue_timeout(self.default_queue_timeout, work)
            .await
    }

    async fn run_with_queue_timeout<Work, Output>(
        &self,
        queue_timeout: Duration,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        let permit =
            tokio::time::timeout(queue_timeout, Arc::clone(&self.semaphore).acquire_owned())
                .await
                .map_err(|_| BlockingTaskError::QueueTimedOut)?
                .map_err(|_| BlockingTaskError::Closed)?;
        Self::run_with_permit(permit, work).await
    }

    #[cfg(test)]
    async fn run_without_queue_timeout<Work, Output>(
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
        Self::run_with_permit(permit, work).await
    }

    async fn run_with_permit<Work, Output>(
        permit: OwnedSemaphorePermit,
        work: Work,
    ) -> Result<Output, BlockingTaskError>
    where
        Work: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
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

/// Authentication operation with independent keyed and global buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AuthAttemptKind {
    /// Login attempt.
    Login,
    /// Registration attempt.
    Register,
}

/// Structured authentication limiter rejection metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AuthRateLimitRejection {
    /// Retry delay returned in the HTTP response.
    pub(crate) retry_after_seconds: u64,
    /// Stable rejection classification.
    pub(crate) reason: &'static str,
    /// Bucket class that rejected the request.
    pub(crate) bucket: &'static str,
    /// Trust classification used to determine the client address.
    pub(crate) source_class: &'static str,
    /// Process-local count for this operation and bucket class.
    pub(crate) rejected_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AuthRateLimitBucket {
    ClientIp,
    Username,
    GlobalBreaker,
}

impl AuthRateLimitBucket {
    fn as_str(self) -> &'static str {
        match self {
            Self::ClientIp => "client_ip",
            Self::Username => "username",
            Self::GlobalBreaker => "global_breaker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthClientSource {
    address: IpAddr,
    class: AuthClientSourceClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthClientSourceClass {
    Direct,
    UntrustedForwardingHeader,
    TrustedForwardingChain,
    TrustedProxyWithoutHeader,
    TrustedProxyInvalidHeader,
    MissingPeer,
}

impl AuthClientSourceClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::UntrustedForwardingHeader => "untrusted_forwarding_header",
            Self::TrustedForwardingChain => "trusted_forwarding_chain",
            Self::TrustedProxyWithoutHeader => "trusted_proxy_without_header",
            Self::TrustedProxyInvalidHeader => "trusted_proxy_invalid_header",
            Self::MissingPeer => "missing_peer",
        }
    }
}

#[derive(Debug)]
struct AuthRateLimiter {
    policy: AuthRateLimitPolicy,
    started_at: Instant,
    ip_buckets: BTreeMap<(AuthAttemptKind, IpAddr), TrackedTokenBucket>,
    username_buckets: BTreeMap<(AuthAttemptKind, String), TrackedTokenBucket>,
    global_login: TokenBucket,
    global_register: TokenBucket,
    next_access_sequence: u64,
    rejection_counts: BTreeMap<(AuthAttemptKind, AuthRateLimitBucket), u64>,
}

impl AuthRateLimiter {
    fn new(policy: AuthRateLimitPolicy) -> Self {
        Self {
            policy,
            started_at: Instant::now(),
            ip_buckets: BTreeMap::new(),
            username_buckets: BTreeMap::new(),
            global_login: TokenBucket::full(policy.global_login, 0),
            global_register: TokenBucket::full(policy.global_register, 0),
            next_access_sequence: 0,
            rejection_counts: BTreeMap::new(),
        }
    }

    fn check(
        &mut self,
        kind: AuthAttemptKind,
        client_source: AuthClientSource,
        username: &str,
    ) -> Result<(), AuthRateLimitRejection> {
        let now = self.started_at.elapsed().as_secs();
        self.check_at(kind, client_source, username, now)
    }

    fn check_at(
        &mut self,
        kind: AuthAttemptKind,
        client_source: AuthClientSource,
        username: &str,
        now: u64,
    ) -> Result<(), AuthRateLimitRejection> {
        let access_sequence = self.next_sequence();
        let ip_policy = match kind {
            AuthAttemptKind::Login => self.policy.login_ip,
            AuthAttemptKind::Register => self.policy.register_ip,
        };
        if let Err(retry_after_seconds) = try_acquire_tracked_bucket(
            &mut self.ip_buckets,
            (kind, client_source.address),
            self.policy.ip_key_limit,
            ip_policy,
            now,
            access_sequence,
        ) {
            return Err(self.rejection(
                kind,
                AuthRateLimitBucket::ClientIp,
                client_source.class,
                retry_after_seconds,
            ));
        }

        let normalized_username = normalize_username(username);
        if let Err(retry_after_seconds) = try_acquire_tracked_bucket(
            &mut self.username_buckets,
            (kind, normalized_username),
            self.policy.username_key_limit,
            self.policy.username,
            now,
            access_sequence,
        ) {
            return Err(self.rejection(
                kind,
                AuthRateLimitBucket::Username,
                client_source.class,
                retry_after_seconds,
            ));
        }

        let global_result = match kind {
            AuthAttemptKind::Login => self.global_login.try_acquire(now, self.policy.global_login),
            AuthAttemptKind::Register => self
                .global_register
                .try_acquire(now, self.policy.global_register),
        };
        if let Err(retry_after_seconds) = global_result {
            return Err(self.rejection(
                kind,
                AuthRateLimitBucket::GlobalBreaker,
                client_source.class,
                retry_after_seconds,
            ));
        }
        Ok(())
    }

    fn clear_username(&mut self, kind: AuthAttemptKind, username: &str) {
        self.username_buckets
            .remove(&(kind, normalize_username(username)));
    }

    fn next_sequence(&mut self) -> u64 {
        self.next_access_sequence = self.next_access_sequence.saturating_add(1);
        self.next_access_sequence
    }

    fn rejection(
        &mut self,
        kind: AuthAttemptKind,
        bucket: AuthRateLimitBucket,
        source_class: AuthClientSourceClass,
        retry_after_seconds: u64,
    ) -> AuthRateLimitRejection {
        let rejected_count = self.rejection_counts.entry((kind, bucket)).or_default();
        *rejected_count = rejected_count.saturating_add(1);
        AuthRateLimitRejection {
            retry_after_seconds,
            reason: "rate_limit_exceeded",
            bucket: bucket.as_str(),
            source_class: source_class.as_str(),
            rejected_count: *rejected_count,
        }
    }
}

#[derive(Debug)]
struct TrackedTokenBucket {
    bucket: TokenBucket,
    last_used_sequence: u64,
}

impl TrackedTokenBucket {
    fn new(policy: TokenBucketPolicy, now: u64, last_used_sequence: u64) -> Self {
        Self {
            bucket: TokenBucket::full(policy, now),
            last_used_sequence,
        }
    }
}

#[derive(Debug)]
struct TokenBucket {
    available_units: u128,
    last_refill_at: u64,
}

impl TokenBucket {
    fn full(policy: TokenBucketPolicy, now: u64) -> Self {
        Self {
            available_units: bucket_capacity_units(policy),
            last_refill_at: now,
        }
    }

    fn try_acquire(&mut self, now: u64, policy: TokenBucketPolicy) -> Result<(), u64> {
        if now > self.last_refill_at {
            let elapsed = now - self.last_refill_at;
            let restored = u128::from(elapsed) * u128::from(policy.refill_tokens);
            self.available_units = self
                .available_units
                .saturating_add(restored)
                .min(bucket_capacity_units(policy));
            self.last_refill_at = now;
        }
        let request_units = u128::from(policy.refill_seconds);
        if self.available_units >= request_units {
            self.available_units -= request_units;
            return Ok(());
        }
        let missing_units = request_units - self.available_units;
        let refill_tokens = u128::from(policy.refill_tokens);
        let retry_after_seconds = missing_units.div_ceil(refill_tokens).max(1);
        Err(u64::try_from(retry_after_seconds).unwrap_or(u64::MAX))
    }
}

fn bucket_capacity_units(policy: TokenBucketPolicy) -> u128 {
    u128::from(policy.capacity) * u128::from(policy.refill_seconds)
}

fn try_acquire_tracked_bucket<Key>(
    buckets: &mut BTreeMap<Key, TrackedTokenBucket>,
    key: Key,
    key_limit: usize,
    policy: TokenBucketPolicy,
    now: u64,
    access_sequence: u64,
) -> Result<(), u64>
where
    Key: Clone + Ord,
{
    if !buckets.contains_key(&key) && buckets.len() >= key_limit {
        evict_least_recently_used(buckets);
    }
    let tracked = buckets
        .entry(key)
        .or_insert_with(|| TrackedTokenBucket::new(policy, now, access_sequence));
    tracked.last_used_sequence = access_sequence;
    tracked.bucket.try_acquire(now, policy)
}

fn evict_least_recently_used<Key>(buckets: &mut BTreeMap<Key, TrackedTokenBucket>)
where
    Key: Clone + Ord,
{
    let oldest = buckets
        .iter()
        .min_by(|(left_key, left), (right_key, right)| {
            left.last_used_sequence
                .cmp(&right.last_used_sequence)
                .then_with(|| left_key.cmp(right_key))
        })
        .map(|(key, _)| key.clone());
    if let Some(key) = oldest {
        buckets.remove(&key);
    }
}

const MAX_TRACKED_USERNAME_CHARS: usize = 32;
const FORWARDED_HEADER: &str = "forwarded";
const X_FORWARDED_FOR_HEADER: &str = "x-forwarded-for";

fn normalize_username(username: &str) -> String {
    username
        .trim()
        .chars()
        .take(MAX_TRACKED_USERNAME_CHARS)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn resolve_auth_client_source(
    peer_address: Option<SocketAddr>,
    headers: &HeaderMap,
    trusted_proxy_cidrs: &[TrustedProxyCidr],
) -> AuthClientSource {
    let Some(peer_address) = peer_address else {
        return AuthClientSource {
            address: IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            class: AuthClientSourceClass::MissingPeer,
        };
    };
    let peer_ip = canonical_client_ip(peer_address.ip());
    let has_forwarding_header =
        headers.contains_key(FORWARDED_HEADER) || headers.contains_key(X_FORWARDED_FOR_HEADER);
    if !is_trusted_proxy(peer_ip, trusted_proxy_cidrs) {
        return AuthClientSource {
            address: peer_ip,
            class: if has_forwarding_header {
                AuthClientSourceClass::UntrustedForwardingHeader
            } else {
                AuthClientSourceClass::Direct
            },
        };
    }

    match parse_forwarding_chain(headers) {
        ForwardingChain::Absent => AuthClientSource {
            address: peer_ip,
            class: AuthClientSourceClass::TrustedProxyWithoutHeader,
        },
        ForwardingChain::Invalid => AuthClientSource {
            address: peer_ip,
            class: AuthClientSourceClass::TrustedProxyInvalidHeader,
        },
        ForwardingChain::Valid(chain) => {
            let mut client_ip = peer_ip;
            for forwarded_ip in chain.into_iter().rev() {
                if !is_trusted_proxy(client_ip, trusted_proxy_cidrs) {
                    break;
                }
                client_ip = canonical_client_ip(forwarded_ip);
            }
            AuthClientSource {
                address: client_ip,
                class: AuthClientSourceClass::TrustedForwardingChain,
            }
        }
    }
}

fn is_trusted_proxy(address: IpAddr, trusted_proxy_cidrs: &[TrustedProxyCidr]) -> bool {
    trusted_proxy_cidrs
        .iter()
        .any(|network| network.contains(address))
}

fn canonical_client_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(address)),
        address => address,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ForwardingChain {
    Absent,
    Invalid,
    Valid(Vec<IpAddr>),
}

fn parse_forwarding_chain(headers: &HeaderMap) -> ForwardingChain {
    if headers.get_all(FORWARDED_HEADER).iter().next().is_some() {
        return parse_forwarded_header(headers);
    }
    if headers
        .get_all(X_FORWARDED_FOR_HEADER)
        .iter()
        .next()
        .is_some()
    {
        return parse_x_forwarded_for_header(headers);
    }
    ForwardingChain::Absent
}

fn parse_forwarded_header(headers: &HeaderMap) -> ForwardingChain {
    let mut chain = Vec::new();
    for value in headers.get_all(FORWARDED_HEADER) {
        let Ok(value) = value.to_str() else {
            return ForwardingChain::Invalid;
        };
        for element in value.split(',') {
            let mut forwarded_for = None;
            for parameter in element.split(';') {
                let Some((name, value)) = parameter.trim().split_once('=') else {
                    return ForwardingChain::Invalid;
                };
                if name.trim().eq_ignore_ascii_case("for") {
                    if forwarded_for.is_some() {
                        return ForwardingChain::Invalid;
                    }
                    forwarded_for = parse_forwarded_node(value.trim());
                    if forwarded_for.is_none() {
                        return ForwardingChain::Invalid;
                    }
                }
            }
            let Some(forwarded_for) = forwarded_for else {
                return ForwardingChain::Invalid;
            };
            chain.push(forwarded_for);
        }
    }
    if chain.is_empty() {
        ForwardingChain::Invalid
    } else {
        ForwardingChain::Valid(chain)
    }
}

fn parse_x_forwarded_for_header(headers: &HeaderMap) -> ForwardingChain {
    let mut chain = Vec::new();
    for value in headers.get_all(X_FORWARDED_FOR_HEADER) {
        let Ok(value) = value.to_str() else {
            return ForwardingChain::Invalid;
        };
        for entry in value.split(',') {
            let Some(address) = parse_forwarded_node(entry.trim()) else {
                return ForwardingChain::Invalid;
            };
            chain.push(address);
        }
    }
    if chain.is_empty() {
        ForwardingChain::Invalid
    } else {
        ForwardingChain::Valid(chain)
    }
}

fn parse_forwarded_node(value: &str) -> Option<IpAddr> {
    let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        let value = &value[1..value.len() - 1];
        if value.contains(['"', '\\']) {
            return None;
        }
        value
    } else if value.contains('"') {
        return None;
    } else {
        value
    };
    value
        .parse::<IpAddr>()
        .ok()
        .or_else(|| value.parse::<SocketAddr>().ok().map(|address| address.ip()))
        .or_else(|| {
            value
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .and_then(|value| value.parse::<IpAddr>().ok())
        })
        .map(canonical_client_ip)
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
    use std::net::{IpAddr, SocketAddr};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::{to_bytes, Body};
    use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
    use litradar_storage::{
        parse_runtime_setting, AuthRateLimitPolicy, ParsedRuntimeSettingValue, RuntimeSettingKey,
        SecretCodec, StorageConfig, TokenBucketPolicy, TrustedProxyCidr,
    };
    use tempfile::tempdir;
    use tower::ServiceExt;
    use tracing::Instrument;

    use super::tracing_test_support::CapturedLogs;
    use super::{
        resolve_auth_client_source, ApiState, AuthAttemptKind, AuthClientSource,
        AuthClientSourceClass, AuthRateLimiter, BlockingExecutor, BlockingTaskError,
    };

    fn bucket(capacity: u32, refill_tokens: u32, refill_seconds: u64) -> TokenBucketPolicy {
        TokenBucketPolicy {
            capacity,
            refill_tokens,
            refill_seconds,
        }
    }

    fn test_policy() -> AuthRateLimitPolicy {
        AuthRateLimitPolicy {
            login_ip: bucket(20, 1, 10),
            username: bucket(2, 1, 10),
            register_ip: bucket(20, 1, 10),
            global_login: bucket(20, 1, 10),
            global_register: bucket(20, 1, 10),
            ip_key_limit: 2,
            username_key_limit: 2,
        }
    }

    fn direct_source(address: &str) -> AuthClientSource {
        AuthClientSource {
            address: address.parse().expect("client IP should parse"),
            class: AuthClientSourceClass::Direct,
        }
    }

    fn trusted_proxy_cidrs(value: &str) -> Vec<TrustedProxyCidr> {
        match parse_runtime_setting(RuntimeSettingKey::TrustedProxyCidrs, value)
            .expect("trusted proxy CIDRs should parse")
        {
            ParsedRuntimeSettingValue::TrustedProxyCidrs(values) => values,
            _ => panic!("trusted proxy setting should use the typed CIDR parser"),
        }
    }

    #[test]
    fn auth_rate_limit_normalizes_usernames_and_returns_retry_delay() {
        let mut limiter = AuthRateLimiter::new(test_policy());
        let source = direct_source("192.0.2.10");

        assert_eq!(
            limiter.check_at(AuthAttemptKind::Login, source, " Alice ", 100),
            Ok(())
        );
        assert_eq!(
            limiter.check_at(AuthAttemptKind::Login, source, "alice", 101),
            Ok(())
        );
        let rejection = limiter
            .check_at(AuthAttemptKind::Login, source, "ALICE", 102)
            .expect_err("normalized username bucket should reject");
        assert_eq!(rejection.retry_after_seconds, 8);
        assert_eq!(rejection.reason, "rate_limit_exceeded");
        assert_eq!(rejection.bucket, "username");
        assert_eq!(rejection.source_class, "direct");
        assert_eq!(rejection.rejected_count, 1);

        limiter.clear_username(AuthAttemptKind::Register, "ALIce");
        assert_eq!(
            limiter
                .check_at(AuthAttemptKind::Login, source, "alice", 106)
                .expect_err("clearing registration must not clear login failures")
                .bucket,
            "username"
        );
        limiter.clear_username(AuthAttemptKind::Login, "ALIce");
        assert_eq!(
            limiter.check_at(AuthAttemptKind::Login, source, "alice", 106),
            Ok(())
        );
    }

    #[test]
    fn auth_rate_limit_front_rejections_do_not_consume_later_buckets() {
        let mut policy = test_policy();
        policy.username = bucket(1, 1, 60);
        policy.global_login = bucket(2, 1, 60);
        let mut limiter = AuthRateLimiter::new(policy);

        assert_eq!(
            limiter.check_at(
                AuthAttemptKind::Login,
                direct_source("192.0.2.1"),
                "alpha",
                10
            ),
            Ok(())
        );
        assert_eq!(
            limiter
                .check_at(
                    AuthAttemptKind::Login,
                    direct_source("192.0.2.1"),
                    "alpha",
                    10
                )
                .expect_err("username bucket should reject")
                .bucket,
            "username"
        );
        assert_eq!(
            limiter.check_at(
                AuthAttemptKind::Login,
                direct_source("192.0.2.2"),
                "beta",
                10
            ),
            Ok(())
        );
        assert_eq!(
            limiter
                .check_at(
                    AuthAttemptKind::Login,
                    direct_source("192.0.2.3"),
                    "gamma",
                    10
                )
                .expect_err("global breaker should now reject")
                .bucket,
            "global_breaker"
        );

        let mut policy = test_policy();
        policy.login_ip = bucket(1, 1, 60);
        policy.username = bucket(10, 1, 60);
        policy.global_login = bucket(2, 1, 60);
        let mut limiter = AuthRateLimiter::new(policy);
        assert_eq!(
            limiter.check_at(
                AuthAttemptKind::Login,
                direct_source("198.51.100.1"),
                "alpha",
                20
            ),
            Ok(())
        );
        assert_eq!(
            limiter
                .check_at(
                    AuthAttemptKind::Login,
                    direct_source("198.51.100.1"),
                    "beta",
                    20
                )
                .expect_err("IP bucket should reject")
                .bucket,
            "client_ip"
        );
        assert!(!limiter
            .username_buckets
            .contains_key(&(AuthAttemptKind::Login, "beta".to_string())));
        assert_eq!(
            limiter.check_at(
                AuthAttemptKind::Login,
                direct_source("198.51.100.2"),
                "beta",
                20
            ),
            Ok(())
        );
    }

    #[test]
    fn auth_rate_limit_lru_maps_remain_bounded_under_rotating_inputs() {
        let mut limiter = AuthRateLimiter::new(test_policy());
        for (address, username) in [
            ("192.0.2.1", "alpha"),
            ("192.0.2.2", "beta"),
            ("192.0.2.1", "alpha"),
            ("192.0.2.3", "gamma"),
        ] {
            limiter
                .check_at(AuthAttemptKind::Login, direct_source(address), username, 10)
                .expect("bounded fixture request should be accepted");
        }

        assert_eq!(limiter.ip_buckets.len(), 2);
        assert!(limiter.ip_buckets.contains_key(&(
            AuthAttemptKind::Login,
            "192.0.2.1".parse::<IpAddr>().expect("IP should parse")
        )));
        assert!(limiter.ip_buckets.contains_key(&(
            AuthAttemptKind::Login,
            "192.0.2.3".parse::<IpAddr>().expect("IP should parse")
        )));
        assert_eq!(limiter.username_buckets.len(), 2);
        assert!(limiter
            .username_buckets
            .contains_key(&(AuthAttemptKind::Login, "alpha".to_string())));
        assert!(limiter
            .username_buckets
            .contains_key(&(AuthAttemptKind::Login, "gamma".to_string())));

        for index in 0..100 {
            let address = SocketAddr::from(([203, 0, 113, index], 443));
            let username = format!("{}-{index}", "密".repeat(1_000));
            let _ = limiter.check_at(
                AuthAttemptKind::Register,
                AuthClientSource {
                    address: address.ip(),
                    class: AuthClientSourceClass::Direct,
                },
                &username,
                20,
            );
        }
        assert!(limiter.ip_buckets.len() <= 2);
        assert!(limiter.username_buckets.len() <= 2);
        assert!(limiter
            .username_buckets
            .keys()
            .all(|(_, username)| username.chars().count() <= 32));
    }

    #[test]
    fn auth_rate_limit_trusted_proxy_chain_uses_first_untrusted_hop_from_right() {
        let trusted = trusted_proxy_cidrs("10.0.0.0/8,2001:db8::/32");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.0.2.10, 198.51.100.5, 10.0.0.1"),
        );

        let untrusted = resolve_auth_client_source(
            Some("203.0.113.9:443".parse().expect("peer should parse")),
            &headers,
            &trusted,
        );
        assert_eq!(untrusted.address, "203.0.113.9".parse::<IpAddr>().unwrap());
        assert_eq!(
            untrusted.class,
            AuthClientSourceClass::UntrustedForwardingHeader
        );
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.200"));
        assert_eq!(
            resolve_auth_client_source(
                Some("203.0.113.9:443".parse().expect("peer should parse")),
                &headers,
                &trusted,
            ),
            untrusted
        );
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.0.2.10, 198.51.100.5, 10.0.0.1"),
        );

        let forwarded = resolve_auth_client_source(
            Some("10.0.0.2:443".parse().expect("peer should parse")),
            &headers,
            &trusted,
        );
        assert_eq!(forwarded.address, "198.51.100.5".parse::<IpAddr>().unwrap());
        assert_eq!(
            forwarded.class,
            AuthClientSourceClass::TrustedForwardingChain
        );

        headers.insert(
            "forwarded",
            HeaderValue::from_static("for=203.0.113.7;proto=https, for=\"10.0.0.1:443\""),
        );
        let standard = resolve_auth_client_source(
            Some("10.0.0.2:443".parse().expect("peer should parse")),
            &headers,
            &trusted,
        );
        assert_eq!(standard.address, "203.0.113.7".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn auth_rate_limit_invalid_or_missing_peer_metadata_uses_shared_safe_sources() {
        let trusted = trusted_proxy_cidrs("10.0.0.0/8");
        let mut headers = HeaderMap::new();
        headers.insert("forwarded", HeaderValue::from_static("for=unknown"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.10"));
        let invalid = resolve_auth_client_source(
            Some("10.0.0.2:443".parse().expect("peer should parse")),
            &headers,
            &trusted,
        );
        assert_eq!(invalid.address, "10.0.0.2".parse::<IpAddr>().unwrap());
        assert_eq!(
            invalid.class,
            AuthClientSourceClass::TrustedProxyInvalidHeader
        );

        let missing = resolve_auth_client_source(None, &headers, &trusted);
        assert_eq!(missing.address, "::".parse::<IpAddr>().unwrap());
        assert_eq!(missing.class, AuthClientSourceClass::MissingPeer);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_executor_bounds_concurrency_and_keeps_runtime_responsive() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root(temp_dir.path()),
            SecretCodec::from_key([1_u8; 32]),
            false,
            1,
            Duration::from_millis(50),
        );
        let should_release = Arc::new(AtomicBool::new(false));
        let queued_started = Arc::new(AtomicBool::new(false));
        let worker_release = Arc::clone(&should_release);
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let first_state = state.clone();
        let first = tokio::spawn(async move {
            first_state
                .run_blocking_with_queue_timeout(Duration::from_secs(2), move || {
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
        let queued_started_in_work = Arc::clone(&queued_started);
        let queued = tokio::spawn(async move {
            queued_state
                .run_blocking(move || {
                    queued_started_in_work.store(true, Ordering::Release);
                    "queued"
                })
                .await
        });
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
        assert_eq!(queued_result, Err(BlockingTaskError::QueueTimedOut));
        assert!(!queued_started.load(Ordering::Acquire));
        assert_eq!(state.run_blocking(|| "available").await, Ok("available"));
        assert!(!queued_started.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_executor_does_not_return_timeout_after_http_work_starts() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root(temp_dir.path()),
            SecretCodec::from_key([1_u8; 32]),
            false,
            1,
            Duration::from_millis(25),
        );
        let should_release = Arc::new(AtomicBool::new(false));
        let did_mutate = Arc::new(AtomicBool::new(false));
        let did_start = Arc::new(tokio::sync::Notify::new());
        let route_state = state.clone();
        let route_release = Arc::clone(&should_release);
        let route_mutation = Arc::clone(&did_mutate);
        let route_started = Arc::clone(&did_start);
        let router = axum::Router::new().route(
            "/started",
            axum::routing::get(move || {
                let state = route_state.clone();
                let worker_release = Arc::clone(&route_release);
                let worker_mutation = Arc::clone(&route_mutation);
                let worker_started = Arc::clone(&route_started);
                async move {
                    match state
                        .run_blocking_with_queue_timeout(Duration::from_millis(25), move || {
                            worker_started.notify_one();
                            while !worker_release.load(Ordering::Acquire) {
                                std::thread::yield_now();
                            }
                            worker_mutation.store(true, Ordering::Release);
                        })
                        .await
                    {
                        Ok(()) => StatusCode::NO_CONTENT,
                        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
                    }
                }
            }),
        );
        let response_task = tokio::spawn(
            router.oneshot(
                Request::get("/started")
                    .body(Body::empty())
                    .expect("request"),
            ),
        );
        did_start.notified().await;
        tokio::time::sleep(Duration::from_millis(75)).await;

        assert!(!response_task.is_finished());
        assert!(!did_mutate.load(Ordering::Acquire));
        assert_eq!(state.blocking_executor.semaphore.available_permits(), 0);

        should_release.store(true, Ordering::Release);
        let response = response_task
            .await
            .expect("HTTP task should join")
            .expect("HTTP route should respond");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(did_mutate.load(Ordering::Acquire));
        assert_eq!(state.blocking_executor.semaphore.available_permits(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_executor_holds_permit_until_cancelled_waiter_work_finishes() {
        let executor = BlockingExecutor::new(1, Duration::from_millis(25));
        let cancelled_executor = executor.clone();
        let should_release = Arc::new(AtomicBool::new(false));
        let worker_release = Arc::clone(&should_release);
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let (finished_sender, finished_receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            cancelled_executor
                .run(move || {
                    let _ = started_sender.send(());
                    while !worker_release.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    let _ = finished_sender.send(());
                })
                .await
        });
        started_receiver
            .await
            .expect("blocking work should acquire a permit and start");

        task.abort();
        assert!(task
            .await
            .expect_err("waiter should be cancelled")
            .is_cancelled());
        assert_eq!(
            executor
                .run_with_queue_timeout(Duration::from_millis(25), || "queued")
                .await,
            Err(BlockingTaskError::QueueTimedOut)
        );

        should_release.store(true, Ordering::Release);
        finished_receiver
            .await
            .expect("cancelled waiter's blocking work should finish");
        assert_eq!(executor.run(|| "available").await, Ok("available"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn kdf_executor_allows_at_most_two_concurrent_operations() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root(temp_dir.path()),
            SecretCodec::from_key([1_u8; 32]),
            false,
            8,
            Duration::from_secs(2),
        );
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let should_release = Arc::new(AtomicBool::new(false));
        let handles = (0..3)
            .map(|_| {
                let state = state.clone();
                let active = Arc::clone(&active);
                let maximum = Arc::clone(&maximum);
                let started = Arc::clone(&started);
                let should_release = Arc::clone(&should_release);
                tokio::spawn(async move {
                    state
                        .run_kdf_blocking(move || {
                            let active_count = active.fetch_add(1, Ordering::AcqRel) + 1;
                            maximum.fetch_max(active_count, Ordering::AcqRel);
                            started.fetch_add(1, Ordering::Release);
                            while !should_release.load(Ordering::Acquire) {
                                std::thread::yield_now();
                            }
                            active.fetch_sub(1, Ordering::AcqRel);
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();

        tokio::time::timeout(Duration::from_secs(1), async {
            while started.load(Ordering::Acquire) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("two KDF operations should start");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(started.load(Ordering::Acquire), 2);
        assert_eq!(maximum.load(Ordering::Acquire), 2);

        should_release.store(true, Ordering::Release);
        for handle in handles {
            assert_eq!(handle.await.expect("KDF task should join"), Ok(()));
        }
        assert_eq!(maximum.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn blocking_executor_close_rejects_new_work() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root(temp_dir.path()),
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
        assert_eq!(
            state.run_kdf_blocking(|| "unused").await,
            Err(BlockingTaskError::Closed)
        );
    }

    #[tokio::test]
    async fn blocking_work_preserves_request_span_for_security_events() {
        let temp_dir = tempdir().expect("temporary directory should be created");
        let state = ApiState::new_with_blocking_limits(
            StorageConfig::from_project_root(temp_dir.path()),
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
