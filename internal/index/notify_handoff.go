package index

import (
	"context"
	"errors"
	"io"
	"os/exec"

	"github.com/QianFuv/LitRadar/internal/platform/process"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// NotifyObservation distinguishes trusted terminal results from ambiguous child output.
type NotifyObservation struct {
	Status   storage.NotifyStatus
	ExitCode *int32
}
type notifyPayload struct {
	ProtocolVersion uint32 `json:"protocol_version"`
	AttemptId       string `json:"attempt_id"`
	Workflow        string `json:"workflow"`
	Mode            string `json:"mode"`
	Status          string `json:"status"`
	DbName          string `json:"db_name"`
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
	IsDryRun                               bool
}

// RunNotifyProcess drains all output while retaining at most 64 KiB and always closes native tree ownership.
func RunNotifyProcess(ctx context.Context, config NotifyProcessConfig, database, manifest, attempt string) (NotifyObservation, error) {
	arguments := []string{"notify", "--secret-key-file", config.SecretKeyFile, "--db", database, "--changes-file", manifest, "--project-root", config.ProjectRoot, "--attempt-id", attempt, "--internal-handoff-json"}
	if config.IsDryRun {
		arguments = append(arguments, "--dry-run")
	}
	child, err := process.Start(ctx, process.Config{Path: config.Executable, Args: arguments, StreamStdout: true, InheritStderr: true})
	if err != nil {
		return NotifyObservation{}, err
	}
	defer child.Close()
	child.Stdin.Close()
	body, exceeded, readError := readNotifyOutput(child.Stdout)
	waitError := child.Wait(ctx)
	var exit *int32
	if waitError == nil {
		code := int32(0)
		exit = &code
	} else {
		var processExit *exec.ExitError
		if errors.As(waitError, &processExit) {
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
	return payload.ProtocolVersion != 1 || payload.AttemptId != attempt || payload.Workflow != "notify" || payload.Mode != expectedMode || payload.DbName != database
}
