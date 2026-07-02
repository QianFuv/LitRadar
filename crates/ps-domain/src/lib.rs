//! Shared domain models and compatibility primitives for the backend.

pub mod ids;
pub mod response;

pub use ids::{stable_sqlite_id, ArticleId, JournalId, UserId};
pub use response::ErrorEnvelope;
