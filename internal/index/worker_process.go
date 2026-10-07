package index

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/process"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// WorkerProcessConfig supplies task-owned request storage and the in-memory bootstrap producer.
type WorkerProcessConfig struct {
	Executable, RequestDirectory string
	Bootstrap                    func(WorkerRequest) WorkerBootstrap
}
type workerLauncher func(context.Context, string, uint64) (*process.Child, error)
type workerReaderEvent struct {
	worker  uint64
	message WorkerMessage
	err     error
}
type ownedWorker struct {
	child *process.Child
	path  string
}

// RunWorkerProcesses owns children, request files, reader goroutines and the parent-only commit loop.
func RunWorkerProcesses(ctx context.Context, writer *ParentWriter, requests []WorkerRequest, config WorkerProcessConfig) (RunMetrics, error) {
	launcher := func(ctx context.Context, path string, worker uint64) (*process.Child, error) {
		return process.Start(ctx, process.Config{Path: config.Executable, Args: []string{"index", "--live-worker-request", path}, Environment: workerEnvironment(), StreamStdout: true, InheritStderr: true})
	}
	return runWorkerProcesses(ctx, writer, requests, config, 30*time.Second, launcher)
}
func workerEnvironment() []string {
	environment := []string{}
	for _, value := range os.Environ() {
		key, _, _ := strings.Cut(value, "=")
		if !strings.EqualFold(key, "LITRADAR_CNKI_CAPTCHA_TOKEN") && !strings.EqualFold(key, process.ParentEnvironment) {
			environment = append(environment, value)
		}
	}
	return append(environment, process.ParentEnvironment+"="+strconv.Itoa(os.Getpid()))
}

// runWorkerProcesses validates before file operations and owns cancellation, children and reader joins.
func runWorkerProcesses(ctx context.Context, writer *ParentWriter, requests []WorkerRequest, config WorkerProcessConfig, heartbeatInterval time.Duration, launcher workerLauncher) (RunMetrics, error) {
	if _, err := NewParentWriter(writer.content, writer.control, writer.context, writer.shouldRecordEvents, requests, writer.Metrics); err != nil {
		return writer.Metrics, err
	}
	if err := prepareWorkerRequestDirectory(config.RequestDirectory); err != nil {
		return writer.Metrics, err
	}
	if len(requests) == 0 {
		return writer.Metrics, nil
	}
	lifetime, cancel := context.WithCancel(ctx)
	events := make(chan workerReaderEvent, len(requests))
	children := make([]ownedWorker, 0, len(requests))
	var readers sync.WaitGroup
	defer func() {
		cancel()
		for _, worker := range children {
			worker.child.Close()
			removeWorkerFile(worker.path)
		}
		readers.Wait()
	}()
	for _, request := range requests {
		worker, err := launchOwnedWorker(lifetime, request, config, launcher)
		if err != nil {
			return writer.Metrics, err
		}
		children = append(children, worker)
		if err := WriteProtocol(worker.child.Stdin, config.Bootstrap(request)); err != nil {
			return writer.Metrics, parentProtocolFailure(request.WorkerId)
		}
		readers.Add(1)
		go readWorkerFrames(lifetime, request.WorkerId, worker.child, events, &readers)
	}
	err := superviseWorkerFrames(ctx, writer, children, events, heartbeatInterval)
	return writer.Metrics, err
}

// prepareWorkerRequestDirectory cleans recognizable stale requests even when no executor is needed.
func prepareWorkerRequestDirectory(directory string) error {
	if err := os.MkdirAll(directory, 0777); err != nil {
		return err
	}
	_, err := CleanupLegacyWorkerRequests(directory, time.Now())
	return err
}

// launchOwnedWorker removes its request on failure and returns ownership before bootstrap writing.
func launchOwnedWorker(ctx context.Context, request WorkerRequest, config WorkerProcessConfig, launcher workerLauncher) (ownedWorker, error) {
	path := filepath.Join(config.RequestDirectory, request.RunId+"-worker-"+strconv.FormatUint(request.WorkerId, 10)+".json")
	body, err := json.Marshal(request)
	if err != nil {
		return ownedWorker{}, err
	}
	if err := os.WriteFile(path, body, 0666); err != nil {
		removeWorkerFile(path)
		return ownedWorker{}, err
	}
	child, err := launcher(ctx, path, request.WorkerId)
	if err != nil {
		removeWorkerFile(path)
		return ownedWorker{}, err
	}
	return ownedWorker{child, path}, nil
}

