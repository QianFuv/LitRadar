package cli

import (
	"context"
	"errors"
	"io"
	"slices"
	"strconv"

	"github.com/QianFuv/LitRadar/internal/delivery"
	"github.com/QianFuv/LitRadar/internal/runtime"
	"github.com/QianFuv/LitRadar/internal/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	schedulerstorage "github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

var schedulerUsage = map[string]any{"usage": []string{
	"litradar scheduler validate --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
	"litradar scheduler run-once TASK_ID --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
	"litradar scheduler dry-run-once TASK_ID --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
}}

func (args *arguments) storage() (config.Config, error) {
	root, err := args.projectRoot()
	if err != nil {
		return config.Config{}, err
	}
	filename, err := args.authDatabase(root)
	return config.FromProjectRoot(root).WithAuthDbPath(filename), err
}

func requireKey(key *string) (string, error) {
	if key == nil {
		return "", errors.New("--secret-key-file is required")
	}
	return *key, nil
}

func loadVerifiedKey(ctx context.Context, filename, key string) (*secrets.Codec, error) {
	codec, err := secrets.Load(key)
	if err != nil {
		return nil, err
	}
	if _, err := secrets.Verify(ctx, filename, codec); err != nil {
		codec.Close()
		return nil, err
	}
	return codec, nil
}

func runScheduler(ctx context.Context, values []string, executable string, output io.Writer) error {
	if hasHelp(values) {
		return writeResult(output, schedulerUsage)
	}
	args := arguments(slices.Clone(values))
	configuration, err := args.storage()
	if err != nil {
		return err
	}
	keyOption, err := args.take("--secret-key-file")
	if err != nil {
		return err
	}
	var id int64
	mode := scheduler.Execute
	isValidate := len(args) == 1 && args[0] == "validate"
	if !isValidate {
		if len(args) != 2 || (args[0] != "run-once" && args[0] != "dry-run-once") {
			return usageError(schedulerUsage)
		}
		id, err = strconv.ParseInt(args[1], 10, 64)
		if err != nil {
			return err
		}
		if args[0] == "dry-run-once" {
			mode = scheduler.DryRun
		}
	}
	key, err := requireKey(keyOption)
	if err != nil {
		return err
	}
	if err := runtime.PreflightStorage(ctx, configuration); err != nil {
		return err
	}
	codec, err := loadVerifiedKey(ctx, configuration.AuthDbPath, key)
	if err != nil {
		return err
	}
	defer codec.Close()
	repository, err := schedulerstorage.Open(configuration.AuthDbPath)
	if err != nil {
		return err
	}
	defer repository.Close()
	if isValidate {
		result, err := scheduler.LoadJobs(ctx, repository)
		if err != nil {
			return err
		}
		return writeResult(output, result)
	}
	result, err := scheduler.RunTaskNow(ctx, repository, scheduler.ProcessConfig{ProjectRoot: configuration.ProjectRoot, AuthDatabase: configuration.AuthDbPath, Executable: executable, SecretKeyFile: key}, id, mode)
	if err != nil {
		return err
	}
	return writeResult(output, result)
}

func runManualDelivery(ctx context.Context, values []string, output io.Writer) error {
	args := arguments(slices.Clone(values))
	configuration, err := args.storage()
	if err != nil {
		return err
	}
	keyOption, err := args.take("--secret-key-file")
	if err != nil {
		return err
	}
	key, err := requireKey(keyOption)
	if err != nil {
		return err
	}
	runOption, err := args.take("--run-id")
	if err != nil {
		return err
	}
	if runOption == nil {
		return errors.New("--run-id is required")
	}
	runId, err := strconv.ParseInt(*runOption, 10, 64)
	if err != nil {
		return err
	}
	owner, err := args.take("--owner-id")
	if err != nil {
		return err
	}
	if owner == nil {
		return errors.New("--owner-id is required")
	}
	if runId <= 0 {
		return errors.New("--run-id must be a positive integer")
	}
	if len(args) != 0 {
		return errors.New("unexpected delivery-run arguments")
	}
	if _, err := authmigration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		return err
	}
	codec, err := loadVerifiedKey(ctx, configuration.AuthDbPath, key)
	if err != nil {
		return err
	}
	defer codec.Close()
	terminal, err := delivery.RunManualDeliveryJob(ctx, configuration, codec, runId, *owner)
	if err != nil {
		return err
	}
	if !terminal.Status.IsTerminal() {
		return errors.New("durable manual delivery job did not reach a terminal state")
	}
	return writeResult(output, map[string]any{"run_id": terminal.Id, "status": terminal.Status})
}
