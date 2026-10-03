// Package auth preserves the complete versioned auth database schema.
package auth

const versionTwoSql = `
        CREATE TABLE scheduled_tasks_v2 (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            name           TEXT    NOT NULL,
            job_spec       TEXT,
            legacy_command TEXT,
            cron           TEXT    NOT NULL,
            enabled        INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            last_run_at    REAL,
            last_status    TEXT    NOT NULL DEFAULT '',
            created_at     REAL    NOT NULL,
            updated_at     REAL    NOT NULL,
            CHECK (
                (job_spec IS NOT NULL AND legacy_command IS NULL)
                OR (job_spec IS NULL AND legacy_command IS NOT NULL)
            ),
            CHECK (job_spec IS NOT NULL OR enabled = 0)
        );

        INSERT INTO scheduled_tasks_v2
            (id, name, job_spec, legacy_command, cron, enabled, last_run_at,
             last_status, created_at, updated_at)
        SELECT
            id, name, NULL, command, cron, 0, last_run_at, last_status,
            created_at, updated_at
        FROM scheduled_tasks;

        DROP TABLE scheduled_tasks;
        ALTER TABLE scheduled_tasks_v2 RENAME TO scheduled_tasks;
        CREATE INDEX idx_scheduled_tasks_enabled ON scheduled_tasks(enabled);
        `

const versionThreeSql = `
        ALTER TABLE scheduled_tasks
            ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
        ALTER TABLE scheduled_tasks
            ADD COLUMN timeout_seconds INTEGER NOT NULL DEFAULT 3600
            CHECK (timeout_seconds BETWEEN 1 AND 86400);
        ALTER TABLE scheduled_tasks
            ADD COLUMN coalesce INTEGER NOT NULL DEFAULT 1
            CHECK (coalesce IN (0, 1));

        CREATE TABLE scheduler_state (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            last_checked_at REAL
        );

        INSERT INTO scheduler_state (id, last_checked_at) VALUES (1, NULL);

        CREATE TABLE scheduler_workers (
            worker_id    TEXT PRIMARY KEY,
            started_at   REAL NOT NULL,
            heartbeat_at REAL NOT NULL
        );

        CREATE TABLE scheduled_task_runs (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id          INTEGER NOT NULL,
            task_name        TEXT    NOT NULL,
            scheduled_for    INTEGER NOT NULL,
            status           TEXT    NOT NULL
                CHECK (status IN ('pending', 'claimed', 'running', 'success',
                                  'failed', 'timed_out', 'error', 'unknown')),
            worker_id        TEXT,
            claim_expires_at REAL,
            claimed_at       REAL,
            started_at       REAL,
            finished_at      REAL,
            output_summary   TEXT NOT NULL DEFAULT '',
            UNIQUE(task_id, scheduled_for)
        );

        CREATE INDEX idx_scheduled_task_runs_task
            ON scheduled_task_runs(task_id, scheduled_for DESC);
        CREATE INDEX idx_scheduled_task_runs_status
            ON scheduled_task_runs(status, claim_expires_at);
        CREATE INDEX idx_scheduler_workers_heartbeat
            ON scheduler_workers(heartbeat_at DESC);
        `

const versionFourSql = `
        CREATE TABLE service_heartbeats (
            service      TEXT NOT NULL CHECK (service IN ('api', 'worker')),
            instance_id  TEXT NOT NULL,
            started_at   REAL NOT NULL,
            heartbeat_at REAL NOT NULL,
            PRIMARY KEY(service, instance_id)
        );

        CREATE INDEX idx_service_heartbeats_recent
            ON service_heartbeats(heartbeat_at DESC);
        `

