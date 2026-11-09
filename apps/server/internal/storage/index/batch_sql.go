package index

const batchCreateSql = `CREATE TABLE index_batches (
             batch_id TEXT PRIMARY KEY CHECK (length(batch_id) BETWEEN 1 AND 512),
             status TEXT NOT NULL
                 CHECK (status IN ('active', 'abandoning', 'completed', 'abandoned')),
             fingerprint TEXT NOT NULL CHECK (length(fingerprint) = 64),
             selection_kind TEXT NOT NULL
                 CHECK (selection_kind IN ('all', 'explicit_file')),
             sync_mode TEXT NOT NULL
                 CHECK (sync_mode IN ('bootstrap', 'incremental', 'full_rescan')),
             issue_batch_size INTEGER NOT NULL CHECK (issue_batch_size > 0),
             notify INTEGER NOT NULL CHECK (notify IN (0, 1)),
             notify_dry_run INTEGER NOT NULL CHECK (notify_dry_run IN (0, 1)),
             started_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             completed_at INTEGER
         );

         CREATE UNIQUE INDEX index_batches_one_active
             ON index_batches ((1))
             WHERE status IN ('active', 'abandoning');

         CREATE TABLE index_batch_catalogs (
             batch_id TEXT NOT NULL,
             ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
             file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 512),
             catalog_name TEXT NOT NULL CHECK (length(catalog_name) BETWEEN 1 AND 512),
             csv_sha256 TEXT NOT NULL CHECK (length(csv_sha256) = 64),
             provider_name TEXT NOT NULL CHECK (length(provider_name) BETWEEN 1 AND 512),
             journal_count INTEGER NOT NULL CHECK (journal_count >= 0),
             phase TEXT NOT NULL CHECK (phase IN (
                 'pending', 'indexing', 'manifest_prepared', 'manifest_published',
                 'notifying', 'completed'
             )),
             run_id TEXT CHECK (run_id IS NULL OR length(run_id) BETWEEN 1 AND 512),
             written_article_count INTEGER CHECK (
                 written_article_count IS NULL OR written_article_count >= 0
             ),
             source_attempt_count INTEGER CHECK (
                 source_attempt_count IS NULL OR source_attempt_count >= 0
             ),
             outcome_manifest_path TEXT,
             notify_attempt_id TEXT CHECK (
                 notify_attempt_id IS NULL OR length(notify_attempt_id) BETWEEN 1 AND 512
             ),
             notify_status TEXT CHECK (notify_status IS NULL OR notify_status IN (
                 'running', 'idle', 'completed', 'skipped', 'failed', 'cancelled',
                 'timed_out', 'unknown'
             )),
             notify_exit_code INTEGER,
             notify_unknown_acknowledged_attempt_id TEXT CHECK (
                 notify_unknown_acknowledged_attempt_id IS NULL
                 OR length(notify_unknown_acknowledged_attempt_id) BETWEEN 1 AND 512
             ),
             notify_unknown_acknowledged_at INTEGER CHECK (
                 notify_unknown_acknowledged_at IS NULL OR notify_unknown_acknowledged_at >= 0
             ),
             manifest_payload BLOB CHECK (
                 manifest_payload IS NULL OR length(manifest_payload) BETWEEN 1 AND 67108864
             ),
             manifest_sha256 TEXT CHECK (
                 manifest_sha256 IS NULL OR length(manifest_sha256) = 64
             ),
             manifest_through_event_id INTEGER CHECK (
                 manifest_through_event_id IS NULL OR manifest_through_event_id > 0
             ),
             manifest_path TEXT,
             manifest_run_id TEXT CHECK (
                 manifest_run_id IS NULL OR length(manifest_run_id) BETWEEN 1 AND 512
             ),
             manifest_generated_at TEXT CHECK (
                 manifest_generated_at IS NULL OR length(manifest_generated_at) BETWEEN 1 AND 512
             ),
             updated_at INTEGER NOT NULL,
             completed_at INTEGER,
             PRIMARY KEY (batch_id, ordinal),
             UNIQUE (batch_id, file_name),
             UNIQUE (batch_id, catalog_name),
             FOREIGN KEY (batch_id) REFERENCES index_batches(batch_id) ON DELETE CASCADE
         );

         CREATE TABLE index_batch_lease (
             lease_key INTEGER PRIMARY KEY CHECK (lease_key = 1),
             batch_id TEXT NOT NULL,
             owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 512),
             heartbeat_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             FOREIGN KEY (batch_id) REFERENCES index_batches(batch_id) ON DELETE CASCADE
         );

         CREATE INDEX index_batch_catalogs_phase
             ON index_batch_catalogs(batch_id, phase);`

const batchUpgradeSql = `ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_attempt_id TEXT CHECK (
                     notify_attempt_id IS NULL OR length(notify_attempt_id) BETWEEN 1 AND 512
                 );
             ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_status TEXT CHECK (notify_status IS NULL OR notify_status IN (
                     'running', 'idle', 'completed', 'skipped', 'failed', 'cancelled',
                     'timed_out', 'unknown'
                 ));
             ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_unknown_acknowledged_attempt_id TEXT CHECK (
                     notify_unknown_acknowledged_attempt_id IS NULL
                     OR length(notify_unknown_acknowledged_attempt_id) BETWEEN 1 AND 512
                 );
             ALTER TABLE index_batch_catalogs
                 ADD COLUMN notify_unknown_acknowledged_at INTEGER CHECK (
                     notify_unknown_acknowledged_at IS NULL
                     OR notify_unknown_acknowledged_at >= 0
                 );
             UPDATE index_batch_catalogs
             SET notify_attempt_id = 'legacy-notify-' || lower(hex(randomblob(16))),
                 notify_status = CASE
                     WHEN phase = 'notifying' THEN 'unknown'
                     WHEN phase = 'completed' AND notify_exit_code = 0 THEN 'completed'
                     ELSE 'unknown'
                 END
             WHERE phase = 'notifying' OR notify_exit_code IS NOT NULL;`
