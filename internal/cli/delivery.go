package cli

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/delivery"
	"github.com/QianFuv/LitRadar/internal/runtime"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	indexmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/index"
)

func deliveryUsage(command string) any {
	return map[string]any{"usage": "litradar " + command + " --secret-key-file PATH [--project-root PATH] [--auth-db PATH] [--db NAME] [--changes-file PATH] [--ai-model MODEL] [--max-candidates N] [--timeout N] [--retries N] [--dedupe-retention-days N] [--dry-run|--no-dry-run]"}
}

func (args *arguments) optionalUnsigned(name string) (*uint64, error) {
	value, err := args.take(name)
	if err != nil || value == nil {
		return nil, err
	}
	number, err := strconv.ParseUint(strings.TrimPrefix(*value, "+"), 10, 64)
	return &number, err
}

func (args *arguments) unsigned(name string, fallback uint64) (uint64, error) {
	value, err := args.optionalUnsigned(name)
	if err != nil || value == nil {
		return fallback, err
	}
	return *value, nil
}

func (args *arguments) signed(name string, fallback int64) (int64, error) {
	value, err := args.take(name)
	if err != nil || value == nil {
		return fallback, err
	}
	return strconv.ParseInt(*value, 10, 64)
}

func runDelivery(ctx context.Context, workflow store.Workflow, values []string, output io.Writer) error {
	command := string(workflow)
	if hasHelp(values) {
		return writeResult(output, deliveryUsage(command))
	}
	args := arguments(slices.Clone(values))
	mode := store.RunModeExecute
	if args.flag("--dry-run") {
		mode = store.RunModeDryRun
	}
	if args.flag("--no-dry-run") {
		mode = store.RunModeExecute
	}
	isHandoff := args.flag("--internal-handoff-json")
	configuration, err := args.storage()
	if err != nil {
		return err
	}
	var keyOption, indexPath, database, changes, attempt, model *string
	for _, option := range []struct {
		name  string
		value **string
	}{{"--secret-key-file", &keyOption}, {"--index-db", &indexPath}, {"--db", &database}, {"--changes-file", &changes}, {"--attempt-id", &attempt}} {
		*option.value, err = args.take(option.name)
		if err != nil {
			return err
		}
	}
	if isHandoff {
		if attempt == nil {
			return errors.New("--internal-handoff-json requires --attempt-id")
		}
		if !isDeliveryAttempt(*attempt) {
			return errors.New("--attempt-id must be exactly 32 hexadecimal characters")
		}
	} else if attempt != nil {
		return errors.New("--attempt-id requires --internal-handoff-json")
	}
	model, err = args.take("--ai-model")
	if err != nil {
		return err
	}
	candidateOption, err := args.optionalUnsigned("--max-candidates")
	if err != nil {
		return err
	}
	var candidates *int
	if candidateOption != nil {
		value := int(min(*candidateOption, math.MaxInt))
		candidates = &value
	}
	timeout, err := args.unsigned("--timeout", 60)
	if err != nil {
		return err
	}
	retries, err := args.unsigned("--retries", 3)
	if err != nil {
		return err
	}
	if retries < 1 || retries > 10 {
		return errors.New("--retries must be between 1 and 10")
	}
	retention, err := args.signed("--dedupe-retention-days", 60)
	if err != nil {
		return err
	}
	if len(args) != 0 {
		return fmt.Errorf("unexpected %s arguments: %s", command, strings.Join(args, " "))
	}
	key, err := requireKey(keyOption)
	if err != nil {
		return err
	}
	if changes != nil {
		if *changes == "" {
			changes = nil
		} else {
			selected := projectPath(configuration.ProjectRoot, *changes)
			changes = &selected
		}
	}
	targets, err := deliveryTargets(configuration, indexPath, database, changes)
	if err != nil {
		return err
	}
	if isHandoff && len(targets) != 1 {
		return errors.New("internal delivery handoff requires exactly one database")
	}
	if err := runtime.PreflightStorage(ctx, configuration); err != nil {
		return err
	}
	codec, err := loadVerifiedKey(ctx, configuration.AuthDbPath, key)
	if err != nil {
		return err
	}
	defer codec.Close()
	for _, target := range targets {
		if _, err := indexmigration.Preflight(ctx, target.path); err != nil {
			return err
		}
	}
	outcomes := []delivery.RunOutcome{}
	status := "idle"
	priority := []string{"idle", "skipped", "completed", "running", "cancelled", "timed_out", "failed", "unknown"}
	for _, target := range targets {
		result, err := delivery.RunRecommendationDelivery(ctx, delivery.RunConfig{AuthDbPath: configuration.AuthDbPath, SecretCodec: codec, IndexDbPath: target.path, DbName: target.name, ChangesFile: changes, AttemptId: attempt, AiModel: model, MaxCandidates: candidates, TimeoutSeconds: timeout, RetryAttempts: int(retries), DedupeRetentionDays: retention, Mode: mode, Workflow: workflow, Trigger: store.TriggerKindScheduled})
		if err != nil {
			return err
		}
		outcomes = append(outcomes, result)
		if slices.Index(priority, result.Status) > slices.Index(priority, status) {
			status = result.Status
		}
	}
	payload := map[string]any{"workflow": workflow, "mode": mode, "status": status, "databases": outcomes}
	if isHandoff {
		payload = map[string]any{"protocol_version": 1, "attempt_id": *attempt, "workflow": workflow, "mode": mode, "status": status, "db_name": outcomes[0].DbName}
	}
	if err := writeResult(output, payload); err != nil {
		return err
	}
	if !slices.Contains([]string{"idle", "completed", "skipped"}, status) {
		return fmt.Errorf("%s delivery finished with %s status", command, status)
	}
	return nil
}

