package cli

import (
	"bufio"
	"context"
	"errors"
	"io"
	"path/filepath"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/internal/auth"
	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/backup"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/maintenance"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

var adminUsage = map[string]any{"usage": []string{
	"litradar admin bootstrap --username NAME --password-stdin [--project-root PATH] [--auth-db PATH]",
	"litradar admin secrets migrate --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
	"litradar admin secrets verify --secret-key-file PATH [--project-root PATH] [--auth-db PATH]",
	"litradar admin secrets rotate --old-key-file PATH --new-key-file PATH [--project-root PATH] [--auth-db PATH]",
	"litradar admin backup create --output PATH [--include-indexes] [--include-push-state] [--project-root PATH] [--auth-db PATH]",
	"litradar admin backup verify --backup PATH [--project-root PATH]",
	"litradar admin backup restore --backup PATH --confirm-restore [--project-root PATH] [--auth-db PATH]",
	"litradar admin index optimize-storage --confirm-index-maintenance [--project-root PATH]",
}}

func writeResult(output io.Writer, result any) error {
	encoded, err := jsonvalue.EncodeJson(result)
	if err != nil {
		return err
	}
	_, err = io.WriteString(output, encoded+"\n")
	return err
}

func usageError(value any) error {
	encoded, err := jsonvalue.EncodeJson(value)
	if err != nil {
		return err
	}
	return errors.New(encoded)
}

func runAdmin(ctx context.Context, values []string, input io.Reader, output io.Writer) error {
	if hasHelp(values) {
		return writeResult(output, adminUsage)
	}
	result, err := adminResult(ctx, arguments(slices.Clone(values)), input)
	if err != nil {
		var failure maintenance.Failure
		if errors.As(err, &failure) {
			if outputError := writeResult(output, map[string]any{"status": "failed", "error": map[string]any{"code": failure.Code, "message": failure.Error(), "recovery_paths": failure.Recovery}}); outputError != nil {
				return outputError
			}
		}
		return err
	}
	return writeResult(output, result)
}

func adminResult(ctx context.Context, args arguments, input io.Reader) (any, error) {
	root, err := args.projectRoot()
	if err != nil {
		return nil, err
	}
	hasExplicitAuth := slices.Contains(args, "--auth-db")
	filename, err := args.authDatabase(root)
	if err != nil {
		return nil, err
	}
	command, err := adminCommand(&args)
	if err != nil {
		return nil, err
	}
	configuration := config.FromProjectRoot(root)
	switch command {
	case "bootstrap":
		username, err := args.take("--username")
		if err != nil {
			return nil, err
		}
		shouldRead := args.flag("--password-stdin")
		if username == nil || !shouldRead || len(args) != 0 {
			return nil, usageError(adminUsage)
		}
		if _, err := authmigration.Migrate(ctx, filename); err != nil {
			return nil, err
		}
		password, err := bufio.NewReader(input).ReadString('\n')
		if err != nil && err != io.EOF {
			return nil, err
		}
		if password == "" {
			return nil, errors.New("password stdin was empty")
		}
		password = strings.TrimRight(password, "\r\n")
		repository, err := authstorage.Open(filename)
		if err != nil {
			return nil, err
		}
		defer repository.Close()
		user, err := auth.New(repository, 2).Bootstrap(ctx, strings.TrimSpace(*username), password, nil)
		if err != nil {
			return nil, err
		}
		return map[string]any{"status": "created", "user": map[string]any{"id": int64(user.Id), "username": user.Username, "is_admin": user.IsAdmin}}, nil
	case "secrets migrate", "secrets verify", "secrets rotate":
		return adminSecrets(ctx, command, args, filename)
	case "backup create":
		output, err := args.take("--output")
		if err != nil {
			return nil, err
		}
		includeIndexes, includePush := args.flag("--include-indexes"), args.flag("--include-push-state")
		if output == nil || len(args) != 0 {
			return nil, usageError(adminUsage)
		}
		directory := projectPath(root, *output)
		manifest, err := backup.Create(ctx, backup.CreateOptions{Config: configuration, AuthDbPath: filename, OutputDir: directory, IncludeIndexDatabases: includeIndexes, IncludePushState: includePush})
		return map[string]any{"status": "created", "backup": directory, "manifest": manifest}, err
	case "backup verify", "backup restore":
		input, err := args.take("--backup")
		if err != nil {
			return nil, err
		}
		isRestore := command == "backup restore"
		isConfirmed := !isRestore || args.flag("--confirm-restore")
		if input == nil || len(args) != 0 || !isConfirmed {
			return nil, usageError(adminUsage)
		}
		directory := projectPath(root, *input)
		if isRestore {
			report, err := backup.Restore(ctx, backup.RestoreOptions{Config: configuration, AuthDbPath: filename, BackupDir: directory})
			return map[string]any{"status": "restored", "backup": directory, "report": report}, err
		}
		manifest, err := backup.Verify(ctx, directory)
		return map[string]any{"status": "verified", "backup": directory, "manifest": manifest}, err
	case "index optimize-storage":
		if hasExplicitAuth {
			return nil, usageError(adminUsage)
		}
		isConfirmed := args.flag("--confirm-index-maintenance")
		if len(args) != 0 {
			return nil, usageError(adminUsage)
		}
		report, err := maintenance.Optimize(ctx, maintenance.Options{Config: configuration, Confirmed: isConfirmed})
		return map[string]any{"status": report.Outcome, "report": report}, err
	default:
		return nil, usageError(adminUsage)
	}
}

func adminCommand(args *arguments) (string, error) {
	command := []string{}
	for index := 0; index < len(*args); {
		argument := (*args)[index]
		if slices.Contains([]string{"--username", "--secret-key-file", "--old-key-file", "--new-key-file", "--output", "--backup"}, argument) {
			if index+1 >= len(*args) {
				return "", errors.New(argument + " requires a value")
			}
			index += 2
		} else if strings.HasPrefix(argument, "-") {
			index++
		} else {
			command = append(command, argument)
			*args = slices.Delete(*args, index, index+1)
		}
	}
	for _, name := range []string{"bootstrap", "secrets migrate", "secrets verify", "secrets rotate", "backup create", "backup verify", "backup restore", "index optimize-storage"} {
		if slices.Equal(command, strings.Split(name, " ")) {
			return name, nil
		}
	}
	return "", usageError(adminUsage)
}

func adminSecrets(ctx context.Context, command string, args arguments, filename string) (any, error) {
	option := "--secret-key-file"
	if command == "secrets rotate" {
		option = "--old-key-file"
	}
	key, err := args.take(option)
	if err != nil {
		return nil, err
	}
	if key == nil {
		return nil, usageError(adminUsage)
	}
	var nextKey *string
	if command == "secrets rotate" {
		nextKey, err = args.take("--new-key-file")
		if err != nil {
			return nil, err
		}
		if nextKey == nil {
			return nil, usageError(adminUsage)
		}
	}
	if len(args) != 0 {
		return nil, usageError(adminUsage)
	}
	if _, err := authmigration.Migrate(ctx, filename); err != nil {
		return nil, err
	}
	codec, err := secrets.Load(*key)
	if err != nil {
		return nil, err
	}
	defer codec.Close()
	switch command {
	case "secrets migrate":
		report, err := secrets.Migrate(ctx, filename, codec)
		return map[string]any{"status": "migrated", "migrated": report.Migrated, "verified": report.Verified, "empty": report.Empty}, err
	case "secrets verify":
		report, err := secrets.Verify(ctx, filename, codec)
		return map[string]any{"status": "verified", "verified": report.Verified, "empty": report.Empty}, err
	default:
		nextCodec, err := secrets.Load(*nextKey)
		if err != nil {
			return nil, err
		}
		defer nextCodec.Close()
		rotated, err := secrets.Rotate(ctx, filename, codec, nextCodec)
		return map[string]any{"status": "rotated", "rotated": rotated}, err
	}
}

func projectPath(root, selected string) string {
	if filepath.IsAbs(selected) {
		return selected
	}
	return filepath.Join(root, selected)
}
