//! Shared Axum application state.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ps_storage::StorageConfig;

const AUTH_USERNAME_ATTEMPT_LIMIT: u32 = 5;
const AUTH_USERNAME_WINDOW_SECONDS: u64 = 5 * 60;
const AUTH_GLOBAL_LOGIN_ATTEMPT_LIMIT: u32 = 100;
const AUTH_GLOBAL_REGISTER_ATTEMPT_LIMIT: u32 = 25;
const AUTH_GLOBAL_WINDOW_SECONDS: u64 = 60;
const AUTH_TRACKED_USERNAME_LIMIT: usize = 4_096;

/// State shared by API route handlers.
#[derive(Debug, Clone)]
pub struct ApiState {
    storage_config: StorageConfig,
    are_session_cookies_secure: bool,
    auth_rate_limiter: Arc<Mutex<AuthRateLimiter>>,
}

impl ApiState {
    /// Build API state from storage configuration.
    ///
    /// # Arguments
    ///
    /// * `storage_config` - Data path configuration.
    /// * `are_session_cookies_secure` - Whether session cookies include Secure.
    ///
    /// # Returns
    ///
    /// Shared API state.
    pub fn new(storage_config: StorageConfig, are_session_cookies_secure: bool) -> Self {
        Self {
            storage_config,
            are_session_cookies_secure,
            auth_rate_limiter: Arc::new(Mutex::new(AuthRateLimiter::new(
                AuthRateLimitConfig::default(),
            ))),
        }
    }

    /// Return storage configuration.
    ///
    /// # Returns
    ///
    /// Storage configuration used by repositories.
    pub fn storage_config(&self) -> &StorageConfig {
        &self.storage_config
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
mod tests {
    use super::{AuthAttemptKind, AuthRateLimitConfig, AuthRateLimiter};

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
}
