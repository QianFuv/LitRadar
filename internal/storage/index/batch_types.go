package index

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"math"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// BatchSchemaVersion identifies the project-level orchestration state layout.
const BatchSchemaVersion = 2

// BatchDatabaseFilename keeps disposable orchestration separate from canonical content databases.
const BatchDatabaseFilename = "index-batches.sqlite"

// CatalogInput retains entries and a digest from one immutable catalog read.
type CatalogInput struct {
	Path, Filename, CatalogName, CsvSha256, ProviderName string
	Entries                                              []domain.JournalCatalogEntry
}

// Format excludes filesystem paths and CSV digests from diagnostics.
func (value CatalogInput) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "CatalogInput{file_name:%q catalog_name:%q provider_name:%q journal_count:%d}", value.Filename, value.CatalogName, value.ProviderName, len(value.Entries))
}

// BatchRequest contains only inputs whose changes invalidate resumable work.
type BatchRequest struct {
	Catalogs                     []CatalogInput
	Selection                    string
	Mode                         domain.IndexSyncMode
	IssueBatchSize               uint64
	ShouldNotify, IsNotifyDryRun bool
}

// NewBatchRequest validates catalog ownership and issue-batch size in canonical error order.
func NewBatchRequest(catalogs []CatalogInput, selection string, mode domain.IndexSyncMode, issueBatchSize uint64, shouldNotify, isNotifyDryRun bool) (BatchRequest, error) {
	request := BatchRequest{catalogs, selection, mode, issueBatchSize, shouldNotify, isNotifyDryRun}
	if len(catalogs) == 0 {
		return BatchRequest{}, batchInput("an index batch must contain at least one catalog")
	}
	if selection == "explicit_file" && len(catalogs) != 1 {
		return BatchRequest{}, batchInput("an explicit-file batch must contain exactly one catalog")
	}
	if issueBatchSize == 0 {
		return BatchRequest{}, batchInput("issue batch size must be greater than zero")
	}
	filenames, names := map[string]bool{}, map[string]bool{}
	for _, catalog := range catalogs {
		for _, field := range [][2]string{{catalog.Filename, "catalog filename must be non-empty and bounded"}, {catalog.CatalogName, "catalog name must be non-empty and bounded"}, {catalog.ProviderName, "provider route must be non-empty and bounded"}} {
			if err := validateIdentifier(field[0], field[1]); err != nil {
				return BatchRequest{}, err
			}
		}
		if len(catalog.CsvSha256) != 64 || !isHexDigest(catalog.CsvSha256) {
			return BatchRequest{}, batchInput("catalog digest must be a SHA-256 hexadecimal value")
		}
		if filenames[catalog.Filename] {
			return BatchRequest{}, batchInput("catalog filenames must be unique within one batch")
		}
		filenames[catalog.Filename] = true
		if names[catalog.CatalogName] {
			return BatchRequest{}, batchInput("catalog names must be unique within one batch")
		}
		names[catalog.CatalogName] = true
	}
	if selection != "all" && selection != "explicit_file" {
		return BatchRequest{}, batchInput("catalog selection is invalid")
	}
	if mode != domain.Bootstrap && mode != domain.Incremental && mode != domain.FullRescan {
		return BatchRequest{}, batchInput("synchronization mode is invalid")
	}
	return request, nil
}

// Fingerprint uses length-prefixed UTF-8 fields and ordered catalog descriptors, preserving the Rust hash contract.
func (request BatchRequest) Fingerprint() string {
	hash := sha256.New()
	field := func(value string) {
		var size [8]byte
		binary.LittleEndian.PutUint64(size[:], uint64(len(value)))
		hash.Write(size[:])
		hash.Write([]byte(value))
	}
	field("litradar-index-batch-v1")
	field(request.Selection)
	field(string(request.Mode))
	field(strconv.FormatUint(request.IssueBatchSize, 10))
	field(boolDigit(request.ShouldNotify))
	field(boolDigit(request.IsNotifyDryRun))
	for ordinal, catalog := range request.Catalogs {
		field(strconv.Itoa(ordinal))
		field(catalog.Filename)
		field(catalog.CatalogName)
		field(catalog.CsvSha256)
		field(catalog.ProviderName)
	}
	return hex.EncodeToString(hash.Sum(nil))
}

