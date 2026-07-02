//! SQLite storage boundaries and path resolution helpers.

pub mod config;
pub mod sqlite;

pub use config::{DatabaseResolutionError, StorageConfig};
pub use sqlite::{open_sqlite_connection, try_load_extension};
