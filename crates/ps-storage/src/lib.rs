//! SQLite storage boundaries and path resolution helpers.

pub mod announcements;
pub mod auth;
pub mod config;
pub mod sqlite;

pub use announcements::{list_active_announcements, AnnouncementRepositoryError};
pub use auth::{
    count_users, create_invite_code, delete_access_token, delete_access_token_by_hash,
    delete_access_tokens_by_name, find_user_credentials_by_id, find_user_credentials_by_username,
    get_user_invite_code, initialize_auth_database, insert_access_token, list_access_tokens,
    random_hex, register_user_with_invite, update_user_password_and_delete_tokens,
    verify_access_token_hash, AccessTokenRow, AuthRepositoryError, AuthUserRow, InviteCodeRow,
    UserCredentialRow,
};
pub use config::{DatabaseResolutionError, StorageConfig};
pub use sqlite::{open_sqlite_connection, try_load_extension};
