package index

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os/exec"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/process"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/index"
)

// NotifyObservation distinguishes trusted terminal results from ambiguous child output.
type NotifyObservation struct {
	Status   storage.NotifyStatus
	ExitCode *int32
}
type notifyPayload struct {
	ProtocolVersion uint32 `json:"protocol_version"`
	AttemptID       string `json:"attempt_id"`
	Workflow        string `json:"workflow"`
	Mode            string `json:"mode"`
	Status          string `json:"status"`
	DBName          string `json:"db_name"`
}

// ClassifyNotifyOutput requires exact handoff identity, bounded complete JSON and exit/result consistency.
func ClassifyNotifyOutput(body []byte, hasExceededLimit bool, attempt, database string, isDryRun bool, exit *int32) NotifyObservation {
	unknown := NotifyObservation{Status: storage.NotifyUnknown, ExitCode: exit}
	if hasExceededLimit {
		return unknown
	}
	var payload notifyPayload
	if decodeWorkerStruct(body, &payload, 0) != nil {
		return unknown
	}
	expectedMode := "execute"
	if isDryRun {
		expectedMode = "dry_run"
	}
	if mismatchedNotifyIdentity(payload, attempt, database, expectedMode) {
		return unknown
	}
	status := storage.NotifyStatus(payload.Status)
	switch status {
	case storage.NotifyRunning, storage.NotifyIdle, storage.NotifyCompleted, storage.NotifySkipped, storage.NotifyFailed, storage.NotifyCancelled, storage.NotifyTimedOut, storage.NotifyUnknown:
	default:
		return unknown
	}
	if exit == nil || (*exit == 0) != status.IsSuccess() {
		return unknown
	}
	return NotifyObservation{status, exit}
}

func readNotifyOutput(reader io.Reader) ([]byte, bool, error) {
	body := make([]byte, 0, 65536)
	hasExceeded := false
	var buffer [8192]byte
	for {
		count, err := reader.Read(buffer[:])
		retained := min(65536-len(body), count)
		body = append(body, buffer[:retained]...)
		hasExceeded = hasExceeded || retained < count
		if errors.Is(err, io.EOF) {
			return body, hasExceeded, nil
		}
		if err != nil {
			return nil, false, err
		}
	}
}

// NotifyProcessConfig contains local invocation paths; no secret contents are serialized.
type NotifyProcessConfig struct {
	Executable, SecretKeyFile, ProjectRoot string
	AuthDBPath                             string
	IsDryRun                               bool
}

// RunNotifyProcess forwards the selected auth store, bounds retained output and closes native tree ownership.
func RunNotifyProcess(ctx context.Context, config NotifyProcessConfig, database, manifest, attempt string) (resultValue1 NotifyObservation, resultError error) {
	arguments := []string{"notify", "--secret-key-file", config.SecretKeyFile, "--db", database, "--changes-file", manifest, "--project-root", config.ProjectRoot, "--attempt-id", attempt, "--internal-handoff-json"}
	if config.AuthDBPath != "" {
		arguments = append(arguments, "--auth-db", config.AuthDBPath)
	}
	if config.IsDryRun {
		arguments = append(arguments, "--dry-run")
	}
	child, err := process.Start(ctx, process.Config{Path: config.Executable, Args: arguments, StreamStdout: true, InheritStderr: true})
	if err != nil {
		return NotifyObservation{}, err
	}
	defer func() {
		if closeError := child.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close child: %w", closeError))
		}
	}()
	if err := child.Stdin.Close(); err != nil {
		slog.Warn("cleanup.failed", "operation", "child.Stdin.Close", "error", err)
	}
	body, exceeded, readError := readNotifyOutput(child.Stdout)
	waitError := child.Wait(ctx)
	var exit *int32
	if waitError == nil {
		code := int32(0)
		exit = &code
	} else {
		if processExit, ok := errors.AsType[*exec.ExitError](waitError); ok {
			if code := processExit.ExitCode(); code >= 0 {
				converted := int32(code)
				exit = &converted
			}
		} else {
			return NotifyObservation{Status: storage.NotifyUnknown}, nil
		}
	}
	if readError != nil {
		return NotifyObservation{Status: storage.NotifyUnknown, ExitCode: exit}, nil
	}
	return ClassifyNotifyOutput(body, exceeded, attempt, database, config.IsDryRun, exit), nil
}

// mismatchedNotifyIdentity rejects any mismatch in the exact trusted notification handoff identity.
func mismatchedNotifyIdentity(payload notifyPayload, attempt, database, expectedMode string) bool {
	return payload.ProtocolVersion != 1 || payload.AttemptID != attempt || payload.Workflow != "notify" || payload.Mode != expectedMode || payload.DBName != database
}
