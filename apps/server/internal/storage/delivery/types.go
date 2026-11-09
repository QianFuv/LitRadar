// Package delivery persists notification runs, progress, leases and dedupe state.
package delivery

// Workflow is the persisted Workflow classification.
type Workflow string

const (
	WorkflowNotify Workflow = "notify"
	WorkflowPush   Workflow = "push"
)

func (value Workflow) valid() bool {
	switch value {
	case WorkflowNotify, WorkflowPush:
		return true
	}
	return false
}

// TriggerKind is the persisted TriggerKind classification.
type TriggerKind string

const (
	TriggerKindScheduled TriggerKind = "scheduled"
	TriggerKindManual    TriggerKind = "manual"
	TriggerKindLegacy    TriggerKind = "legacy"
)

func (value TriggerKind) valid() bool {
	switch value {
	case TriggerKindScheduled, TriggerKindManual, TriggerKindLegacy:
		return true
	}
	return false
}

// RunMode is the persisted RunMode classification.
type RunMode string

const (
	RunModeDryRun  RunMode = "dry_run"
	RunModeExecute RunMode = "execute"
)

func (value RunMode) valid() bool {
	switch value {
	case RunModeDryRun, RunModeExecute:
		return true
	}
	return false
}

// RunStatus is the persisted RunStatus classification.
type RunStatus string

const (
	RunStatusQueued     RunStatus = "queued"
	RunStatusClaimed    RunStatus = "claimed"
	RunStatusRunning    RunStatus = "running"
	RunStatusCancelling RunStatus = "cancelling"
	RunStatusCompleted  RunStatus = "completed"
	RunStatusFailed     RunStatus = "failed"
	RunStatusCancelled  RunStatus = "cancelled"
	RunStatusTimedOut   RunStatus = "timed_out"
	RunStatusSkipped    RunStatus = "skipped"
	RunStatusUnknown    RunStatus = "unknown"
)

func (value RunStatus) valid() bool {
	switch value {
	case RunStatusQueued, RunStatusClaimed, RunStatusRunning, RunStatusCancelling, RunStatusCompleted, RunStatusFailed, RunStatusCancelled, RunStatusTimedOut, RunStatusSkipped, RunStatusUnknown:
		return true
	}
	return false
}

// CheckpointStatus is the persisted CheckpointStatus classification.
type CheckpointStatus string

const (
	CheckpointStatusIdle      CheckpointStatus = "idle"
	CheckpointStatusRunning   CheckpointStatus = "running"
	CheckpointStatusCompleted CheckpointStatus = "completed"
	CheckpointStatusFailed    CheckpointStatus = "failed"
	CheckpointStatusSkipped   CheckpointStatus = "skipped"
	CheckpointStatusUnknown   CheckpointStatus = "unknown"
)

func (value CheckpointStatus) valid() bool {
	switch value {
	case CheckpointStatusIdle, CheckpointStatusRunning, CheckpointStatusCompleted, CheckpointStatusFailed, CheckpointStatusSkipped, CheckpointStatusUnknown:
		return true
	}
	return false
}

// ItemKind is the persisted ItemKind classification.
type ItemKind string

const (
	ItemKindIssue      ItemKind = "issue"
	ItemKindInPress    ItemKind = "inpress"
	ItemKindArticle    ItemKind = "article"
	ItemKindSubscriber ItemKind = "subscriber"
)

func (value ItemKind) valid() bool {
	switch value {
	case ItemKindIssue, ItemKindInPress, ItemKindArticle, ItemKindSubscriber:
		return true
	}
	return false
}

// ItemStatus is the persisted ItemStatus classification.
type ItemStatus string

const (
	ItemStatusPending   ItemStatus = "pending"
	ItemStatusClaimed   ItemStatus = "claimed"
	ItemStatusSending   ItemStatus = "sending"
	ItemStatusSucceeded ItemStatus = "succeeded"
	ItemStatusFailed    ItemStatus = "failed"
	ItemStatusSkipped   ItemStatus = "skipped"
	ItemStatusCancelled ItemStatus = "cancelled"
	ItemStatusUnknown   ItemStatus = "unknown"
)

