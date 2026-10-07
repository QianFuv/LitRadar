package index

import (
	"context"
	"errors"
	"fmt"
	"io"
	"math"
	"path/filepath"
	"time"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const maximumProviderPages uint64 = 100000

// ExecutionError retains the local cause while exposing only a fixed failure classification on worker pipes.
type ExecutionError struct {
	Failure WorkerFailure
	Message string
	Cause   error
}

func (err *ExecutionError) Error() string { return err.Message }
func (err *ExecutionError) Unwrap() error { return err.Cause }
func invalidWorker(reason string) error {
	return &ExecutionError{Failure: WorkerFailure{Class: "invalid_config", Operation: "configuration"}, Message: reason}
}
func workerProtocolFailure() error {
	return &ExecutionError{Failure: WorkerFailure{Class: "worker", Operation: "worker_protocol"}, Message: "worker protocol operation failed"}
}
func workerFailure(err error) WorkerFailure {
	var execution *ExecutionError
	if errors.As(err, &execution) {
		return execution.Failure
	}
	var source *provider.Error
	if errors.As(err, &source) {
		return WorkerFailure{Class: "provider", Operation: "provider_request"}
	}
	return WorkerFailure{Class: "worker", Operation: "worker_process"}
}

// WorkerProviderFactory constructs one owned fetch provider after bootstrap and request validation.
type WorkerProviderFactory func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error)

// RunFetchWorker reads the one-shot bootstrap and reports a terminal frame even when execution fails.
func RunFetchWorker(ctx context.Context, request WorkerRequest, input io.Reader, output io.Writer, factory WorkerProviderFactory) error {
	reader := NewProtocolReader(input)
	sequence := uint64(0)
	execution := func() error {
		var bootstrap WorkerBootstrap
		if err := reader.Read(&bootstrap); err != nil {
			return workerProtocolFailure()
		}
		if err := validateWorkerBootstrap(request, bootstrap); err != nil {
			return err
		}
		if err := validateWorkerRequest(request); err != nil {
			return err
		}
		implementation, closeProvider, err := factory(request, bootstrap)
		if err != nil {
			return err
		}
		if closeProvider != nil {
			defer closeProvider()
		}
		return fetchWorkerAssignments(ctx, request, implementation, reader, output, &sequence)
	}()
	terminal := WorkerMessage{Type: "succeeded", ProtocolVersion: WorkerProtocolVersion, WorkerId: request.WorkerId, Sequence: sequence}
	if execution != nil {
		failure := workerFailure(execution)
		terminal.Type = "failed"
		terminal.Failure = &failure
	}
	if err := WriteProtocol(output, terminal); err != nil {
		return workerProtocolFailure()
	}
	return nil
}
func validateWorkerBootstrap(request WorkerRequest, bootstrap WorkerBootstrap) error {
	isScholarly := request.ProviderName == "scholarly"
	if bootstrap.ProtocolVersion != WorkerProtocolVersion || bootstrap.WorkerId != request.WorkerId || request.ProviderName != "cnki" && bootstrap.CnkiCaptchaToken != nil || invalidScholarlyBootstrap(isScholarly, bootstrap) {
		return invalidWorker("worker bootstrap is invalid")
	}
	if bootstrap.ProviderProxyUrl != nil {
		if _, err := transport.ExplicitProxy(*bootstrap.ProviderProxyUrl); err != nil {
			return invalidWorker("worker bootstrap is invalid")
		}
	}
	return nil
}