// CatalogPhase describes the durable publication and notification boundary for one catalog.
type CatalogPhase string

const (
	CatalogPending           CatalogPhase = "pending"
	CatalogIndexing          CatalogPhase = "indexing"
	CatalogManifestPrepared  CatalogPhase = "manifest_prepared"
	CatalogManifestPublished CatalogPhase = "manifest_published"
	CatalogNotifying         CatalogPhase = "notifying"
	CatalogCompleted         CatalogPhase = "completed"
)

// NotifyStatus distinguishes ambiguous outcomes from safe terminal retry and success states.
type NotifyStatus string

const (
	NotifyRunning   NotifyStatus = "running"
	NotifyIdle      NotifyStatus = "idle"
	NotifyCompleted NotifyStatus = "completed"
	NotifySkipped   NotifyStatus = "skipped"
	NotifyFailed    NotifyStatus = "failed"
	NotifyCancelled NotifyStatus = "cancelled"
	NotifyTimedOut  NotifyStatus = "timed_out"
	NotifyUnknown   NotifyStatus = "unknown"
)

// IsSuccess permits catalog completion only after a trusted successful terminal outcome.
func (status NotifyStatus) IsSuccess() bool {
	return status == NotifyIdle || status == NotifyCompleted || status == NotifySkipped
}
func (status NotifyStatus) canRetry() bool {
	return status == NotifyFailed || status == NotifyCancelled || status == NotifyTimedOut
}

// NotifyHandoffState retains stable dedupe identity and explicit acknowledgement of a previous Unknown attempt.
type NotifyHandoffState struct {
	AttemptId                    string
	Status                       NotifyStatus
	ExitCode                     *int32
	UnknownAcknowledgedAttemptId *string
	UnknownAcknowledgedAt        *int64
}

// NotifyAttemptPreparation determines whether to launch, reuse success, or require Unknown acknowledgement.
type NotifyAttemptPreparation struct {
	Decision string
	State    NotifyHandoffState
}

// BatchCatalogOutcome preserves completed indexing counters during publication and notification recovery.
type BatchCatalogOutcome struct {
	RunId               string
	JournalCount        uint64
	WrittenArticleCount int64
	SourceAttemptCount  uint64
	ManifestPath        *string
}

// ManifestIntent preserves immutable publication bytes so ACK recovery never rebuilds from depleted outbox rows.
type ManifestIntent struct {
	Payload                  []byte
	Sha256                   string
	ThroughEventId           *int64
	Path, RunId, GeneratedAt string
}

// Format excludes publication bytes, digests and directory components.
func (value ManifestIntent) Format(state fmt.State, verb rune) {
	fmt.Fprintf(state, "ManifestIntent{payload_bytes:%d has_cursor:%t file_name:%q run_id:%q generated_at:%q}", len(value.Payload), value.ThroughEventId != nil, filepath.Base(value.Path), value.RunId, value.GeneratedAt)
}

// NewManifestIntent validates bounded metadata without interpreting the already prepared JSON bytes.
func NewManifestIntent(payload []byte, through *int64, path, run, generated string) (ManifestIntent, error) {
	if len(payload) == 0 || len(payload) > 64*1024*1024 {
		return ManifestIntent{}, batchInput("manifest payload must be non-empty and bounded")
	}
	if through != nil && *through <= 0 {
		return ManifestIntent{}, batchInput("manifest outbox cursor must be positive")
	}
	if err := validateRelativePath(path); err != nil {
		return ManifestIntent{}, err
	}
	if err := validateIdentifier(run, "manifest run identifier must be non-empty and bounded"); err != nil {
		return ManifestIntent{}, err
	}
	if err := validateIdentifier(generated, "manifest timestamp must be non-empty and bounded"); err != nil {
		return ManifestIntent{}, err
	}
	return ManifestIntent{slices.Clone(payload), sha256Hex(payload), copyValue(through), path, run, generated}, nil
}