func isDeliveryAttempt(value string) bool {
	if len(value) != 32 {
		return false
	}
	for _, character := range []byte(value) {
		if !(character >= '0' && character <= '9' || character >= 'a' && character <= 'f' || character >= 'A' && character <= 'F') {
			return false
		}
	}
	return true
}

type deliveryTarget struct{ path, name string }

func deliveryTargets(configuration config.Config, indexPath, database, changes *string) ([]deliveryTarget, error) {
	if indexPath != nil {
		filename := projectPath(configuration.ProjectRoot, *indexPath)
		name := filepath.Base(filename)
		if name == "." || name == ".." || name == string(filepath.Separator) {
			name = "index.sqlite"
		}
		if database != nil {
			name = normalizeDatabase(*database)
		}
		return []deliveryTarget{{filename, name}}, nil
	}
	if database == nil && changes != nil {
		data, err := os.ReadFile(*changes)
		if err != nil {
			return nil, err
		}
		if !jsonvalue.ValidJson(string(data)) {
			return nil, errors.New("invalid change manifest JSON")
		}
		var payload map[string]json.RawMessage
		if err := json.Unmarshal(data, &payload); err != nil {
			return nil, err
		}
		var value string
		if json.Unmarshal(payload["db_name"], &value) != nil || strings.TrimSpace(value) == "" {
			return nil, errors.New("Change manifest missing db_name; specify --db explicitly")
		}
		value = strings.TrimSpace(value)
		database = &value
	}
	if database != nil {
		name := normalizeDatabase(*database)
		filename := filepath.Join(configuration.IndexDir, name)
		if _, err := os.Stat(filename); err != nil {
			return nil, config.ErrNotFound
		}
		return []deliveryTarget{{filename, name}}, nil
	}
	if err := os.MkdirAll(configuration.IndexDir, 0777); err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(configuration.IndexDir)
	if err != nil {
		return nil, err
	}
	targets := []deliveryTarget{}
	for _, entry := range entries {
		if entry.Name() != ".sqlite" && filepath.Ext(entry.Name()) == ".sqlite" && utf8.ValidString(entry.Name()) {
			targets = append(targets, deliveryTarget{filepath.Join(configuration.IndexDir, entry.Name()), entry.Name()})
		}
	}
	if len(targets) == 0 {
		return nil, config.ErrNoDatabases
	}
	return targets, nil
}

func normalizeDatabase(value string) string {
	path := strings.TrimRightFunc(value, func(character rune) bool { return character < 128 && os.IsPathSeparator(uint8(character)) })
	for path != "" && filepath.Base(path) == "." && path != "." {
		path = strings.TrimRightFunc(path[:len(path)-1], func(character rune) bool { return character < 128 && os.IsPathSeparator(uint8(character)) })
	}
	name := strings.TrimSpace(filepath.Base(path))
	if value == "" || name == "." || name == ".." || name == string(filepath.Separator) {
		name = strings.TrimSpace(value)
	}
	if !strings.HasSuffix(name, ".sqlite") {
		name += ".sqlite"
	}
	return name
}
