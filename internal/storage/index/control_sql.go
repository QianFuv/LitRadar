package index

const controlLeaseSchema = `CREATE TABLE IF NOT EXISTS provider_leases (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             run_id TEXT NOT NULL,
             heartbeat_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             PRIMARY KEY (catalog_name, provider_name)
         );`

const controlStateSchema = `CREATE TABLE IF NOT EXISTS provider_sync_anchors (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             catalog_id TEXT NOT NULL,
             committed_anchor TEXT
                 CHECK (committed_anchor IS NULL OR (
                     length(CAST(committed_anchor AS BLOB)) BETWEEN 1 AND 65536
                 )),
             completed_at TEXT NOT NULL CHECK (length(completed_at) > 0),
             completed_batch_id TEXT
                 CHECK (completed_batch_id IS NULL OR length(completed_batch_id) > 0),
             PRIMARY KEY (catalog_name, provider_name, catalog_id)
         );

         CREATE TABLE IF NOT EXISTS provider_run_checkpoints (
             catalog_name TEXT NOT NULL,
             provider_name TEXT NOT NULL,
             catalog_id TEXT NOT NULL,
             batch_id TEXT CHECK (batch_id IS NULL OR length(batch_id) > 0),
             run_id TEXT NOT NULL CHECK (length(run_id) > 0),
             sync_mode TEXT NOT NULL
                 CHECK (sync_mode IN ('bootstrap', 'incremental', 'full_rescan')),
             base_anchor TEXT
                 CHECK (base_anchor IS NULL OR (
                     length(CAST(base_anchor AS BLOB)) BETWEEN 1 AND 65536
                 )),
             traversal_checkpoint TEXT
                 CHECK (traversal_checkpoint IS NULL OR (
                     length(CAST(traversal_checkpoint AS BLOB)) BETWEEN 1 AND 65536
                 )),
             started_at TEXT NOT NULL CHECK (length(started_at) > 0),
             updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
             PRIMARY KEY (catalog_name, provider_name, catalog_id)
         );

         CREATE INDEX IF NOT EXISTS idx_provider_sync_anchors_catalog
             ON provider_sync_anchors(catalog_name, catalog_id);
         CREATE INDEX IF NOT EXISTS idx_provider_run_checkpoints_catalog
             ON provider_run_checkpoints(catalog_name, catalog_id);`
