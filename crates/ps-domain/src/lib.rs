//! Shared domain models and compatibility primitives for the backend.

pub mod announcements;
pub mod health;
pub mod ids;
pub mod response;

pub use announcements::AnnouncementInfo;
pub use health::HealthResponse;
pub use ids::{stable_sqlite_id, ArticleId, JournalId, UserId};
pub use response::ErrorEnvelope;
