// Package scheduler defines typed jobs and durable scheduler response contracts.
package scheduler

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/platform/cron"
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
		value = struct {
			Kind         string  `json:"kind"`
			MetadataFile *string `json:"metadata_file,omitempty"`
			Notify       bool    `json:"notify"`
			Push         bool    `json:"push"`
		}{job.Kind, job.MetadataFile, job.Notify, job.Push}
	case "notify", "push":
		value = struct {
			Kind          string  `json:"kind"`
			Database      *string `json:"database,omitempty"`
			MaxCandidates *uint64 `json:"max_candidates,omitempty"`
		}{job.Kind, job.Database, job.MaxCandidates}
	default:
		return nil, errors.New("invalid scheduled job kind")
	}
	encoded, err := jsonvalue.EncodeJson(value)
	return []byte(encoded), err
}

// UnmarshalJSON rejects unknown fields, duplicate fields, null booleans and lossy strings.
func (job *Job) UnmarshalJSON(data []byte) error {
	invalid := errors.New("invalid scheduled job JSON")
	if !jsonvalue.ValidJson(string(data)) {
		return invalid
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return invalid
	}
	fields := map[string]json.RawMessage{}
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return invalid
		}
		name, ok := token.(string)
		if !ok {
			return invalid
		}
		if _, exists := fields[name]; exists {
			return invalid
		}
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil {
			return invalid
		}
		fields[name] = raw
	}
	var result Job
	if raw, ok := fields["kind"]; !ok || json.Unmarshal(raw, &result.Kind) != nil || result.Kind == "" {
		return invalid
	}
	for name, raw := range fields {
		var target any
		switch name {
		case "kind":
			continue
		case "metadata_file":
			if result.Kind == "index" {
				target = &result.MetadataFile
			}
		case "notify":
			if result.Kind == "index" && string(raw) != "null" {
				target = &result.Notify
			}
		case "push":
			if result.Kind == "index" && string(raw) != "null" {
				target = &result.Push
			}
		case "database":
			if result.Kind == "notify" || result.Kind == "push" {
				target = &result.Database
			}
		case "max_candidates":
			if result.Kind == "notify" || result.Kind == "push" {
				target = &result.MaxCandidates
			}
		}
		if target == nil || json.Unmarshal(raw, target) != nil {
			return invalid
		}
	}
	if result.Kind != "index" && result.Kind != "notify" && result.Kind != "push" {
		return invalid
	}
	*job = result
	return nil
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
		if !(character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || character == '.' || character == '_' || character == '-') {
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
	var name string
	if data[0] == '"' {
		if json.Unmarshal(data, &name) != nil {
			return invalid
		}
	} else if data[0] == '{' {
		decoder := json.NewDecoder(bytes.NewReader(data))
		decoder.Token()
		if !decoder.More() {
			return invalid
		}
		token, err := decoder.Token()
		if err != nil {
			return invalid
		}
		var ok bool
		name, ok = token.(string)
		if !ok {
			return invalid
		}
		var raw json.RawMessage
		if decoder.Decode(&raw) != nil || !bytes.Equal(raw, []byte("null")) || decoder.More() {
			return invalid
		}
	} else {
		return invalid
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
	Id             int64    `json:"id"`
	Name           string   `json:"name"`
	Job            *Job     `json:"job"`
	LegacyCommand  *string  `json:"legacy_command"`
	Cron           string   `json:"cron"`
	Timezone       string   `json:"timezone"`
	TimeoutSeconds uint64   `json:"timeout_seconds"`
	Coalesce       bool     `json:"coalesce"`
	Enabled        bool     `json:"enabled"`
	LastRunAt      *float64 `json:"last_run_at"`
	LastStatus     State    `json:"last_status"`
	CreatedAt      float64  `json:"created_at"`
	UpdatedAt      float64  `json:"updated_at"`
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
	Id           int64    `json:"id"`
	TaskId       int64    `json:"task_id"`
	TaskName     string   `json:"task_name"`
	ScheduledFor int64    `json:"scheduled_for"`
	Status       State    `json:"status"`
	WorkerId     *string  `json:"worker_id"`
	ClaimedAt    *float64 `json:"claimed_at"`
	StartedAt    *float64 `json:"started_at"`
	FinishedAt   *float64 `json:"finished_at"`
}

// Worker reports persisted heartbeat health against the caller's wall clock.
type Worker struct {
	WorkerId    string  `json:"worker_id"`
	StartedAt   float64 `json:"started_at"`
	HeartbeatAt float64 `json:"heartbeat_at"`
	IsHealthy   bool    `json:"is_healthy"`
}

// Status combines the durable cursor, worker health and bounded recent history.
type Status struct {
	LastCheckedAt *float64 `json:"last_checked_at"`
	Workers       []Worker `json:"workers"`
	RecentRuns    []Run    `json:"recent_runs"`
}
