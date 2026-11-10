// Package scheduler defines typed jobs and durable scheduler response contracts.
package scheduler

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/cron"
)

// Job contains exactly one allowlisted application command and its typed arguments.
type Job struct {
	Kind          string
	MetadataFile  *string
	Notify, Push  bool
	Database      *string
	MaxCandidates *uint64
}

// ValidationError identifies public job/timing rejections separately from corrupt persisted JSON.
type ValidationError struct{ Message string }

func (failure *ValidationError) Error() string { return failure.Message }

// MarshalJSON preserves the original tagged variant field order and omission rules.
func (job Job) MarshalJSON() ([]byte, error) {
	var value any
	switch job.Kind {
	case "index":
		value = IndexJob{job.Kind, job.MetadataFile, job.Notify, job.Push}
	case "notify", "push":
		value = DeliveryJob{job.Kind, job.Database, job.MaxCandidates}
	default:
		return nil, errors.New("invalid scheduled job kind")
	}
	encoded, err := jsonvalue.EncodeJson(value)
	return []byte(encoded), err
}

// UnmarshalJSON rejects unknown fields, duplicate fields, null booleans and lossy strings.
func (job *Job) UnmarshalJSON(data []byte) error {
	invalid := errors.New("invalid scheduled job JSON")
	fields, err := scheduledJobFields(data, invalid)
	if err != nil {
		return err
	}
	var result Job
	if raw, ok := fields["kind"]; !ok || json.Unmarshal(raw, &result.Kind) != nil || result.Kind == "" {
		return invalid
	}
	for name, raw := range fields {
		if name == "kind" {
			continue
		}
		target := scheduledJobFieldTarget(&result, name, raw)
		if target == nil || json.Unmarshal(raw, target) != nil {
			return invalid
		}
	}
	if !isScheduledJobKind(result.Kind) {
		return invalid
	}
	*job = result
	return nil
}

// scheduledJobFields validates the entire JSON before retaining duplicate-free object field bytes.
func scheduledJobFields(data []byte, invalid error) (map[string]json.RawMessage, error) {
	if !jsonvalue.ValidJson(string(data)) {
		return nil, invalid
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return nil, invalid
	}
	fields := map[string]json.RawMessage{}
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return nil, invalid
		}
		name, ok := token.(string)
		if !ok {
			return nil, invalid
		}
		if _, exists := fields[name]; exists {
			return nil, invalid
		}
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil {
			return nil, invalid
		}
		fields[name] = raw
	}
	return fields, nil
}

// scheduledJobFieldTarget admits only the active variant without invoking business validation.
func scheduledJobFieldTarget(job *Job, name string, raw json.RawMessage) any {
	if job.Kind == "index" {
		return indexJobFieldTarget(job, name, raw)
	}
	if job.Kind == "notify" || job.Kind == "push" {
		return deliveryJobFieldTarget(job, name)
	}
	return nil
}

// indexJobFieldTarget retains nullable metadata and nonnullable boolean fields.
func indexJobFieldTarget(job *Job, name string, raw json.RawMessage) any {
	switch name {
	case "metadata_file":
		return &job.MetadataFile
	case "notify", "push":
		if string(raw) == "null" {
			return nil
		}
		if name == "notify" {
			return &job.Notify
		}
		return &job.Push
	}
	return nil
}

// deliveryJobFieldTarget retains nullable database and uint64 candidate pointer fields.
func deliveryJobFieldTarget(job *Job, name string) any {
	switch name {
	case "database":
		return &job.Database
	case "max_candidates":
		return &job.MaxCandidates
	}
	return nil
}

// isScheduledJobKind recognizes the exact three decoded application variants.
func isScheduledJobKind(kind string) bool {
	return kind == "index" || kind == "notify" || kind == "push"
}

// Validate checks the original filename and candidate allowlists without normalizing values.
func (job Job) Validate() error {
	switch job.Kind {
	case "index":
		return validateFilename(job.MetadataFile, ".csv", "metadata file")
	case "notify", "push":
		if err := validateFilename(job.Database, ".sqlite", "database"); err != nil {
			return err
		}
		if job.MaxCandidates != nil && (*job.MaxCandidates < 1 || *job.MaxCandidates > 1000) {
			return &ValidationError{"max_candidates must be between 1 and 1000"}
		}
		return nil
	default:
		return &ValidationError{"invalid scheduled job kind"}
	}
}