const versionFiveSql = `
        CREATE TABLE scheduled_task_runs_v5 (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id          INTEGER NOT NULL,
            task_name        TEXT    NOT NULL,
            scheduled_for    INTEGER NOT NULL,
            status           TEXT    NOT NULL
                CHECK (status IN ('pending', 'claimed', 'running', 'success',
                                  'failed', 'timed_out', 'error', 'unknown',
                                  'cancelled')),
            worker_id        TEXT,
            claim_expires_at REAL,
            claimed_at       REAL,
            started_at       REAL,
            finished_at      REAL,
            output_summary   TEXT NOT NULL DEFAULT '',
            UNIQUE(task_id, scheduled_for)
        );

        INSERT INTO scheduled_task_runs_v5
            (id, task_id, task_name, scheduled_for, status, worker_id,
             claim_expires_at, claimed_at, started_at, finished_at,
             output_summary)
        SELECT
            id, task_id, task_name, scheduled_for, status, worker_id,
            claim_expires_at, claimed_at, started_at, finished_at,
            output_summary
        FROM scheduled_task_runs;

        DROP TABLE scheduled_task_runs;
        ALTER TABLE scheduled_task_runs_v5 RENAME TO scheduled_task_runs;
        CREATE INDEX idx_scheduled_task_runs_task
            ON scheduled_task_runs(task_id, scheduled_for DESC);
        CREATE INDEX idx_scheduled_task_runs_status
            ON scheduled_task_runs(status, claim_expires_at);
        `

const versionSixSql = `
        CREATE TABLE managed_meta_catalogs (
            filename       TEXT PRIMARY KEY,
            bundle_version INTEGER NOT NULL CHECK (bundle_version > 0),
            applied_sha256 TEXT NOT NULL CHECK (length(applied_sha256) = 64)
        );
        `

const versionNineSql = `CREATE TABLE security_audit_events (
             id                  INTEGER PRIMARY KEY AUTOINCREMENT,
             actor_id            INTEGER CHECK (actor_id IS NULL OR actor_id > 0),
             target_id           INTEGER CHECK (target_id IS NULL OR target_id > 0),
             action              TEXT NOT NULL CHECK (
                 length(action) BETWEEN 1 AND 64 AND action NOT GLOB '*[^a-z0-9_]*'
             ),
             outcome             TEXT NOT NULL CHECK (
                 length(outcome) BETWEEN 1 AND 64 AND outcome NOT GLOB '*[^a-z0-9_]*'
             ),
             reason              TEXT NOT NULL DEFAULT '' CHECK (
                 length(reason) <= 64 AND reason NOT GLOB '*[^a-z0-9_]*'
             ),
             request_id          TEXT NOT NULL DEFAULT '' CHECK (
                 length(request_id) <= 128 AND request_id NOT GLOB '*[^A-Za-z0-9_.:-]*'
             ),
             source_class        TEXT NOT NULL DEFAULT '' CHECK (
                 length(source_class) <= 64 AND source_class NOT GLOB '*[^a-z0-9_]*'
             ),
             bucket              TEXT NOT NULL DEFAULT '' CHECK (
                 length(bucket) <= 64 AND bucket NOT GLOB '*[^a-z0-9_]*'
             ),
             rejected_count      INTEGER NOT NULL DEFAULT 0 CHECK (rejected_count >= 0),
             retry_after_seconds INTEGER NOT NULL DEFAULT 0 CHECK (retry_after_seconds >= 0),
             occurred_at         REAL NOT NULL
         );

         CREATE INDEX idx_security_audit_events_occurred
             ON security_audit_events(occurred_at, id);
         CREATE INDEX idx_security_audit_events_action_outcome
             ON security_audit_events(action, outcome, occurred_at DESC);
         CREATE INDEX idx_security_audit_events_actor
             ON security_audit_events(actor_id, occurred_at DESC);
         CREATE INDEX idx_security_audit_events_request
             ON security_audit_events(request_id) WHERE request_id <> '';

         CREATE TRIGGER security_audit_events_no_update
         BEFORE UPDATE ON security_audit_events
         BEGIN
             SELECT RAISE(ABORT, 'security audit events are append-only');
         END;

         CREATE TABLE security_audit_maintenance (
             id                INTEGER PRIMARY KEY CHECK (id = 1),
             last_retention_at REAL
         );
         INSERT INTO security_audit_maintenance (id, last_retention_at) VALUES (1, NULL);`