// invalidScholarlyBootstrap checks paired configuration and the absolute workset directory.
func invalidScholarlyBootstrap(isScholarly bool, bootstrap WorkerBootstrap) bool {
	return isScholarly != (bootstrap.ScholarlyConfig != nil) || isScholarly != (bootstrap.ScholarlyWorksetDir != nil) || bootstrap.ScholarlyWorksetDir != nil && !filepath.IsAbs(*bootstrap.ScholarlyWorksetDir)
}
func validateWorkerRequest(request WorkerRequest) error {
	if request.ProtocolVersion != WorkerProtocolVersion || request.ProcessCount == 0 || request.WorkerId >= request.ProcessCount {
		return invalidWorker("worker protocol request is invalid")
	}
	if _, err := indexdomain.ValidateConcurrency(request.SourceWorkerCount, request.ProcessCount, request.ProviderName == "scholarly"); err != nil {
		return invalidWorker(err.Error())
	}
	ordinals := map[uint64]bool{}
	for _, assignment := range request.Assignments {
		if ordinals[assignment.JournalOrdinal] {
			return invalidWorker("worker journal assignments are invalid")
		}
		ordinals[assignment.JournalOrdinal] = true
	}
	return nil
}

// LiveWorkerProvider creates the existing source adapters and returns their explicit cleanup operation.
func LiveWorkerProvider(request WorkerRequest, bootstrap WorkerBootstrap) (provider.IndexContent, func(), error) {
	proxy := transport.Proxy{}
	if bootstrap.ProviderProxyUrl != nil {
		var err error
		proxy, err = transport.ExplicitProxy(*bootstrap.ProviderProxyUrl)
		if err != nil {
			return nil, nil, invalidWorker("worker bootstrap is invalid")
		}
	}
	switch request.ProviderName {
	case "scholarly":
		return scholarlyWorkerProvider(request, bootstrap, proxy)
	case "cnki":
		return cnkiWorkerProvider(request, bootstrap, proxy)
	default:
		return nil, nil, invalidWorker(fmt.Sprintf("index provider %s is not registered", request.ProviderName))
	}
}

// scholarlyWorkerProvider owns the transport until registry initialization succeeds.
func scholarlyWorkerProvider(request WorkerRequest, bootstrap WorkerBootstrap, proxy transport.Proxy) (provider.IndexContent, func(), error) {

	if bootstrap.ScholarlyConfig == nil || bootstrap.ScholarlyWorksetDir == nil {
		return nil, nil, invalidWorker("Scholarly workset directory is required")
	}
	config := bootstrap.ScholarlyConfig.WithWorkerContext(request.WorkerId, request.ProcessCount).WithScheduleEpoch(request.ScheduleEpochUnixMillis)
	client, err := scholarly.NewLiveTransport(config, int(request.SourceWorkerCount), proxy)
	if err != nil {
		return nil, nil, &ExecutionError{Failure: WorkerFailure{Class: "provider_setup", Operation: "provider_setup"}, Message: "scholarly indexing provider could not initialize", Cause: err}
	}
	registration, err := scholarly.NewIndexRegistration(client, config.HasSemanticScholarKey(), *bootstrap.ScholarlyWorksetDir)
	if err != nil {
		client.CloseIdleConnections()
		return nil, nil, &ExecutionError{Failure: WorkerFailure{Class: "registry", Operation: "provider_registry"}, Message: err.Error(), Cause: err}
	}
	return registration.IndexContent(), client.CloseIdleConnections, nil
}

// cnkiWorkerProvider closes both the adapter and its transport after successful construction.
func cnkiWorkerProvider(request WorkerRequest, bootstrap WorkerBootstrap, proxy transport.Proxy) (provider.IndexContent, func(), error) {

	client, err := cnki.NewLiveTransport(cnki.LiveConfig{TimeoutSeconds: request.TimeoutSeconds, CaptchaToken: bootstrap.CnkiCaptchaToken}, proxy, time.Time{})
	if err != nil {
		return nil, nil, &ExecutionError{Failure: WorkerFailure{Class: "provider_setup", Operation: "provider_setup"}, Message: "domestic CNKI indexing provider could not initialize", Cause: err}
	}
	implementation, err := sources.NewCnkiIndexProviderWithWorkers(client, int(request.SourceWorkerCount))
	if err != nil {
		client.Close()
		return nil, nil, &ExecutionError{Failure: WorkerFailure{Class: "registry", Operation: "provider_registry"}, Message: err.Error(), Cause: err}
	}
	return implementation, func() { implementation.Close(); client.Close() }, nil
}

