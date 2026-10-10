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

// deliveryInvocation retains the admitted CLI options before storage access.
type deliveryInvocation struct {
	configuration                  config.Config
	options                        delivery.RunConfig
	keyOption, indexPath, database *string
	isHandoff                      bool
}

// runDelivery admits options and targets before owning delivery resources.
func runDelivery(ctx context.Context, workflow store.Workflow, values []string, output io.Writer) error {
	command := string(workflow)
	if hasHelp(values) {
		return writeResult(output, deliveryUsage(command))
	}
	args := arguments(slices.Clone(values))
	invocation, err := parseDeliveryInvocation(&args, workflow)
	if err != nil {
		return err
	}
	key, err := requireKey(invocation.keyOption)
	if err != nil {
		return err
	}
	if changes := invocation.options.ChangesFile; changes != nil {
		if *changes == "" {
			invocation.options.ChangesFile = nil
		} else {
			selected := projectPath(invocation.configuration.ProjectRoot, *changes)
			invocation.options.ChangesFile = &selected
		}
	}
	targets, err := deliveryTargets(invocation.configuration, invocation.indexPath, invocation.database, invocation.options.ChangesFile)
	if err != nil {
		return err
	}
	if invocation.isHandoff && len(targets) != 1 {
		return errors.New("internal delivery handoff requires exactly one database")
	}
	return executeDeliveryInvocation(ctx, invocation, targets, key, output)
}

// parseDeliveryInvocation preserves flag precedence and handoff admission before budgets.
func parseDeliveryInvocation(args *arguments, workflow store.Workflow) (deliveryInvocation, error) {
	invocation := deliveryInvocation{options: delivery.RunConfig{Mode: store.RunModeExecute, Workflow: workflow, Trigger: store.TriggerKindScheduled}}
	if args.flag("--dry-run") {
		invocation.options.Mode = store.RunModeDryRun
	}
	if args.flag("--no-dry-run") {
		invocation.options.Mode = store.RunModeExecute
	}
	invocation.isHandoff = args.flag("--internal-handoff-json")
	var err error
	invocation.configuration, err = args.storage()
	if err != nil {
		return invocation, err
	}
	if err := parseDeliveryIdentity(args, &invocation); err != nil {
		return invocation, err
	}
	if err := parseDeliveryBudgets(args, &invocation.options); err != nil {
		return invocation, err
	}
	if len(*args) != 0 {
		return invocation, fmt.Errorf("unexpected %s arguments: %s", workflow, strings.Join(*args, " "))
	}
	return invocation, nil
}

// parseDeliveryIdentity consumes ordered identity options and validates the handoff pair.
func parseDeliveryIdentity(args *arguments, invocation *deliveryInvocation) error {
	for _, option := range []struct {
		name  string
		value **string
	}{{"--secret-key-file", &invocation.keyOption}, {"--index-db", &invocation.indexPath}, {"--db", &invocation.database}, {"--changes-file", &invocation.options.ChangesFile}, {"--attempt-id", &invocation.options.AttemptId}} {
		value, err := args.take(option.name)
		if err != nil {
			return err
		}
		*option.value = value
	}
	attempt := invocation.options.AttemptId
	if invocation.isHandoff {
		if attempt == nil {
			return errors.New("--internal-handoff-json requires --attempt-id")
		}
		if !isDeliveryAttempt(*attempt) {
			return errors.New("--attempt-id must be exactly 32 hexadecimal characters")
		}
	} else if attempt != nil {
		return errors.New("--attempt-id requires --internal-handoff-json")
	}
	return nil
}

// parseDeliveryBudgets admits model, saturated candidate counts and execution limits.
func parseDeliveryBudgets(args *arguments, options *delivery.RunConfig) error {
	var err error
	options.AiModel, err = args.take("--ai-model")
	if err != nil {
		return err
	}
	candidateOption, err := args.optionalUnsigned("--max-candidates")
	if err != nil {
		return err
	}
	if candidateOption != nil {
		value := int(min(*candidateOption, math.MaxInt))
		options.MaxCandidates = &value
	}
	options.TimeoutSeconds, err = args.unsigned("--timeout", 60)
	if err != nil {
		return err
	}
	retries, err := args.unsigned("--retries", 3)
	if err != nil {
		return &diagnosticError{cause: err, diagnostic: "--retries must be an integer between 1 and 10"}
	}
	if retries < 1 || retries > 10 {
		return &diagnosticError{
			cause:      errors.New("--retries must be between 1 and 10"),
			diagnostic: "--retries must be between 1 and 10",
		}
	}
	options.RetryAttempts = int(retries)
	options.DedupeRetentionDays, err = args.signed("--dedupe-retention-days", 60)
	return err
}

