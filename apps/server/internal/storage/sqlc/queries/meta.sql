-- name: MaximumVersion :one
SELECT MAX(bundle_version) FROM managed_meta_catalogs;

-- name: CatalogState :one
SELECT bundle_version,applied_sha256 FROM managed_meta_catalogs WHERE filename=?;

-- name: StoreCatalogState :execresult
INSERT INTO managed_meta_catalogs(filename,bundle_version,applied_sha256) VALUES(?,?,?) ON CONFLICT(filename) DO UPDATE SET bundle_version=excluded.bundle_version,applied_sha256=excluded.applied_sha256;