// readWorkerFrames makes every frame delivery cancellable so teardown can join blocked readers.
func readWorkerFrames(ctx context.Context, worker uint64, child *process.Child, events chan<- workerReaderEvent, readers *sync.WaitGroup) {
	defer readers.Done()
	reader := NewProtocolReader(child.Stdout)
	for {
		var message WorkerMessage
		err := reader.Read(&message)
		select {
		case events <- workerReaderEvent{worker, message, err}:
		case <-ctx.Done():
			return
		}
		if err != nil {
			return
		}
	}
}

// superviseWorkerFrames serializes page commits and requires terminal, EOF and successful exit.
func superviseWorkerFrames(ctx context.Context, writer *ParentWriter, children []ownedWorker, events <-chan workerReaderEvent, heartbeatInterval time.Duration) error {
	remaining := len(children)
	ticker := time.NewTicker(heartbeatInterval)
	defer ticker.Stop()
	for remaining > 0 {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			if err := storage.HeartbeatLease(ctx, writer.control, writer.context.CatalogName, writer.context.ProviderName, writer.context.RunId, time.Now().Unix()); err != nil {
				return err
			}
		case event := <-events:
			if event.err != nil {
				if err := finishWorkerProcess(ctx, writer, children[event.worker], event); err != nil {
					return err
				}
				remaining--
			} else if err := writer.Handle(ctx, event.worker, event.message, children[event.worker].child.Stdin); err != nil {
				return err
			}
		}
	}
	return nil
}

// finishWorkerProcess closes stdin before waiting and removes the request only after tree cleanup.
func finishWorkerProcess(ctx context.Context, writer *ParentWriter, worker ownedWorker, event workerReaderEvent) error {
	var framing *ProtocolError
	if !errors.As(event.err, &framing) || framing.Kind != "end" {
		return parentProtocolFailure(event.worker)
	}
	if !writer.progress[event.worker].hasTerminal {
		return parentProtocolFailure(event.worker)
	}
	worker.child.Stdin.Close()
	if err := writer.End(event.worker, worker.child.Wait(ctx)); err != nil {
		return err
	}
	if err := worker.child.Close(); err != nil {
		return parentFailure(event.worker, WorkerFailure{Class: "worker", Operation: "worker_process"})
	}
	removeWorkerFile(worker.path)
	return nil
}

// CleanupLegacyWorkerRequests deletes only old, recognizable pre-v8 ordinary request files.
func CleanupLegacyWorkerRequests(directory string, now time.Time) (uint64, error) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return 0, err
	}
	var removed uint64
	for _, entry := range entries {
		didRemove, err := removeLegacyWorkerRequest(directory, entry, now)
		if err != nil {
			return removed, err
		}
		if didRemove {
			removed++
		}
	}
	return removed, nil
}

// removeLegacyWorkerRequest retains recent, oversized, unrecognized and current protocol files.
func removeLegacyWorkerRequest(directory string, entry os.DirEntry, now time.Time) (bool, error) {
	if !entry.Type().IsRegular() {
		return false, nil
	}
	metadata, err := entry.Info()
	if err != nil {
		return false, err
	}
	if metadata.Size() > 64*1024*1024 || now.Sub(metadata.ModTime()) < 300*time.Second {
		return false, nil
	}
	path := filepath.Join(directory, entry.Name())
	body, err := os.ReadFile(path)
	if err != nil {
		return false, err
	}
	var metadataFields struct {
		Version uint32 `json:"protocol_version"`
		Run     string `json:"run_id"`
		Worker  uint64 `json:"worker_id"`
	}
	if decodeWorkerFields(body, &metadataFields, 0, true) != nil {
		return false, nil
	}
	if metadataFields.Version >= WorkerProtocolVersion || entry.Name() != metadataFields.Run+"-worker-"+strconv.FormatUint(metadataFields.Worker, 10)+".json" {
		return false, nil
	}
	if err := removeWorkerFile(path); err != nil {
		return false, err
	}
	return true, nil
}

// RunWorkerRequestFile activates parent-death protection before reading request data or bootstrap secrets.
func RunWorkerRequestFile(ctx context.Context, path string) error {
	stop, err := process.StartParentGuard()
	if err != nil {
		return err
	}
	defer stop()
	body, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var request WorkerRequest
	if err := json.Unmarshal(body, &request); err != nil {
		return err
	}
	return RunFetchWorker(ctx, request, os.Stdin, os.Stdout, LiveWorkerProvider)
}
