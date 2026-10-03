// Package backup creates verified portable snapshots and restores inactive deployments.
package backup

import (
	"errors"
	"fmt"

	"github.com/QianFuv/LitRadar/internal/storage/config"
)

// FormatVersion includes managed metadata in newly created backups.
const FormatVersion uint32 = 2

// ActiveHeartbeatMaxAge is the inclusive activity window in seconds.
const ActiveHeartbeatMaxAge float64 = 90

// Failure classifies portable-backup validation and restore compensation errors.
type Failure struct{ Kind, Message string }

// Error prefixes the diagnostic with its stable operation category.
func (failure Failure) Error() string {
	prefixes := map[string]string{"input": "invalid backup input: ", "manifest": "invalid backup manifest: ", "unsupported": "unsupported backup: ", "integrity": "backup integrity error: ", "rollback": "restore rollback failed: "}
	return prefixes[failure.Kind] + failure.Message
}
func failure(kind, format string, values ...any) error {
	return Failure{kind, fmt.Sprintf(format, values...)}
}

// ErrActiveTarget prevents replacement while a service may still own the deployment.
var ErrActiveTarget = errors.New("restore refused because a recent API or worker heartbeat marks the target active")

// ErrManifestJson rejects invalid typed manifests before component validation.
var ErrManifestJson = errors.New("invalid backup manifest JSON")

// Selection explicitly controls complete group replacement during restore.
type Selection struct {
	Metadata       bool `json:"metadata"`
	IndexDatabases bool `json:"index_databases"`
	PushState      bool `json:"push_state"`
}

// Component records portable path, complete bytes and optional SQLite version.
type Component struct {
	Kind          string `json:"kind"`
	Path          string `json:"path"`
	Size          uint64 `json:"size"`
	Sha256        string `json:"sha256"`
	SchemaVersion *int64 `json:"schema_version,omitempty"`
}

// Manifest supports current v2 and historical v1 snapshots without metadata replacement.
type Manifest struct {
	Format     string      `json:"format"`
	Version    uint32      `json:"version"`
	CreatedAt  float64     `json:"created_at"`
	Selection  Selection   `json:"selection"`
	Components []Component `json:"components"`
}

// CreateOptions selects the source deployment and optional complete data groups.
type CreateOptions struct {
	Config                                  config.Config
	AuthDbPath, OutputDir                   string
	IncludeIndexDatabases, IncludePushState bool
}

// RestoreOptions identifies the verified backup and inactive target deployment.
type RestoreOptions struct {
	Config                config.Config
	AuthDbPath, BackupDir string
}

// RestoreReport counts replaced files and records the selected data groups.
type RestoreReport struct {
	RestoredFiles          int  `json:"restored_files"`
	RestoredDatabases      int  `json:"restored_databases"`
	RestoredMetadata       bool `json:"restored_metadata"`
	RestoredIndexDatabases bool `json:"restored_index_databases"`
	RestoredPushState      bool `json:"restored_push_state"`
}

// ServiceKind identifies the API or worker activity that blocks offline restore.
type ServiceKind string

const (
	Api    ServiceKind = "api"
	Worker ServiceKind = "worker"
)