// fetchWorkerAssignments executes assignments in their frozen request order.
func fetchWorkerAssignments(ctx context.Context, request WorkerRequest, implementation provider.IndexContent, reader *ProtocolReader, writer io.Writer, sequence *uint64) error {
	for _, assignment := range request.Assignments {
		if err := fetchWorkerAssignment(ctx, request, assignment, implementation, reader, writer, sequence); err != nil {
			return err
		}
	}
	return nil
}

// fetchWorkerAssignment checks traversal limits only after the parent acknowledges durable progress.
func fetchWorkerAssignment(ctx context.Context, request WorkerRequest, assignment WorkerAssignment, implementation provider.IndexContent, reader *ProtocolReader, writer io.Writer, sequence *uint64) error {
	checkpoint := assignment.TraversalCheckpoint
	seen := map[string]bool{}
	if checkpoint != nil {
		seen[*checkpoint] = true
	}
	for page := uint64(0); page < maximumProviderPages; page++ {
		batch, err := implementation.Fetch(ctx, assignment.Entry, domain.IndexFetchContext{Mode: assignment.Mode, CommittedAnchor: assignment.CommittedAnchor, TraversalCheckpoint: checkpoint})
		if err != nil {
			return &ExecutionError{Failure: WorkerFailure{Class: "provider", Operation: "provider_request"}, Message: err.Error(), Cause: err}
		}
		if batch.CatalogId != assignment.Entry.CatalogId {
			return invalidWorker("provider batch catalog identity is invalid")
		}
		if err := acknowledgeWorkerPage(request, assignment, batch, page, reader, writer, sequence); err != nil {
			return err
		}
		if batch.Progress.State == domain.Complete {
			break
		}
		if err := advanceWorkerCheckpoint(batch.Progress, seen, page); err != nil {
			return err
		}
		checkpoint = batch.Progress.Checkpoint
	}
	return nil
}

// acknowledgeWorkerPage advances the sequence only after an exactly correlated acknowledgement.
func acknowledgeWorkerPage(request WorkerRequest, assignment WorkerAssignment, batch domain.ProviderBatch, page uint64, reader *ProtocolReader, writer io.Writer, sequence *uint64) error {
	isComplete := batch.Progress.State == domain.Complete
	if err := WriteProtocol(writer, WorkerMessage{Type: "batch", ProtocolVersion: WorkerProtocolVersion, WorkerId: request.WorkerId, Sequence: *sequence, JournalOrdinal: assignment.JournalOrdinal, PageIndex: page, Batch: &batch}); err != nil {
		return workerProtocolFailure()
	}
	var acknowledgement ParentMessage
	if reader.Read(&acknowledgement) != nil || mismatchedWorkerAcknowledgement(acknowledgement, request, assignment, *sequence, page, isComplete) {
		return workerProtocolFailure()
	}
	if *sequence == math.MaxUint64 {
		return invalidWorker("worker sequence limit exceeded")
	}
	*sequence++
	return nil
}

// mismatchedWorkerAcknowledgement binds the acknowledgement to the emitted page and completion state.
func mismatchedWorkerAcknowledgement(acknowledgement ParentMessage, request WorkerRequest, assignment WorkerAssignment, sequence, page uint64, isComplete bool) bool {
	return acknowledgement.ProtocolVersion != WorkerProtocolVersion || acknowledgement.WorkerId != request.WorkerId || acknowledgement.Sequence != sequence || acknowledgement.JournalOrdinal != assignment.JournalOrdinal || acknowledgement.PageIndex != page || acknowledgement.IsComplete != isComplete
}

// advanceWorkerCheckpoint rejects missing, repeated and over-limit continuation after its commit.
func advanceWorkerCheckpoint(progress domain.ProviderProgress, seen map[string]bool, page uint64) error {
	if progress.Checkpoint == nil {
		return invalidWorker("provider checkpoint is missing")
	}
	if seen[*progress.Checkpoint] {
		return invalidWorker("index provider returned a repeated checkpoint")
	}
	seen[*progress.Checkpoint] = true
	if page+1 == maximumProviderPages {
		return invalidWorker("provider page limit exceeded")
	}
	return nil
}
