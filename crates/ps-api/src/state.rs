//! Shared Axum application state.

use ps_storage::StorageConfig;

/// State shared by API route handlers.
#[derive(Debug, Clone)]
pub struct ApiState {
    storage_config: StorageConfig,
    are_session_cookies_secure: bool,
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
}