func (value ItemStatus) valid() bool {
	switch value {
	case ItemStatusPending, ItemStatusClaimed, ItemStatusSending, ItemStatusSucceeded, ItemStatusFailed, ItemStatusSkipped, ItemStatusCancelled, ItemStatusUnknown:
		return true
	}
	return false
}

// DedupeStatus is the persisted DedupeStatus classification.
type DedupeStatus string

const (
	DedupeStatusReserved  DedupeStatus = "reserved"
	DedupeStatusConfirmed DedupeStatus = "confirmed"
	DedupeStatusUnknown   DedupeStatus = "unknown"
)

func (value DedupeStatus) valid() bool {
	switch value {
	case DedupeStatusReserved, DedupeStatusConfirmed, DedupeStatusUnknown:
		return true
	}
	return false
}

// CheckpointRecord preserves the durable delivery contract.
type CheckpointRecord struct {
	Id                 int64            `json:"id"`
	Workflow           Workflow         `json:"workflow"`
	DbName             string           `json:"db_name"`
	Status             CheckpointStatus `json:"status"`
	LegacyStatus       *string          `json:"legacy_status"`
	SnapshotJson       string           `json:"snapshot_json"`
	LastCompletedRunAt *string          `json:"last_completed_run_at"`
	Revision           int64            `json:"revision"`
	LegacySourceHash   *string          `json:"legacy_source_hash"`
	LegacySourceName   *string          `json:"legacy_source_name"`
	LegacyImportedAt   *float64         `json:"legacy_imported_at"`
	CreatedAt          float64          `json:"created_at"`
	UpdatedAt          float64          `json:"updated_at"`
}

// CheckpointUpdate preserves the durable delivery contract.
type CheckpointUpdate struct {
	Status             CheckpointStatus `json:"status"`
	SnapshotJson       string           `json:"snapshot_json"`
	LastCompletedRunAt *string          `json:"last_completed_run_at"`
	UpdatedAt          float64          `json:"updated_at"`
}

// RunCreate preserves the durable delivery contract.
type RunCreate struct {
	ExternalId  string      `json:"external_id"`
	Workflow    Workflow    `json:"workflow"`
	ScopeKey    string      `json:"scope_key"`
	DbName      *string     `json:"db_name"`
	TriggerKind TriggerKind `json:"trigger_kind"`
	Mode        RunMode     `json:"mode"`
	UserId      *int64      `json:"user_id"`
	DeadlineAt  *float64    `json:"deadline_at"`
	CreatedAt   float64     `json:"created_at"`
}

// RunRecord preserves the durable delivery contract.
type RunRecord struct {
	Id                    int64       `json:"id"`
	ExternalId            string      `json:"external_id"`
	Workflow              Workflow    `json:"workflow"`
	ScopeKey              string      `json:"scope_key"`
	DbName                *string     `json:"db_name"`
	TriggerKind           TriggerKind `json:"trigger_kind"`
	Mode                  RunMode     `json:"mode"`
	UserId                *int64      `json:"user_id"`
	Status                RunStatus   `json:"status"`
	LegacyStatus          *string     `json:"legacy_status"`
	OwnerId               *string     `json:"owner_id"`
	LeaseExpiresAt        *float64    `json:"lease_expires_at"`
	DeadlineAt            *float64    `json:"deadline_at"`
	CancellationRequested bool        `json:"cancellation_requested"`
	ResultJson            *string     `json:"result_json"`
	ErrorCode             *string     `json:"error_code"`
	Revision              int64       `json:"revision"`
	CreatedAt             float64     `json:"created_at"`
	StartedAt             *float64    `json:"started_at"`
	UpdatedAt             float64     `json:"updated_at"`
	FinishedAt            *float64    `json:"finished_at"`
}

// RunItemCreate preserves the durable delivery contract.
type RunItemCreate struct {
	ItemKind  ItemKind `json:"item_kind"`
	ItemKey   string   `json:"item_key"`
	UserId    *int64   `json:"user_id"`
	ArticleId *int64   `json:"article_id"`
}

