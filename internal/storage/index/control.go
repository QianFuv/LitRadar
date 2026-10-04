package index

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"unicode"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// ControlSchemaVersion is the version of the disposable provider control store.
const ControlSchemaVersion = 5

// SyncScope selects one maintained journal inside a catalog/provider namespace.
type SyncScope struct{ CatalogName, ProviderName, CatalogId string }

// SyncRun binds disposable progress to the frozen owner, mode and successful base anchor.
type SyncRun struct {
	Scope          SyncScope
	BatchId, RunId string
	Mode           domain.IndexSyncMode
	BaseAnchor     *string
}

// ProviderSyncAnchor represents completed coverage, including completion without a reusable anchor.
type ProviderSyncAnchor struct {
	CommittedAnchor  *string `json:"committed_anchor"`
	CompletedAt      string  `json:"completed_at"`
	CompletedBatchId *string `json:"completed_batch_id"`
}

// ProviderRunCheckpoint preserves the first attempt's epoch while ownership and progress advance.
type ProviderRunCheckpoint struct {
	BatchId             *string              `json:"batch_id"`
	RunId               string               `json:"run_id"`
	Mode                domain.IndexSyncMode `json:"mode"`
	BaseAnchor          *string              `json:"base_anchor"`
	TraversalCheckpoint *string              `json:"traversal_checkpoint"`
	StartedAt           string               `json:"started_at"`
	UpdatedAt           string               `json:"updated_at"`
}

// Format excludes opaque state from diagnostic formatting, including pointer and Go-syntax forms.
func (value ProviderRunCheckpoint) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "ProviderRunCheckpoint{has_batch:%t run:%q mode:%s has_base:%t has_checkpoint:%t started:%q updated:%q}", value.BatchId != nil, value.RunId, value.Mode, value.BaseAnchor != nil, value.TraversalCheckpoint != nil, value.StartedAt, value.UpdatedAt)
}

// Format exposes only the presence of opaque completed state.
func (value ProviderSyncAnchor) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "ProviderSyncAnchor{has_anchor:%t completed:%q has_batch:%t}", value.CommittedAnchor != nil, value.CompletedAt, value.CompletedBatchId != nil)
}

// Format retains safe ownership information without exposing the frozen provider anchor.
func (value SyncRun) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "SyncRun{scope:%v batch:%q run:%q mode:%s has_base:%t}", value.Scope, value.BatchId, value.RunId, value.Mode, value.BaseAnchor != nil)
}

// JournalSyncPreparation skips only same-batch completed coverage when resume is enabled.
type JournalSyncPreparation struct {
	ShouldSkip bool                   `json:"skip"`
	Checkpoint *ProviderRunCheckpoint `json:"checkpoint"`
}

// ControlError exposes stable recovery classifications and safe orchestration metadata.
type ControlError struct {
	Kind, Reason, RunId     string
	FoundVersion, ExpiresAt int64
	Stored, Requested       domain.IndexSyncMode
}

func (err *ControlError) Error() string {
	switch err.Kind {
	case "unsupported_version":
		return fmt.Sprintf("unsupported index control schema version %d; maximum supported is %d", err.FoundVersion, ControlSchemaVersion)
	case "active_lease":
		return fmt.Sprintf("index control scope is owned by active run %s until %d", err.RunId, err.ExpiresAt)
	case "ownership_lost":
		return fmt.Sprintf("index run %s no longer owns its control lease", err.RunId)
	case "run_ownership_lost":
		return fmt.Sprintf("index run %s no longer owns the frozen journal synchronization state", err.RunId)
	case "mode_mismatch":
		return fmt.Sprintf("in-flight journal mode %s does not match requested mode %s; retry the original mode or disable resume", debugMode(err.Stored), debugMode(err.Requested))
	case "batch_mismatch":
		return "in-flight journal state does not belong to the active index batch; retry the original batch or disable resume"
	default:
		return "invalid disposable synchronization state: " + err.Reason
	}
}