func validateFilename(value *string, extension, label string) error {
	if value == nil {
		return nil
	}
	isAllowed := len(*value) > 0 && len(*value) <= 128 && !strings.HasPrefix(*value, ".") && !strings.Contains(*value, "..") && strings.HasSuffix(*value, extension)
	for _, character := range *value {
		if !isScheduledFilenameCharacter(character) {
			isAllowed = false
		}
	}
	if !isAllowed {
		return &ValidationError{label + " must be a safe " + extension + " basename"}
	}
	return nil
}

// ValidateTiming checks the explicit IANA zone before the execution deadline range.
func ValidateTiming(timezone string, timeout uint64) error {
	if _, err := cron.Location(timezone); err != nil {
		return &ValidationError{"timezone must be a valid IANA name"}
	}
	if timeout < 1 || timeout > 86400 {
		return &ValidationError{"timeout_seconds must be between 1 and 86400"}
	}
	return nil
}

// State is the stable persisted scheduler state.
type State string

// UnmarshalJSON accepts declared enum values while keeping permissive legacy mapping storage-only.
func (state *State) UnmarshalJSON(data []byte) error {
	invalid := errors.New("invalid scheduler state JSON")
	if !jsonvalue.ValidJson(string(data)) {
		return invalid
	}
	data = bytes.TrimSpace(data)
	name, err := scheduledStateWireName(data, invalid)
	if err != nil {
		return err
	}
	value := State(name)
	switch value {
	case Idle, Pending, Claimed, Running, Success, Failed, TimedOut, Error, Unknown, Cancelled:
		*state = value
		return nil
	}
	return invalid
}

const (
	Idle      State = "idle"
	Pending   State = "pending"
	Claimed   State = "claimed"
	Running   State = "running"
	Success   State = "success"
	Failed    State = "failed"
	TimedOut  State = "timed_out"
	Error     State = "error"
	Unknown   State = "unknown"
	Cancelled State = "cancelled"
)

// PersistedState retains legacy empty-idle and unrecognized-unknown mappings.
func PersistedState(value string) State {
	if value == "" {
		return Idle
	}
	state := State(value)
	switch state {
	case Idle, Pending, Claimed, Running, Success, Failed, TimedOut, Error, Cancelled:
		return state
	}
	return Unknown
}

// IsTerminal identifies states allowed to finalize claims.
func (state State) IsTerminal() bool {
	switch state {
	case Success, Failed, TimedOut, Error, Unknown, Cancelled:
		return true
	}
	return false
}

// Task is the persisted administrator task response, including retained legacy commands.
type Task struct {
	Id             int64    `json:"id" nullable:"false" format:"int64"`
	Name           string   `json:"name" nullable:"false"`
	Job            *Job     `json:"job" required:"false" allowNull:"true" nullable:"false"`
	LegacyCommand  *string  `json:"legacy_command" required:"false" nullable:"true"`
	Cron           string   `json:"cron" nullable:"false"`
	Timezone       string   `json:"timezone" nullable:"false"`
	TimeoutSeconds uint64   `json:"timeout_seconds" nullable:"false" minimum:"0" format:"int64"`
	Coalesce       bool     `json:"coalesce" nullable:"false"`
	Enabled        bool     `json:"enabled" nullable:"false"`
	LastRunAt      *float64 `json:"last_run_at" required:"false" nullable:"true" format:"double"`
	LastStatus     State    `json:"last_status" nullable:"false" enum:"idle,pending,claimed,running,success,failed,timed_out,error,unknown,cancelled"`
	CreatedAt      float64  `json:"created_at" nullable:"false" format:"double"`
	UpdatedAt      float64  `json:"updated_at" nullable:"false" format:"double"`
}

// Create specifies all effective task creation values after API defaulting.
type Create struct {
	Name              string
	Job               Job
	Cron, Timezone    string
	TimeoutSeconds    uint64
	Coalesce, Enabled bool
}