// IndexBatchCatalog is the persisted phase and recoverable output of one ordered catalog.
type IndexBatchCatalog struct {
	Ordinal                             uint64
	Filename, CatalogName, ProviderName string
	JournalCount                        uint64
	Phase                               CatalogPhase
	Outcome                             *BatchCatalogOutcome
	ManifestIntent                      *ManifestIntent
	NotifyHandoff                       *NotifyHandoffState
}

// IndexBatch is the project-wide batch currently fenced by one invocation owner.
type IndexBatch struct {
	BatchId, OwnerId string
	StartedAt        int64
	DidResume        bool
	Catalogs         []IndexBatchCatalog
}

// BatchAdmission distinguishes executable work from durable abandonment awaiting checkpoint cleanup.
type BatchAdmission struct {
	IsAbandoning bool
	Batch        IndexBatch
}

// BatchError carries bounded recovery decisions without provider payloads, secrets or fingerprints.
type BatchError struct {
	Kind, Reason, OwnerId   string
	FoundVersion, ExpiresAt int64
	Fields                  []string
}

func (err *BatchError) Error() string {
	switch err.Kind {
	case "unsupported_version":
		return fmt.Sprintf("unsupported index batch schema version %d; maximum supported is %d", err.FoundVersion, BatchSchemaVersion)
	case "compatibility":
		return "active index batch is incompatible in: " + strings.Join(err.Fields, ", ") + "; restore the original inputs or disable resume"
	case "active_lease":
		return fmt.Sprintf("index batch is owned by active invocation %s until %d", err.OwnerId, err.ExpiresAt)
	case "ownership_lost":
		return "index invocation " + err.OwnerId + " no longer owns the project batch lease"
	case "abandonment_pending":
		return "an index batch abandonment is incomplete; retry with resume disabled"
	case "published_notification_pending":
		return "active index batch has a published notification handoff; resume it and acknowledge Unknown if required before disabling resume"
	case "input":
		return "invalid index batch input: " + err.Reason
	default:
		return "invalid disposable index batch state: " + err.Reason
	}
}

func batchInput(reason string) error { return &BatchError{Kind: "input", Reason: reason} }
func batchState(reason string) error { return &BatchError{Kind: "state", Reason: reason} }
func validateIdentifier(value, reason string) error {
	if value == "" || len(value) > 512 {
		return batchInput(reason)
	}
	for _, character := range value {
		if unicode.IsControl(character) {
			return batchInput(reason)
		}
	}
	return nil
}
func validateRelativePath(value string) error {
	if err := validateIdentifier(value, "manifest path must be non-empty and bounded"); err != nil {
		return err
	}
	normalized := value
	if runtime.GOOS == "windows" {
		normalized = strings.ReplaceAll(value, `\`, "/")
	}
	hasDrivePrefix := runtime.GOOS == "windows" && len(value) >= 2 && value[1] == ':' && (value[0] >= 'A' && value[0] <= 'Z' || value[0] >= 'a' && value[0] <= 'z')
	if hasDrivePrefix || strings.HasPrefix(normalized, "/") {
		return batchInput("manifest path must remain project-relative")
	}
	for _, part := range strings.Split(normalized, "/") {
		if part == ".." {
			return batchInput("manifest path must remain project-relative")
		}
	}
	return nil
}
func isHexDigest(value string) bool {
	for _, character := range []byte(value) {
		if !(character >= '0' && character <= '9' || character >= 'a' && character <= 'f' || character >= 'A' && character <= 'F') {
			return false
		}
	}
	return true
}
func sha256Hex(value []byte) string { sum := sha256.Sum256(value); return hex.EncodeToString(sum[:]) }
func boolDigit(value bool) string {
	if value {
		return "1"
	}
	return "0"
}
func batchLeaseExpiry(now int64) int64 {
	if now > math.MaxInt64-300 {
		return math.MaxInt64
	}
	return now + 300
}
func sqliteCount(value uint64) (int64, error) {
	if value > math.MaxInt64 {
		return 0, batchInput("batch count exceeds SQLite integer capacity")
	}
	return int64(value), nil
}
func storedCount(value int64) (uint64, error) {
	if value < 0 {
		return 0, batchState("stored batch count is invalid")
	}
	return uint64(value), nil
}

func sameOptional[T comparable](first, second *T) bool {
	if first == nil || second == nil {
		return first == nil && second == nil
	}
	return *first == *second
}