const versionTenSql = `CREATE TABLE delivery_checkpoints (
             id                    INTEGER PRIMARY KEY AUTOINCREMENT,
             workflow              TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             db_name               TEXT NOT NULL CHECK (length(db_name) BETWEEN 1 AND 255),
             status                TEXT NOT NULL DEFAULT 'idle' CHECK (
                 status IN ('idle', 'running', 'completed', 'failed', 'skipped', 'unknown')
             ),
             legacy_status         TEXT CHECK (
                 legacy_status IS NULL OR length(legacy_status) BETWEEN 1 AND 128
             ),
             snapshot_json         TEXT NOT NULL,
             last_completed_run_at TEXT,
             revision              INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             legacy_source_hash    TEXT CHECK (
                 legacy_source_hash IS NULL OR (
                     length(legacy_source_hash) = 64
                     AND legacy_source_hash NOT GLOB '*[^0-9a-f]*'
                 )
             ),
             legacy_source_name    TEXT CHECK (
                 legacy_source_name IS NULL OR length(legacy_source_name) BETWEEN 1 AND 255
             ),
             legacy_imported_at    REAL,
             created_at            REAL NOT NULL,
             updated_at            REAL NOT NULL,
             CHECK (
                 (legacy_source_hash IS NULL AND legacy_source_name IS NULL
                  AND legacy_imported_at IS NULL)
                 OR (legacy_source_hash IS NOT NULL AND legacy_source_name IS NOT NULL
                     AND legacy_imported_at IS NOT NULL)
             )
         );
         CREATE UNIQUE INDEX idx_delivery_checkpoints_scope
             ON delivery_checkpoints(workflow, db_name);

         CREATE TABLE delivery_runs (
             id                     INTEGER PRIMARY KEY AUTOINCREMENT,
             external_id            TEXT NOT NULL CHECK (length(external_id) BETWEEN 1 AND 128),
             workflow               TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             scope_key              TEXT NOT NULL CHECK (length(scope_key) BETWEEN 1 AND 255),
             db_name                TEXT CHECK (db_name IS NULL OR length(db_name) BETWEEN 1 AND 255),
             trigger_kind           TEXT NOT NULL CHECK (
                 trigger_kind IN ('scheduled', 'manual', 'legacy')
             ),
             mode                   TEXT NOT NULL CHECK (mode IN ('dry_run', 'execute')),
             user_id                INTEGER CHECK (user_id IS NULL OR user_id > 0),
             status                 TEXT NOT NULL CHECK (
                 status IN (
                     'queued', 'claimed', 'running', 'cancelling', 'completed', 'failed',
                     'cancelled', 'timed_out', 'skipped', 'unknown'
                 )
             ),
             legacy_status          TEXT CHECK (
                 legacy_status IS NULL OR length(legacy_status) BETWEEN 1 AND 128
             ),
             owner_id               TEXT CHECK (
                 owner_id IS NULL OR length(owner_id) BETWEEN 1 AND 128
             ),
             lease_expires_at       REAL,
             deadline_at            REAL,
             cancellation_requested INTEGER NOT NULL DEFAULT 0 CHECK (
                 cancellation_requested IN (0, 1)
             ),
             result_json            TEXT,
             error_code             TEXT CHECK (
                 error_code IS NULL OR length(error_code) BETWEEN 1 AND 64
             ),
             revision               INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             created_at             REAL NOT NULL,
             started_at             REAL,
             updated_at             REAL NOT NULL,
             finished_at            REAL,
             CHECK (
                 (status = 'queued' AND owner_id IS NULL AND lease_expires_at IS NULL
                  AND finished_at IS NULL)
                 OR (status IN ('claimed', 'running', 'cancelling')
                     AND owner_id IS NOT NULL AND lease_expires_at IS NOT NULL
                     AND finished_at IS NULL)
                 OR (status IN ('completed', 'failed', 'cancelled', 'timed_out', 'skipped', 'unknown')
                     AND owner_id IS NULL AND lease_expires_at IS NULL
                     AND finished_at IS NOT NULL)
             ),
             CHECK (
                 (trigger_kind = 'manual' AND user_id IS NOT NULL)
                 OR (trigger_kind <> 'manual' AND db_name IS NOT NULL)
             ),
             CHECK (deadline_at IS NULL OR deadline_at > created_at)
         );
         CREATE UNIQUE INDEX idx_delivery_runs_external_scope
             ON delivery_runs(workflow, scope_key, external_id);
         CREATE UNIQUE INDEX idx_delivery_runs_active_scope
             ON delivery_runs(workflow, db_name)
             WHERE db_name IS NOT NULL
               AND status IN ('claimed', 'running', 'cancelling');
         CREATE UNIQUE INDEX idx_delivery_runs_active_manual_user
             ON delivery_runs(user_id)
             WHERE trigger_kind = 'manual'
               AND user_id IS NOT NULL
               AND status IN ('queued', 'claimed', 'running', 'cancelling');
         CREATE INDEX idx_delivery_runs_queue
             ON delivery_runs(status, created_at, id);
         CREATE INDEX idx_delivery_runs_owner_lease
             ON delivery_runs(owner_id, lease_expires_at)
             WHERE owner_id IS NOT NULL;

         CREATE TABLE delivery_run_items (
             id               INTEGER PRIMARY KEY AUTOINCREMENT,
             delivery_run_id  INTEGER NOT NULL REFERENCES delivery_runs(id) ON DELETE CASCADE,
             item_kind        TEXT NOT NULL CHECK (
                 item_kind IN ('issue', 'inpress', 'article', 'subscriber')
             ),
             item_key         TEXT NOT NULL CHECK (length(item_key) BETWEEN 1 AND 512),
             user_id          INTEGER CHECK (user_id IS NULL OR user_id > 0),
             article_id       INTEGER CHECK (article_id IS NULL OR article_id > 0),
             status           TEXT NOT NULL CHECK (
                 status IN (
                     'pending', 'claimed', 'sending', 'succeeded', 'failed', 'skipped',
                     'cancelled', 'unknown'
                 )
             ),
             legacy_status    TEXT CHECK (
                 legacy_status IS NULL OR length(legacy_status) BETWEEN 1 AND 128
             ),
             owner_id         TEXT CHECK (
                 owner_id IS NULL OR length(owner_id) BETWEEN 1 AND 128
             ),
             lease_expires_at REAL,
             attempt_count    INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
             result_json      TEXT,
             error_code       TEXT CHECK (
                 error_code IS NULL OR length(error_code) BETWEEN 1 AND 64
             ),
             revision         INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             created_at       REAL NOT NULL,
             started_at       REAL,
             updated_at       REAL NOT NULL,
             finished_at      REAL,
             UNIQUE(delivery_run_id, item_kind, item_key),
             CHECK (
                 (status = 'pending' AND owner_id IS NULL AND lease_expires_at IS NULL
                  AND finished_at IS NULL)
                 OR (status IN ('claimed', 'sending')
                     AND owner_id IS NOT NULL AND lease_expires_at IS NOT NULL
                     AND finished_at IS NULL)
                 OR (status IN ('succeeded', 'failed', 'skipped', 'cancelled', 'unknown')
                     AND owner_id IS NULL AND lease_expires_at IS NULL
                     AND finished_at IS NOT NULL)
             )
         );
         CREATE INDEX idx_delivery_run_items_claim
             ON delivery_run_items(delivery_run_id, status, lease_expires_at, id);

         CREATE TABLE delivery_dedupe (
             id                  INTEGER PRIMARY KEY AUTOINCREMENT,
             workflow            TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             db_name             TEXT NOT NULL CHECK (length(db_name) BETWEEN 1 AND 255),
             user_id             INTEGER NOT NULL CHECK (user_id > 0),
             article_id          INTEGER NOT NULL CHECK (article_id > 0),
             delivery_run_id     INTEGER REFERENCES delivery_runs(id) ON DELETE SET NULL,
             status              TEXT NOT NULL CHECK (status IN ('reserved', 'confirmed', 'unknown')),
             message_id          TEXT CHECK (
                 message_id IS NULL OR length(message_id) BETWEEN 1 AND 256
             ),
             reservation_owner   TEXT CHECK (
                 reservation_owner IS NULL OR length(reservation_owner) BETWEEN 1 AND 128
             ),
             legacy_delivered_at TEXT,
             revision            INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             reserved_at         REAL NOT NULL,
             delivered_at        REAL,
             updated_at          REAL NOT NULL,
             CHECK (
                 (status = 'reserved' AND delivery_run_id IS NOT NULL
                  AND reservation_owner IS NOT NULL AND delivered_at IS NULL)
                 OR (status IN ('confirmed', 'unknown')
                     AND reservation_owner IS NULL AND delivered_at IS NOT NULL)
             )
         );
         CREATE UNIQUE INDEX idx_delivery_dedupe_identity
             ON delivery_dedupe(workflow, db_name, user_id, article_id);
         CREATE INDEX idx_delivery_dedupe_status_time
             ON delivery_dedupe(status, updated_at, id);

         CREATE TABLE delivery_leases (
             id                  INTEGER PRIMARY KEY AUTOINCREMENT,
             workflow            TEXT NOT NULL CHECK (workflow IN ('notify', 'push')),
             db_name             TEXT NOT NULL CHECK (length(db_name) BETWEEN 1 AND 255),
             delivery_run_id     INTEGER REFERENCES delivery_runs(id) ON DELETE SET NULL,
             owner_id            TEXT CHECK (
                 owner_id IS NULL OR length(owner_id) BETWEEN 1 AND 128
             ),
             revision            INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
             acquired_at         REAL,
             heartbeat_at        REAL,
             expires_at          REAL,
             updated_at          REAL NOT NULL,
             CHECK (
                 (owner_id IS NULL AND delivery_run_id IS NULL AND acquired_at IS NULL
                  AND heartbeat_at IS NULL AND expires_at IS NULL)
                 OR (owner_id IS NOT NULL AND delivery_run_id IS NOT NULL
                     AND acquired_at IS NOT NULL AND heartbeat_at IS NOT NULL
                     AND expires_at IS NOT NULL)
             )
         );
         CREATE UNIQUE INDEX idx_delivery_leases_scope
             ON delivery_leases(workflow, db_name);
         CREATE INDEX idx_delivery_leases_expiration
             ON delivery_leases(expires_at)
             WHERE owner_id IS NOT NULL;`

