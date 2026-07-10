//! SQLite storage boundaries and path resolution helpers.

pub mod announcements;
pub mod auth;
pub mod business;
pub mod cnki;
pub mod config;
pub mod index;
pub mod migrations;
pub mod sqlite;

pub use announcements::{list_active_announcements, AnnouncementRepositoryError};
pub use auth::{
    bootstrap_admin, count_users, create_invite_code, delete_access_token,
    delete_access_token_by_hash, delete_access_tokens_by_name, find_user_credentials_by_id,
    find_user_credentials_by_username, get_user_invite_code, initialize_auth_database,
    insert_access_token, list_access_tokens, random_hex, register_user_with_invite,
    update_user_password_and_delete_tokens, verify_access_token_hash, AccessTokenRow,
    AuthRepositoryError, AuthUserRow, InviteCodeRow, UserCredentialRow,
};
pub use business::{
    add_favorite, admin_create_invite_code, batch_is_favorited, bulk_add_favorites,
    bulk_move_favorites, bulk_remove_favorites, count_favorites, count_weekly_articles,
    create_announcement, create_folder, create_scheduled_task, delete_announcement, delete_folder,
    delete_invite_code, delete_scheduled_task, delete_user, get_admin_stats, get_announcement,
    get_notification_settings, get_scheduled_task, get_tracking_folder, is_favorited,
    list_all_announcements, list_all_invite_codes, list_all_users, list_available_database_names,
    list_favorite_articles, list_favorites, list_folders, list_notification_subscribers,
    list_runtime_settings, list_scheduled_tasks, normalize_database_names,
    record_scheduled_task_run, remove_favorite, rename_folder, set_tracking_folder, set_user_admin,
    update_announcement, update_scheduled_task, upsert_notification_settings,
    upsert_runtime_settings, BusinessRepositoryError,
};
pub use cnki::{
    delete_cnki_session, get_cnki_session_data, get_cnki_session_status, touch_cnki_session_used,
    upsert_cnki_session, CnkiRepositoryError, CnkiSessionData,
};
pub use config::{DatabaseResolutionError, StorageConfig};
pub use index::{
    article_fulltext_redirect_url, article_fulltext_target, collect_inpress_article_counts,
    collect_issue_article_counts, fetch_candidates_for_article_ids,
    fetch_candidates_for_inpress_keys, fetch_candidates_for_issue_keys, get_article,
    get_article_access, get_issue, get_journal, get_weekly_updates, list_areas, list_articles,
    list_index_database_names, list_issues, list_journal_options, list_journals, list_sources,
    list_years, ArticleFulltextTarget, ArticleListParams, CnkiFulltextTarget, IndexRepositoryError,
    IssueListParams, JournalListParams,
};
pub use migrations::{
    migrate_auth_database, migrate_existing_index_databases, migrate_index_database,
    migrate_storage, MigrationError, AUTH_SCHEMA_VERSION, INDEX_SCHEMA_VERSION,
};
pub use sqlite::{open_sqlite_connection, try_load_extension};