// Update distinguishes omitted task replacements from explicitly false values.
type Update struct {
	TaskId            int64
	Name              *string
	Job               *Job
	Cron, Timezone    *string
	TimeoutSeconds    *uint64
	Coalesce, Enabled *bool
}

// Run is the bounded public task execution history without captured output.
type Run struct {
	Id           int64    `json:"id" nullable:"false" format:"int64"`
	TaskId       int64    `json:"task_id" nullable:"false" format:"int64"`
	TaskName     string   `json:"task_name" nullable:"false"`
	ScheduledFor int64    `json:"scheduled_for" nullable:"false" format:"int64"`
	Status       State    `json:"status" nullable:"false" enum:"idle,pending,claimed,running,success,failed,timed_out,error,unknown,cancelled"`
	WorkerId     *string  `json:"worker_id" required:"false" nullable:"true"`
	ClaimedAt    *float64 `json:"claimed_at" required:"false" nullable:"true" format:"double"`
	StartedAt    *float64 `json:"started_at" required:"false" nullable:"true" format:"double"`
	FinishedAt   *float64 `json:"finished_at" required:"false" nullable:"true" format:"double"`
}

// Worker reports persisted heartbeat health against the caller's wall clock.
type Worker struct {
	WorkerId    string  `json:"worker_id" nullable:"false"`
	StartedAt   float64 `json:"started_at" nullable:"false" format:"double"`
	HeartbeatAt float64 `json:"heartbeat_at" nullable:"false" format:"double"`
	IsHealthy   bool    `json:"is_healthy" nullable:"false"`
}

// Status combines the durable cursor, worker health and bounded recent history.
type Status struct {
	LastCheckedAt *float64 `json:"last_checked_at" required:"false" nullable:"true" format:"double"`
	Workers       []Worker `json:"workers" nullable:"false"`
	RecentRuns    []Run    `json:"recent_runs" nullable:"false"`
}

// isScheduledFilenameCharacter preserves the ASCII-only basename alphabet without normalization.
func isScheduledFilenameCharacter(character rune) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || character == '.' || character == '_' || character == '-'
}

// scheduledStateWireName reads a validated string or exactly one externally tagged null state.
func scheduledStateWireName(data []byte, invalid error) (string, error) {
	var name string
	if data[0] == '"' {
		if json.Unmarshal(data, &name) != nil {
			return "", invalid
		}
	} else if data[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.Token()
		if !decoder.More() {
			return "", invalid
		}
		token, err := decoder.Token()
		if err != nil {
			return "", invalid
		}
		var ok bool
		name, ok = token.(string)
		if !ok {
			return "", invalid
		}
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil || !bytes.Equal(raw, []byte("null")) || decoder.More() {
			return "", invalid
		}
	} else {
		return "", invalid
	}
	return name, nil
}

// IndexJob is the serialized index command and its arguments.
type IndexJob struct {
	// Kind identifies the index command.
	Kind string `json:"kind" enum:"index"`
	// MetadataFile optionally selects one catalog file.
	MetadataFile *string `json:"metadata_file,omitempty" nullable:"true" maxLength:"128" pattern:"^[A-Za-z0-9_-]+(?:\\.[A-Za-z0-9_-]+)*\\.csv$"`
	// Notify schedules notification after indexing.
	Notify bool `json:"notify" required:"false"`
	// Push schedules delivery after indexing.
	Push bool `json:"push" required:"false"`
}

// DeliveryJob is the serialized notification or delivery command.
type DeliveryJob struct {
	// Kind identifies notification or delivery.
	Kind string `json:"kind" enum:"notify,push"`
	// Database optionally selects one source database.
	Database *string `json:"database,omitempty" nullable:"true" maxLength:"128" pattern:"^[A-Za-z0-9_-]+(?:\\.[A-Za-z0-9_-]+)*\\.sqlite$"`
	// MaxCandidates optionally bounds candidate selection.
	MaxCandidates *uint64 `json:"max_candidates,omitempty" nullable:"true" minimum:"1" maximum:"1000"`
}