const versionElevenSql = `UPDATE folders
         SET is_tracking = 0
         WHERE is_tracking = 1
           AND id NOT IN (
               SELECT MIN(id) FROM folders WHERE is_tracking = 1 GROUP BY user_id
           );
         CREATE UNIQUE INDEX idx_folders_one_tracking_per_user
             ON folders(user_id) WHERE is_tracking = 1;`

const versionSixteenSql = `CREATE INDEX IF NOT EXISTS idx_favorites_cursor ON favorites(user_id, folder_id, created_at DESC, id DESC);`

const versionSeventeenSql = `DROP INDEX IF EXISTS idx_invite_codes_code;
         DROP INDEX IF EXISTS idx_notification_settings_user;`

const versionTwentySql = `
        CREATE TABLE scheduled_task_runs_v20 (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id          INTEGER NOT NULL,
            task_name        TEXT    NOT NULL,
            scheduled_for    INTEGER NOT NULL,
            status           TEXT    NOT NULL
                CHECK (status IN ('pending', 'claimed', 'running', 'success',
                                  'failed', 'timed_out', 'error', 'unknown',
                                  'cancelled')),
            worker_id        TEXT,
            claim_expires_at REAL,
            claimed_at       REAL,
            started_at       REAL,
            finished_at      REAL,
            output_summary   TEXT NOT NULL DEFAULT '',
            trigger_kind     TEXT NOT NULL DEFAULT 'scheduled'
                CHECK (trigger_kind IN ('scheduled', 'manual'))
        );

        INSERT INTO scheduled_task_runs_v20
            (id, task_id, task_name, scheduled_for, status, worker_id,
             claim_expires_at, claimed_at, started_at, finished_at,
             output_summary)
        SELECT
            id, task_id, task_name, scheduled_for, status, worker_id,
            claim_expires_at, claimed_at, started_at, finished_at,
            output_summary
        FROM scheduled_task_runs;

        DROP TABLE scheduled_task_runs;
        ALTER TABLE scheduled_task_runs_v20 RENAME TO scheduled_task_runs;
        CREATE INDEX idx_scheduled_task_runs_task
            ON scheduled_task_runs(task_id, scheduled_for DESC);
        CREATE INDEX idx_scheduled_task_runs_status
            ON scheduled_task_runs(status, claim_expires_at);
        CREATE UNIQUE INDEX idx_scheduled_task_runs_slot
            ON scheduled_task_runs(task_id, scheduled_for) WHERE trigger_kind = 'scheduled';
        `

