package delivery

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	storageconfig "github.com/QianFuv/LitRadar/internal/storage/config"
)

type legacyInput struct {
	workflow               Workflow
	sourceName, sourceHash string
	state                  legacyState
}

func collectLegacy(root string) ([]legacyInput, error) {
	inputs := []legacyInput{}
	for _, directory := range []struct {
		name     string
		workflow Workflow
	}{{"push_state", WorkflowNotify}, {"folder_push_state", WorkflowPush}} {
		found, err := collectLegacyDirectory(root, directory.name, directory.workflow)
		if err != nil {
			return nil, err
		}
		inputs = append(inputs, found...)

	}
	sort.Slice(inputs, func(left, right int) bool {
		if inputs[left].workflow != inputs[right].workflow {
			return inputs[left].workflow < inputs[right].workflow
		}
		return inputs[left].sourceName < inputs[right].sourceName
	})
	return inputs, nil
}

func sortedKeys[V any](values map[string]V) []string {
	result := make([]string, 0, len(values))
	for key := range values {
		result = append(result, key)
	}
	sort.Strings(result)
	return result
}

func legacyNumeric(value string) (int64, error) {
	parsed, err := strconv.ParseInt(value, 10, 64)
	if err != nil || parsed <= 0 {
		return 0, &Error{Kind: "invalid_legacy"}
	}
	return parsed, nil
}

func legacyPair(value string) (int64, int64, error) {
	left, right, exists := strings.Cut(value, ":")
	if !exists {
		return 0, 0, &Error{Kind: "invalid_legacy"}
	}
	first, err := legacyNumeric(left)
	if err != nil {
		return 0, 0, err
	}
	second, err := legacyNumeric(right)
	return first, second, err
}

func validateLegacy(name string, state legacyState) error {
	if err := validateDbName(state.DbName); err != nil {
		return err
	}
	if name != strings.TrimSuffix(state.DbName, ".sqlite")+".json" {
		return &Error{Kind: "invalid_legacy"}
	}
	if err := validateOptionalText(state.LastCompletedRunAt, 128, "Legacy completion timestamp is invalid"); err != nil {
		return err
	}
	if err := validateLegacySnapshot(state.Snapshot); err != nil {
		return err
	}
	if state.Run != nil {
		if err := validateLegacyRun(*state.Run); err != nil {
			return err
		}
	}
	for _, key := range sortedKeys(state.DeliveryDedupe) {
		if _, _, err := legacyPair(key); err != nil {
			return err
		}
		if err := validateText(state.DeliveryDedupe[key], 128, "Legacy dedupe timestamp is invalid"); err != nil {
			return err
		}
	}
	return nil
}

// ImportLegacyFiles validates every source before atomically importing the scan, preserving source bytes and raw-byte hashes.
func ImportLegacyFiles(ctx context.Context, config storageconfig.Config, now float64) (LegacyImportResult, error) {
	if err := validateTime(now, "Legacy delivery import time is invalid"); err != nil {
		return LegacyImportResult{}, err
	}
	started := time.Now()
	inputs, err := collectLegacy(config.ProjectRoot)
	if err != nil {
		emitLegacyImportFailure(ctx, started, err)
		return LegacyImportResult{}, err
	}
	repository, err := Open(config.AuthDbPath)
	if err != nil {
		return LegacyImportResult{}, err
	}
	defer repository.Close()
	result, err := repository.importLegacy(ctx, inputs, now)
	if err != nil {
		emitLegacyImportFailure(ctx, started, err)
		return result, err
	}
	slog.InfoContext(ctx, "", "event", "delivery.legacy_import.completed", "component", "delivery", "outcome", "success", "discovered_count", result.DiscoveredCount, "imported_count", result.ImportedCount, "skipped_count", result.SkippedCount, "item_count", result.ItemCount, "dedupe_count", result.DedupeCount, "duration_ms", time.Since(started).Milliseconds())
	return result, nil
}

func emitLegacyImportFailure(ctx context.Context, started time.Time, err error) {
	kind := legacyImportFailureKind(err)
	slog.ErrorContext(ctx, "", "event", "delivery.legacy_import.failed", "component", "delivery", "outcome", "failure", "error_kind", kind, "duration_ms", time.Since(started).Milliseconds())
}

func collectLegacyDirectory(root, name string, workflow Workflow) ([]legacyInput, error) {
	inputs := []legacyInput{}
	path := filepath.Join(root, "data", name)
	entries, err := readLegacyDirectory(path)
	if err != nil {
		return nil, err
	}
	for _, entry := range entries {
		name := entry.Name()
		if !utf8.ValidString(name) {
			return nil, &Error{Kind: "invalid_legacy"}
		}
		if !strings.HasSuffix(name, ".json") || strings.HasSuffix(name, ".changes.json") {
			continue
		}
		input, err := readLegacySource(path, name, workflow)
		if err != nil {
			return nil, err
		}
		inputs = append(inputs, input)
	}
	return inputs, nil
}

