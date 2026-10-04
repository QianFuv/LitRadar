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
func runWorkerProcesses(ctx context.Context, writer *ParentWriter, requests []WorkerRequest, config WorkerProcessConfig, heartbeatInterval time.Duration, launcher workerLauncher) (RunMetrics, error) {
	if _, err := NewParentWriter(writer.content, writer.control, writer.context, writer.shouldRecordEvents, requests, writer.Metrics); err != nil {
		return writer.Metrics, err
	}
	if len(requests) == 0 {
		if err := os.MkdirAll(config.RequestDirectory, 0777); err != nil {
			return writer.Metrics, err
		}
		_, err := CleanupLegacyWorkerRequests(config.RequestDirectory, time.Now())
		return writer.Metrics, err
	}
	if err := os.MkdirAll(config.RequestDirectory, 0777); err != nil {
		return writer.Metrics, err
	}
	if _, err := CleanupLegacyWorkerRequests(config.RequestDirectory, time.Now()); err != nil {
		return writer.Metrics, err
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
		path := filepath.Join(config.RequestDirectory, request.RunId+"-worker-"+strconv.FormatUint(request.WorkerId, 10)+".json")
		body, err := json.Marshal(request)
		if err != nil {
			return writer.Metrics, err
		}
		if err := os.WriteFile(path, body, 0666); err != nil {
			removeWorkerFile(path)
			return writer.Metrics, err
		}
		child, err := launcher(lifetime, path, request.WorkerId)
		if err != nil {
			removeWorkerFile(path)
			return writer.Metrics, err
		}
		children = append(children, ownedWorker{child, path})
		if err := WriteProtocol(child.Stdin, config.Bootstrap(request)); err != nil {
			return writer.Metrics, parentProtocolFailure(request.WorkerId)
		}
		readers.Add(1)
		go func(worker uint64, child *process.Child) {
			defer readers.Done()
			reader := NewProtocolReader(child.Stdout)
			for {
				var message WorkerMessage
				err := reader.Read(&message)
				select {
				case events <- workerReaderEvent{worker, message, err}:
				case <-lifetime.Done():
					return
				}
				if err != nil {
					return
				}
			}
		}(request.WorkerId, child)
	}
	remaining := len(children)
	ticker := time.NewTicker(heartbeatInterval)
	defer ticker.Stop()
	heartbeat := func() error {
		return storage.HeartbeatLease(ctx, writer.control, writer.context.CatalogName, writer.context.ProviderName, writer.context.RunId, time.Now().Unix())
	}
	for remaining > 0 {
		select {
		case <-ctx.Done():
			return writer.Metrics, ctx.Err()
		case <-ticker.C:
			if err := heartbeat(); err != nil {
				return writer.Metrics, err
			}
		case event := <-events:
			if event.err != nil {
				var framing *ProtocolError
				if !errors.As(event.err, &framing) || framing.Kind != "end" {
					return writer.Metrics, parentProtocolFailure(event.worker)
				}
				if !writer.progress[event.worker].hasTerminal {
					return writer.Metrics, parentProtocolFailure(event.worker)
				}
				child := children[event.worker].child
				child.Stdin.Close()
				if err := writer.End(event.worker, child.Wait(ctx)); err != nil {
					return writer.Metrics, err
				}
				if err := child.Close(); err != nil {
					return writer.Metrics, parentFailure(event.worker, WorkerFailure{Class: "worker", Operation: "worker_process"})
				}
				removeWorkerFile(children[event.worker].path)
				remaining--
			} else if err := writer.Handle(ctx, event.worker, event.message, children[event.worker].child.Stdin); err != nil {
				return writer.Metrics, err
			}
		}
	}
	return writer.Metrics, nil
}

// CleanupLegacyWorkerRequests deletes only old, recognizable pre-v8 ordinary request files.
func CleanupLegacyWorkerRequests(directory string, now time.Time) (uint64, error) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return 0, err
	}
	var removed uint64
	for _, entry := range entries {
		if !entry.Type().IsRegular() {
			continue
		}
		metadata, err := entry.Info()
		if err != nil {
			return removed, err
		}
		if metadata.Size() > 64*1024*1024 || now.Sub(metadata.ModTime()) < 300*time.Second {
			continue
		}
		path := filepath.Join(directory, entry.Name())
		body, err := os.ReadFile(path)
		if err != nil {
			return removed, err
		}
		var metadataFields struct {
			Version uint32 `json:"protocol_version"`
			Run     string `json:"run_id"`
			Worker  uint64 `json:"worker_id"`
		}
		if decodeWorkerFields(body, &metadataFields, 0, true) != nil {
			continue
		}
		if metadataFields.Version >= WorkerProtocolVersion || entry.Name() != metadataFields.Run+"-worker-"+strconv.FormatUint(metadataFields.Worker, 10)+".json" {
			continue
		}
		if err := removeWorkerFile(path); err != nil {
			return removed, err
		}
		removed++
	}
	return removed, nil
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