const authTablesSql = `
    CREATE TABLE IF NOT EXISTS users (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
        password_hash TEXT    NOT NULL,
        salt          TEXT    NOT NULL,
        is_admin      INTEGER NOT NULL DEFAULT 0,
        created_at    REAL    NOT NULL,
        updated_at    REAL    NOT NULL,
        token_generation INTEGER NOT NULL DEFAULT 0 CHECK (token_generation >= 0)
    );

    CREATE TABLE IF NOT EXISTS access_tokens (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        token_hash  TEXT    NOT NULL UNIQUE,
        name        TEXT    NOT NULL DEFAULT '',
        expires_at  REAL    NOT NULL,
        created_at  REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS cnki_sessions (
        user_id          INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
        session_json     TEXT    NOT NULL DEFAULT '{}',
        qr_uuid          TEXT    NOT NULL DEFAULT '',
        status           TEXT    NOT NULL DEFAULT 'empty',
        token_expires_at REAL,
        created_at       REAL    NOT NULL,
        updated_at       REAL    NOT NULL,
        last_used_at     REAL
    );

    CREATE TABLE IF NOT EXISTS folders (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        name        TEXT    NOT NULL,
        is_tracking INTEGER NOT NULL DEFAULT 0,
        created_at  REAL    NOT NULL,
        updated_at  REAL    NOT NULL,
        UNIQUE(user_id, name)
    );

    CREATE TABLE IF NOT EXISTS favorites (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        folder_id   INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
        article_id  INTEGER NOT NULL,
        db_name     TEXT    NOT NULL DEFAULT '',
        note        TEXT    NOT NULL DEFAULT '',
        created_at  REAL    NOT NULL,
        UNIQUE(user_id, folder_id, article_id, db_name)
    );

    CREATE TABLE IF NOT EXISTS invite_codes (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        code        TEXT    NOT NULL UNIQUE,
        created_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_at     REAL,
        created_at  REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS notification_settings (
        id                      INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id                 INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
        keywords                TEXT    NOT NULL DEFAULT '[]',
        directions              TEXT    NOT NULL DEFAULT '[]',
        selected_databases      TEXT    NOT NULL DEFAULT '[]',
        delivery_method         TEXT    NOT NULL DEFAULT 'folder',
        pushplus_token          TEXT    NOT NULL DEFAULT '',
        pushplus_template       TEXT    NOT NULL DEFAULT 'markdown',
        pushplus_topic          TEXT    NOT NULL DEFAULT '',
        pushplus_channel        TEXT    NOT NULL DEFAULT 'wechat',
        sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0,
        ai_base_url             TEXT    NOT NULL DEFAULT '',
        ai_api_key              TEXT    NOT NULL DEFAULT '',
        ai_model                TEXT    NOT NULL DEFAULT '',
        ai_system_prompt        TEXT    NOT NULL DEFAULT '',
        ai_backup_base_url      TEXT    NOT NULL DEFAULT '',
        ai_backup_api_key       TEXT    NOT NULL DEFAULT '',
        ai_backup_model         TEXT    NOT NULL DEFAULT '',
        ai_backup_system_prompt TEXT    NOT NULL DEFAULT '',
        ai_retry_attempts       INTEGER NOT NULL DEFAULT 3,
        enabled                 INTEGER NOT NULL DEFAULT 1,
        created_at              REAL    NOT NULL,
        updated_at              REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS scheduled_tasks (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        name        TEXT    NOT NULL,
        command     TEXT    NOT NULL,
        cron        TEXT    NOT NULL,
        enabled     INTEGER NOT NULL DEFAULT 1,
        last_run_at REAL,
        last_status TEXT    NOT NULL DEFAULT '',
        created_at  REAL    NOT NULL,
        updated_at  REAL    NOT NULL
    );

    CREATE TABLE IF NOT EXISTS runtime_settings (
        key        TEXT PRIMARY KEY,
        value      TEXT NOT NULL DEFAULT '',
        updated_at REAL NOT NULL
    );

    CREATE TABLE IF NOT EXISTS announcements (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        title      TEXT    NOT NULL,
        message    TEXT    NOT NULL,
        priority   TEXT    NOT NULL DEFAULT 'normal',
        enabled    INTEGER NOT NULL DEFAULT 1,
        created_at REAL    NOT NULL,
        updated_at REAL    NOT NULL
    );
`