// executeDeliveryInvocation keeps the verified codec alive through preflight, execution and output.
func executeDeliveryInvocation(ctx context.Context, invocation deliveryInvocation, targets []deliveryTarget, key string, output io.Writer) error {
	configuration := invocation.configuration
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
	options := invocation.options
	options.AuthDbPath, options.SecretCodec = configuration.AuthDbPath, codec
	outcomes, status, err := runDeliveryTargets(ctx, options, targets)
	if err != nil {
		return err
	}
	payload := map[string]any{"workflow": options.Workflow, "mode": options.Mode, "status": status, "databases": outcomes}
	if invocation.isHandoff {
		payload = map[string]any{"protocol_version": 1, "attempt_id": *options.AttemptId, "workflow": options.Workflow, "mode": options.Mode, "status": status, "db_name": outcomes[0].DbName}
	}
	if err := writeResult(output, payload); err != nil {
		return err
	}
	if !slices.Contains([]string{"idle", "completed", "skipped"}, status) {
		return fmt.Errorf("%s delivery finished with %s status", options.Workflow, status)
	}
	return nil
}

// runDeliveryTargets executes admitted databases sequentially and retains status precedence.
func runDeliveryTargets(ctx context.Context, options delivery.RunConfig, targets []deliveryTarget) ([]delivery.RunOutcome, string, error) {
	outcomes := []delivery.RunOutcome{}
	status := "idle"
	priority := []string{"idle", "skipped", "completed", "running", "cancelled", "timed_out", "failed", "unknown"}
	for _, target := range targets {
		options.IndexDbPath, options.DbName = target.path, target.name
		result, err := delivery.RunRecommendationDelivery(ctx, options)
		if err != nil {
			return nil, "", err
		}
		outcomes = append(outcomes, result)
		if slices.Index(priority, result.Status) > slices.Index(priority, status) {
			status = result.Status
		}
	}
	return outcomes, status, nil
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
		return explicitDeliveryTarget(configuration, *indexPath, database), nil
	}
	if database == nil && changes != nil {
		value, err := deliveryManifestDatabase(*changes)
		if err != nil {
			return nil, err
		}
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
	return enumerateDeliveryTargets(configuration.IndexDir)
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

// explicitDeliveryTarget resolves an explicit path without filesystem admission.
func explicitDeliveryTarget(configuration config.Config, indexPath string, database *string) []deliveryTarget {
	filename := projectPath(configuration.ProjectRoot, indexPath)
	name := filepath.Base(filename)
	if name == "." || name == ".." || name == string(filepath.Separator) {
		name = "index.sqlite"
	}
	if database != nil {
		name = normalizeDatabase(*database)
	}
	return []deliveryTarget{{filename, name}}
}

// deliveryManifestDatabase reads the strict manifest while retaining duplicate-key decoding.
func deliveryManifestDatabase(filename string) (string, error) {
	data, err := os.ReadFile(filename)
	if err != nil {
		return "", err
	}
	if !jsonvalue.ValidJson(string(data)) {
		return "", errors.New("invalid change manifest JSON")
	}
	var payload map[string]json.RawMessage
	if err := json.Unmarshal(data, &payload); err != nil {
		return "", err
	}
	var value string
	if json.Unmarshal(payload["db_name"], &value) != nil || strings.TrimSpace(value) == "" {
		return "", errors.New("Change manifest missing db_name; specify --db explicitly")
	}
	value = strings.TrimSpace(value)
	return value, nil
}

// enumerateDeliveryTargets retains sorted SQLite names, including directory entries.
func enumerateDeliveryTargets(indexDirectory string) ([]deliveryTarget, error) {
	if err := os.MkdirAll(indexDirectory, 0777); err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(indexDirectory)
	if err != nil {
		return nil, err
	}
	targets := []deliveryTarget{}
	for _, entry := range entries {
		if entry.Name() != ".sqlite" && filepath.Ext(entry.Name()) == ".sqlite" && utf8.ValidString(entry.Name()) {
			targets = append(targets, deliveryTarget{filepath.Join(indexDirectory, entry.Name()), entry.Name()})
		}
	}
	if len(targets) == 0 {
		return nil, config.ErrNoDatabases
	}
	return targets, nil
}