func readLegacySource(path, name string, workflow Workflow) (legacyInput, error) {
	filename := filepath.Join(path, name)
	metadata, err := os.Lstat(filename)
	if err != nil {
		return legacyInput{}, &Error{Kind: "io", cause: err}
	}
	if metadata.Mode()&os.ModeSymlink != 0 || !metadata.Mode().IsRegular() {
		return legacyInput{}, &Error{Kind: "invalid_legacy"}
	}
	if metadata.Size() > 16*1024*1024 {
		return legacyInput{}, &Error{Kind: "legacy_size"}
	}
	body, err := os.ReadFile(filename)
	if err != nil {
		return legacyInput{}, &Error{Kind: "io", cause: err}
	}
	state, err := decodeLegacy(body)
	if err != nil {
		return legacyInput{}, err
	}
	if err := validateLegacy(name, state); err != nil {
		return legacyInput{}, err
	}
	hash := sha256.Sum256(body)
	return legacyInput{workflow, name, hex.EncodeToString(hash[:]), state}, nil
}

func validateLegacySnapshot(snapshot legacySnapshot) error {
	for _, key := range sortedKeys(snapshot.IssueArticleCounts) {
		if _, _, err := legacyPair(key); err != nil {
			return err
		}
		if snapshot.IssueArticleCounts[key] < 0 {
			return &Error{Kind: "invalid_legacy"}
		}
	}
	for _, key := range sortedKeys(snapshot.InpressArticleCounts) {
		if _, err := legacyNumeric(key); err != nil {
			return err
		}
		if snapshot.InpressArticleCounts[key] < 0 {
			return &Error{Kind: "invalid_legacy"}
		}
	}
	return nil
}

func validateLegacyRun(run legacyRun) error {
	if err := validateIdentifier(run.RunId, "Legacy delivery run id is invalid"); err != nil {
		return err
	}
	for index, keys := range [][]string{run.PendingIssueKeys, run.DoneIssueKeys, run.PendingInpressKeys, run.DoneInpressKeys} {
		if err := validateLegacyRunKeys(keys, index < 2); err != nil {
			return err
		}
	}
	for _, id := range run.DeliveredArticleIds {
		if id <= 0 {
			return &Error{Kind: "invalid_legacy"}
		}
	}
	for _, result := range run.UserResults {
		if _, err := legacyNumeric(result.SubscriberId); err != nil {
			return err
		}
	}
	return nil
}

func validateLegacyRunKeys(keys []string, isIssue bool) error {
	seen := map[string]bool{}
	for _, key := range keys {
		var err error
		if isIssue {
			_, _, err = legacyPair(key)
		} else {
			_, err = legacyNumeric(key)
		}
		if err != nil {
			return err
		}
		if seen[key] {
			return &Error{Kind: "invalid_legacy"}
		}
		seen[key] = true
	}
	return nil
}

func legacyImportFailureKind(err error) string {
	kind := "sqlite"
	var classified *Error
	if errors.As(err, &classified) {
		switch classified.Kind {
		case "io":
			kind = "io"
		case "json":
			kind = "invalid_json"
		case "input":
			kind = "invalid_input"
		case "stored":
			kind = "invalid_stored_state"
		case "not_found":
			kind = "not_found"
		case "conflict":
			kind = "conflict"
		case "audit":
			kind = "audit_persistence"
		default:
			kind = legacySourceFailureKind(classified.Kind)
		}
	}
	return kind
}

func legacySourceFailureKind(kind string) string {
	switch kind {
	case "invalid_legacy":
		return "invalid_legacy_state"
	case "legacy_conflict":
		return "legacy_import_conflict"
	case "legacy_size":
		return "legacy_state_too_large"
	default:
		return "sqlite"
	}
}

func readLegacyDirectory(path string) ([]os.DirEntry, error) {
	if _, err := os.Stat(path); err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, &Error{Kind: "io", cause: err}
	}
	metadata, err := os.Lstat(path)
	if err != nil {
		return nil, &Error{Kind: "io", cause: err}
	}
	if metadata.Mode()&os.ModeSymlink != 0 || !metadata.IsDir() {
		return nil, &Error{Kind: "invalid_legacy"}
	}
	entries, err := os.ReadDir(path)
	if err != nil {
		return nil, &Error{Kind: "io", cause: err}
	}
	return entries, nil
}