const authIndexesSql = `
    CREATE INDEX IF NOT EXISTS idx_access_tokens_user ON access_tokens(user_id);
    CREATE INDEX IF NOT EXISTS idx_folders_user ON folders(user_id);
    CREATE INDEX IF NOT EXISTS idx_favorites_folder ON favorites(folder_id);
    CREATE INDEX IF NOT EXISTS idx_favorites_user ON favorites(user_id);
    CREATE INDEX IF NOT EXISTS idx_invite_codes_code ON invite_codes(code);
    CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by ON invite_codes(created_by);
    CREATE INDEX IF NOT EXISTS idx_notification_settings_user ON notification_settings(user_id);
    CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_enabled ON scheduled_tasks(enabled);
    CREATE INDEX IF NOT EXISTS idx_announcements_enabled ON announcements(enabled);
`

const notificationSettingsV14TableSql = `
    CREATE TABLE notification_settings_v14 (
        id                      INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id                 INTEGER NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
        keywords                TEXT    NOT NULL DEFAULT '[]' CHECK (
            CASE WHEN json_valid(keywords)
                 THEN json_type(keywords) = 'array'
                 ELSE 0
            END
        ),
        directions              TEXT    NOT NULL DEFAULT '[]' CHECK (
            CASE WHEN json_valid(directions)
                 THEN json_type(directions) = 'array'
                 ELSE 0
            END
        ),
        selected_databases      TEXT    NOT NULL DEFAULT '[]' CHECK (
            CASE WHEN json_valid(selected_databases)
                 THEN json_type(selected_databases) = 'array'
                 ELSE 0
            END
        ),
        delivery_method         TEXT    NOT NULL DEFAULT 'folder',
        pushplus_token          TEXT    NOT NULL DEFAULT '',
        pushplus_template       TEXT    NOT NULL DEFAULT 'markdown',
        pushplus_topic          TEXT    NOT NULL DEFAULT '',
        pushplus_channel        TEXT    NOT NULL DEFAULT 'wechat',
        sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0,
        ai_base_url             TEXT    NOT NULL DEFAULT '',
        ai_api_key              TEXT    NOT NULL DEFAULT '',
        ai_model                TEXT    NOT NULL DEFAULT '',
        ai_system_prompt        TEXT    NOT NULL DEFAULT '',
        ai_backup_base_url      TEXT    NOT NULL DEFAULT '',
        ai_backup_api_key       TEXT    NOT NULL DEFAULT '',
        ai_backup_model         TEXT    NOT NULL DEFAULT '',
        ai_backup_system_prompt TEXT    NOT NULL DEFAULT '',
        ai_retry_attempts       INTEGER NOT NULL DEFAULT 3,
        enabled                 INTEGER NOT NULL DEFAULT 1,
        created_at              REAL    NOT NULL,
        updated_at              REAL    NOT NULL
    );
`