// RunItemRecord preserves the durable delivery contract.
type RunItemRecord struct {
	Id             int64      `json:"id"`
	DeliveryRunId  int64      `json:"delivery_run_id"`
	ItemKind       ItemKind   `json:"item_kind"`
	ItemKey        string     `json:"item_key"`
	UserId         *int64     `json:"user_id"`
	ArticleId      *int64     `json:"article_id"`
	Status         ItemStatus `json:"status"`
	LegacyStatus   *string    `json:"legacy_status"`
	OwnerId        *string    `json:"owner_id"`
	LeaseExpiresAt *float64   `json:"lease_expires_at"`
	AttemptCount   int64      `json:"attempt_count"`
	ResultJson     *string    `json:"result_json"`
	ErrorCode      *string    `json:"error_code"`
	Revision       int64      `json:"revision"`
	CreatedAt      float64    `json:"created_at"`
	StartedAt      *float64   `json:"started_at"`
	UpdatedAt      float64    `json:"updated_at"`
	FinishedAt     *float64   `json:"finished_at"`
}

// DedupeRecord preserves the durable delivery contract.
type DedupeRecord struct {
	Id                int64        `json:"id"`
	Workflow          Workflow     `json:"workflow"`
	DbName            string       `json:"db_name"`
	UserId            int64        `json:"user_id"`
	ArticleId         int64        `json:"article_id"`
	DeliveryRunId     *int64       `json:"delivery_run_id"`
	Status            DedupeStatus `json:"status"`
	MessageId         *string      `json:"message_id"`
	ReservationOwner  *string      `json:"reservation_owner"`
	LegacyDeliveredAt *string      `json:"legacy_delivered_at"`
	Revision          int64        `json:"revision"`
	ReservedAt        float64      `json:"reserved_at"`
	DeliveredAt       *float64     `json:"delivered_at"`
	UpdatedAt         float64      `json:"updated_at"`
}

// DedupeResolution preserves the durable delivery contract.
type DedupeResolution struct {
	Id               int64 `json:"id"`
	ExpectedRevision int64 `json:"expected_revision"`
}

// LeaseRecord preserves the durable delivery contract.
type LeaseRecord struct {
	Id            int64    `json:"id"`
	Workflow      Workflow `json:"workflow"`
	DbName        string   `json:"db_name"`
	DeliveryRunId *int64   `json:"delivery_run_id"`
	OwnerId       *string  `json:"owner_id"`
	Revision      int64    `json:"revision"`
	AcquiredAt    *float64 `json:"acquired_at"`
	HeartbeatAt   *float64 `json:"heartbeat_at"`
	ExpiresAt     *float64 `json:"expires_at"`
	UpdatedAt     float64  `json:"updated_at"`
}

// LegacyImportResult preserves the durable delivery contract.
type LegacyImportResult struct {
	DiscoveredCount int `json:"discovered_count"`
	ImportedCount   int `json:"imported_count"`
	SkippedCount    int `json:"skipped_count"`
	ItemCount       int `json:"item_count"`
	DedupeCount     int `json:"dedupe_count"`
}

// RecoveryResult preserves the durable delivery contract.
type RecoveryResult struct {
	ResetItemCount      int `json:"reset_item_count"`
	UnknownItemCount    int `json:"unknown_item_count"`
	ReleasedDedupeCount int `json:"released_dedupe_count"`
	UnknownDedupeCount  int `json:"unknown_dedupe_count"`
}

// IsActive reports whether the run owns an execution lease.
func (status RunStatus) IsActive() bool {
	return status == RunStatusClaimed || status == RunStatusRunning || status == RunStatusCancelling
}

// IsTerminal reports a completed run state.
func (status RunStatus) IsTerminal() bool {
	return status.valid() && status != RunStatusQueued && !status.IsActive()
}

// IsTerminal reports a completed item state.
func (status ItemStatus) IsTerminal() bool {
	return status.valid() && status != ItemStatusPending && status != ItemStatusClaimed && status != ItemStatusSending
}
