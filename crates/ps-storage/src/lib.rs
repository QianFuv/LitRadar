//! SQLite storage boundaries and path resolution helpers.

pub mod announcements;
pub mod config;
pub mod sqlite;

pub use announcements::{list_active_announcements, AnnouncementRepositoryError};
pub use config::{DatabaseResolutionError, StorageConfig};
pub use sqlite::{open_sqlite_connection, try_load_extension};
