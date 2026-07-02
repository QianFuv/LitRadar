//! Announcement response models shared by public API handlers.

use serde::{Deserialize, Serialize};

/// Public announcement payload returned by the API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnouncementInfo {
    /// Announcement identifier.
    pub id: i64,
    /// Announcement title.
    pub title: String,
    /// Announcement body text.
    pub message: String,
    /// Priority label used for display ordering.
    pub priority: String,
    /// Whether the announcement is visible to public clients.
    pub enabled: bool,
    /// Creation timestamp as a Unix epoch float.
    pub created_at: f64,
    /// Last update timestamp as a Unix epoch float.
    pub updated_at: f64,
}
