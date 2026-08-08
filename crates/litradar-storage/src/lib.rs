//! SQLite storage boundaries and path resolution helpers.

pub mod announcements;
mod article_authors;
pub mod auth;
pub mod backup;
pub mod business;
pub mod cnki;
pub mod config;
pub mod index;
pub mod meta;
pub mod migrations;
pub mod secrets;
pub mod sqlite;

pub use announcements::{list_active_announcements, AnnouncementRepositoryError};
pub use auth::{
    bootstrap_admin, bootstrap_admin_with_audit, compare_and_swap_legacy_password_hash,
    compare_and_swap_user_password_and_delete_tokens_with_audit, count_users, create_invite_code,
    create_invite_code_with_audit, delete_access_token, delete_access_token_by_hash,
    delete_access_token_by_hash_with_audit, delete_access_token_with_audit,
    delete_all_access_tokens_with_audit, find_user_credentials_by_id,
    find_user_credentials_by_username, get_user_invite_code, initialize_auth_database,
    insert_personal_access_token, insert_personal_access_token_with_audit,
    issue_invite_code_with_audit, list_access_tokens, random_hex, register_user_with_invite,
    register_user_with_invite_and_audit, replace_login_access_token,
    replace_login_access_token_with_audit, revoke_user_invite_code_with_audit,
    rotate_user_invite_code_with_audit, update_user_password_and_delete_tokens,
    update_user_password_and_delete_tokens_with_audit, verify_access_token_hash, AccessTokenRow,
    AuthRepositoryError, AuthUserRow, InviteCodeRow, UserCredentialRow,
};
pub use backup::{
    create_backup, delete_service_heartbeat, has_recent_service_heartbeat,
    record_service_heartbeat, restore_backup, verify_backup, BackupComponent, BackupComponentKind,
    BackupCreateOptions, BackupError, BackupManifest, BackupRestoreOptions, BackupRestoreReport,
    BackupSelection, ServiceKind, ACTIVE_HEARTBEAT_MAX_AGE_SECONDS, BACKUP_FORMAT_VERSION,
};
pub use business::{
    acquire_delivery_lease, add_favorite, admin_create_invite_code,
    admin_create_invite_code_with_audit, admin_create_invite_code_with_policy_and_audit,
    admit_delivery_run, append_security_audit_event, batch_is_favorited, bulk_add_favorites,
    bulk_move_favorites, bulk_remove_favorites, canonicalize_outbound_base_url, claim_delivery_run,
    claim_delivery_run_item, claim_next_delivery_run_item, claim_ready_scheduled_runs,
    cleanup_confirmed_delivery_dedupe, cleanup_security_audit_events,
    compare_and_swap_delivery_checkpoint, count_favorites, count_weekly_articles,
    create_announcement, create_announcement_with_audit, create_folder, create_scheduled_task,
    create_scheduled_task_with_audit, delete_announcement, delete_announcement_with_audit,
    delete_folder, delete_scheduled_task, delete_scheduled_task_with_audit, delete_user,
    delete_user_with_audit, enqueue_delivery_run, enqueue_scheduled_runs,
    ensure_delivery_run_items, finalize_delivery_attempt, finalize_delivery_run,
    finalize_delivery_run_item, finalize_delivery_run_with_checkpoint,
    finalize_queued_delivery_run, finish_scheduled_run, get_admin_stats, get_announcement,
    get_notification_settings, get_notification_subscriber, get_scheduled_task,
    get_scheduler_last_checked_at, get_scheduler_status, get_tracking_folder,
    heartbeat_scheduled_run, import_legacy_delivery_state_files, insert_delivery_run_items,
    is_favorited, list_all_announcements, list_all_invite_codes, list_all_users,
    list_available_database_names, list_delivery_dedupe_for_scope, list_delivery_run_items,
    list_dispatchable_manual_delivery_runs, list_favorite_articles, list_favorites, list_folders,
    list_notification_subscribers, list_runtime_settings, list_scheduled_tasks,
    list_security_audit_events, load_ai_allowed_base_urls, load_audit_retention_days,
    load_delivery_checkpoint, load_delivery_dedupe, load_delivery_lease, load_delivery_run,
    load_delivery_worker_concurrency, load_latest_manual_delivery_run,
    load_manual_delivery_run_by_external_id, load_manual_delivery_run_by_external_id_for_admin,
    load_runtime_logging_settings, load_runtime_settings, mark_delivery_run_item_sending,
    normalize_database_names, parse_runtime_setting, reconcile_delivery_run_after_takeover,
    record_scheduled_task_run, record_scheduler_check, record_scheduler_heartbeat,
    release_delivery_dedupe_reservation, release_delivery_dedupe_reservations,
    release_delivery_lease, remove_favorite, rename_folder, renew_delivery_lease,
    renew_delivery_run, renew_delivery_run_item, report_security_audit_persistence_failure,
    request_delivery_run_cancellation, reserve_delivery_dedupe, resolve_delivery_dedupe,
    revoke_admin_invite_code, revoke_admin_invite_code_with_audit, runtime_setting_default,
    security_audit_persistence_failure_count, set_tracking_folder, set_user_admin,
    set_user_admin_with_audit, start_delivery_run, start_scheduled_run, update_announcement,
    update_announcement_with_audit, update_scheduled_task, update_scheduled_task_with_audit,
    upsert_notification_settings, upsert_runtime_settings, upsert_runtime_settings_with_audit,
    AuthRateLimitPolicy, BusinessRepositoryError, DeliveryCheckpointRecord,
    DeliveryCheckpointStatus, DeliveryCheckpointUpdate, DeliveryDedupeRecord,
    DeliveryDedupeReserveOutcome, DeliveryDedupeResolution, DeliveryDedupeStatus, DeliveryItemKind,
    DeliveryItemStatus, DeliveryLeaseAcquireOutcome, DeliveryLeaseRecord, DeliveryRecoveryResult,
    DeliveryRepositoryError, DeliveryRunAdmissionOutcome, DeliveryRunClaimOutcome,
    DeliveryRunCreate, DeliveryRunFinalization, DeliveryRunItemCreate, DeliveryRunItemRecord,
    DeliveryRunMode, DeliveryRunRecord, DeliveryRunStatus, DeliveryTriggerKind, DeliveryWorkflow,
    LegacyDeliveryImportResult, ParsedRuntimeSettingValue, RuntimeLoggingSettings,
    RuntimeSettingKey, ScheduledRunClaim, ScheduledTaskCreateParams, ScheduledTaskUpdateParams,
    SecurityAuditError, SecurityAuditEvent, SecurityAuditRecord, SecurityAuditRetentionResult,
    TokenBucketPolicy, TrustedProxyCidr, DEFAULT_AUDIT_RETENTION_DAYS,
    DEFAULT_AUTH_RATE_LIMIT_POLICY_JSON, DEFAULT_DELIVERY_WORKER_CONCURRENCY,
    DEFAULT_RUNTIME_LOG_FILTER, DEFAULT_RUNTIME_LOG_FORMAT, MAX_AUDIT_RETENTION_DAYS,
    MAX_DELIVERY_WORKER_CONCURRENCY, MIN_AUDIT_RETENTION_DAYS,
};
pub use cnki::{
    compare_and_swap_cnki_session, delete_cnki_session, get_cnki_session_data,
    get_cnki_session_status, reserve_cnki_session_operation, touch_cnki_session_used,
    upsert_cnki_session, CnkiRepositoryError, CnkiSessionData,
};
pub use config::{DatabaseResolutionError, StorageConfig};
pub use index::{
    collect_inpress_article_counts, collect_issue_article_counts, fetch_candidates_for_article_ids,
    fetch_candidates_for_inpress_keys, fetch_candidates_for_issue_keys, get_article,
    get_article_locator, get_issue, get_journal, get_weekly_updates, list_areas, list_articles,
    list_index_database_names, list_issues, list_journal_options, list_journals, list_years,
    ArticleListParams, IndexRepositoryError, IssueListParams, JournalListParams,
};
pub use meta::{
    discover_packaged_meta_dir, prepare_managed_meta, ManagedMetaAction, ManagedMetaCatalogReport,
    ManagedMetaError, ManagedMetaPreparationReport,
};
pub use migrations::{
    migrate_auth_database, migrate_existing_index_databases, migrate_index_database,
    migrate_storage, MigrationError, AUTH_SCHEMA_VERSION, INDEX_SCHEMA_VERSION,
};
pub use secrets::{
    migrate_database_secrets, rotate_database_secrets, verify_database_secrets, SecretCodec,
    SecretError, SecretMigrationReport, SecretVerificationReport,
};
pub use sqlite::{
    cleanup_sqlite_sidecars, open_sqlite_connection, try_load_extension, SqliteSidecarCleanup,
};