const notificationSettingsV14TriggersSql = `
    CREATE TRIGGER notification_settings_json_strings_insert
    AFTER INSERT ON notification_settings
    WHEN EXISTS (SELECT 1 FROM json_each(NEW.keywords) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.directions) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.selected_databases) WHERE type <> 'text')
    BEGIN
        SELECT RAISE(ABORT, 'notification settings JSON arrays must contain strings');
    END;

    CREATE TRIGGER notification_settings_json_strings_update
    AFTER UPDATE OF keywords, directions, selected_databases ON notification_settings
    WHEN EXISTS (SELECT 1 FROM json_each(NEW.keywords) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.directions) WHERE type <> 'text')
      OR EXISTS (SELECT 1 FROM json_each(NEW.selected_databases) WHERE type <> 'text')
    BEGIN
        SELECT RAISE(ABORT, 'notification settings JSON arrays must contain strings');
    END;
`

const inviteLifecycleTablesSql = `
    CREATE TABLE invite_codes_v12 (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        code        TEXT    NOT NULL UNIQUE,
        created_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_at     REAL,
        created_at  REAL    NOT NULL,
        expires_at  REAL    NOT NULL CHECK (expires_at > created_at),
        revoked_at  REAL    CHECK (revoked_at IS NULL OR revoked_at >= created_at),
        max_uses    INTEGER NOT NULL CHECK (max_uses BETWEEN 1 AND 1000),
        use_count   INTEGER NOT NULL DEFAULT 0 CHECK (
            use_count >= 0 AND use_count <= max_uses
        ),
        CHECK (used_by IS NULL OR used_at IS NOT NULL)
    );

    CREATE TABLE invite_code_uses (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        invite_code_id INTEGER NOT NULL REFERENCES invite_codes_v12(id) ON DELETE RESTRICT,
        user_id        INTEGER REFERENCES users(id) ON DELETE SET NULL,
        used_at        REAL NOT NULL
    );
`

