//! Shared Axum application state.

use ps_storage::StorageConfig;

/// State shared by API route handlers.
#[derive(Debug, Clone)]
pub struct ApiState {
    storage_config: StorageConfig,
}

impl ApiState {
    /// Build API state from storage configuration.
    ///
    /// # Arguments
    ///
    /// * `storage_config` - Data path configuration.
    ///
    /// # Returns
    ///
    /// Shared API state.
    pub fn new(storage_config: StorageConfig) -> Self {
        Self { storage_config }
    }

    /// Return storage configuration.
    ///
    /// # Returns
    ///
    /// Storage configuration used by repositories.
    pub fn storage_config(&self) -> &StorageConfig {
        &self.storage_config
    }
}