func debugMode(mode domain.IndexSyncMode) string {
	switch mode {
	case domain.Bootstrap:
		return "Bootstrap"
	case domain.Incremental:
		return "Incremental"
	case domain.FullRescan:
		return "FullRescan"
	default:
		return string(mode)
	}
}
func invalidSync(reason string) error { return &ControlError{Kind: "invalid_state", Reason: reason} }
func validateOpaque(value *string, field string) error {
	if value == nil {
		return nil
	}
	if *value == "" {
		return invalidSync(field)
	}
	if len(*value) > 65536 {
		return invalidSync("opaque synchronization state exceeds 65,536 bytes")
	}
	if !utf8.ValidString(*value) {
		return invalidSync("opaque synchronization state is not UTF-8")
	}
	return nil
}
func validateRunMetadata(run, timestamp string) error {
	if run == "" || timestamp == "" {
		return invalidSync("run id and timestamp must not be empty")
	}
	return nil
}
func validateBatchId(value string) error {
	if value == "" || len(value) > 512 {
		return invalidSync("batch id must be non-empty and bounded")
	}
	for _, character := range value {
		if unicode.IsControl(character) {
			return invalidSync("batch id must be non-empty and bounded")
		}
	}
	return nil
}

// OpenControl creates only disposable parent directories and reads version before connection policy.
func OpenControl(ctx context.Context, path string) (*Connection, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0777); err != nil {
		return nil, err
	}
	database, err := sqlite.OpenMigration(path)
	if err != nil {
		return nil, err
	}
	physical, err := database.Conn(ctx)
	if err != nil {
		database.Close()
		return nil, err
	}
	connection := &Connection{physical, database}
	if _, err := physical.ExecContext(ctx, "PRAGMA busy_timeout=30000"); err != nil {
		connection.Close()
		return nil, err
	}
	if err := InitControl(ctx, physical); err != nil {
		connection.Close()
		return nil, err
	}
	return connection, nil
}

// ReadSyncAnchor validates bounded opaque data without interpreting provider-specific state.
func ReadSyncAnchor(ctx context.Context, connection *sql.Conn, scope SyncScope) (*ProviderSyncAnchor, error) {
	var anchor, batch sqlite.OptionalText
	var completed sqlite.Text
	err := connection.QueryRowContext(ctx, "SELECT committed_anchor,completed_at,completed_batch_id FROM provider_sync_anchors WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3", scope.CatalogName, scope.ProviderName, scope.CatalogId).Scan(&anchor, &completed, &batch)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if err := validateOpaque(anchor.Value, "committed anchor"); err != nil {
		return nil, err
	}
	return &ProviderSyncAnchor{anchor.Value, string(completed), batch.Value}, nil
}

// ReadRunCheckpoint retains stored mode and first-attempt time for resumable provider traversal.
func ReadRunCheckpoint(ctx context.Context, connection *sql.Conn, scope SyncScope) (*ProviderRunCheckpoint, error) {
	var batch, base, checkpoint sqlite.OptionalText
	var run, mode, started, updated sqlite.Text
	err := connection.QueryRowContext(ctx, "SELECT batch_id,run_id,sync_mode,base_anchor,traversal_checkpoint,started_at,updated_at FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3", scope.CatalogName, scope.ProviderName, scope.CatalogId).Scan(&batch, &run, &mode, &base, &checkpoint, &started, &updated)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if err := validateOpaque(base.Value, "base anchor"); err != nil {
		return nil, err
	}
	if err := validateOpaque(checkpoint.Value, "traversal checkpoint"); err != nil {
		return nil, err
	}
	parsed := domain.IndexSyncMode(mode)
	if parsed != domain.Bootstrap && parsed != domain.Incremental && parsed != domain.FullRescan {
		return nil, invalidSync("stored synchronization mode is invalid")
	}
	return &ProviderRunCheckpoint{batch.Value, string(run), parsed, base.Value, checkpoint.Value, string(started), string(updated)}, nil
}