const inviteLifecycleIndexesSql = `
    CREATE INDEX IF NOT EXISTS idx_invite_codes_code ON invite_codes(code);
    CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by ON invite_codes(created_by);
    CREATE UNIQUE INDEX IF NOT EXISTS idx_invite_codes_one_unrevoked_creator
        ON invite_codes(created_by)
        WHERE created_by IS NOT NULL AND revoked_at IS NULL;
    CREATE INDEX IF NOT EXISTS idx_invite_codes_lifecycle
        ON invite_codes(revoked_at, expires_at, use_count, max_uses);
    CREATE INDEX IF NOT EXISTS idx_invite_code_uses_code_time
        ON invite_code_uses(invite_code_id, used_at, id);
    CREATE INDEX IF NOT EXISTS idx_invite_code_uses_user
        ON invite_code_uses(user_id, used_at, id);
`

const versionEighteenSql = `
CREATE TABLE cfp_journals (
    journal_key TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    checked_on TEXT NOT NULL,
    source_url TEXT,
    source_statement TEXT
);
CREATE TABLE cfp_journal_aliases (
    catalog_id TEXT PRIMARY KEY,
    journal_key TEXT NOT NULL REFERENCES cfp_journals(journal_key) ON DELETE CASCADE
);
CREATE INDEX idx_cfp_alias_owner ON cfp_journal_aliases(journal_key);
CREATE TABLE cfp_sources (
    source_key TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    config_version INTEGER NOT NULL DEFAULT 0,
    capture TEXT NOT NULL,
    capture_format TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    etag TEXT,
    last_modified TEXT,
    last_attempt INTEGER,
    last_success INTEGER,
    last_error TEXT,
    status TEXT NOT NULL,
    generation INTEGER NOT NULL DEFAULT 0,
    lease_expires_at INTEGER,
    revision INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE cfp_source_journals (
    source_key TEXT NOT NULL REFERENCES cfp_sources(source_key) ON DELETE CASCADE,
    journal_key TEXT NOT NULL REFERENCES cfp_journals(journal_key) ON DELETE CASCADE,
    PRIMARY KEY(source_key, journal_key)
);
CREATE TABLE cfp_notices (
    journal_key TEXT NOT NULL,
    notice_key TEXT NOT NULL,
    source_key TEXT NOT NULL,
    source_json TEXT NOT NULL,
    normalized_json TEXT NOT NULL,
    display_order INTEGER NOT NULL,
    PRIMARY KEY(journal_key, notice_key),
    FOREIGN KEY(source_key, journal_key) REFERENCES cfp_source_journals(source_key, journal_key)
);
CREATE INDEX idx_cfp_notice_source ON cfp_notices(source_key, journal_key, display_order);
CREATE TABLE cfp_seed_imports (
    seed_id TEXT PRIMARY KEY,
    format_version INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    journal_count INTEGER NOT NULL,
    notice_count INTEGER NOT NULL
);
`
