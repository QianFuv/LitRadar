package index

import (
	"bufio"
	"bytes"
	"context"
	contentfixture "github.com/QianFuv/LitRadar/internal/testkit/content"

	"errors"
	"io"

	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

type workerTestProvider func(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error)

func (function workerTestProvider) Fetch(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
	return function(ctx, entry, fetch)
}

func workerContentFixture(t *testing.T) (domain.JournalCatalogEntry, domain.ProviderBatch) {
	t.Helper()
	return contentfixture.Batch()
}
func workerFixtureRequest(t *testing.T) (WorkerRequest, domain.ProviderBatch) {
	t.Helper()
	entry, batch := workerContentFixture(t)
	return WorkerRequest{ProtocolVersion: 8, CatalogName: "catalog", ProviderName: "cnki", RunId: "run", WorkerId: 0, ProcessCount: 1, SourceWorkerCount: 1, TimeoutSeconds: 30, Assignments: []WorkerAssignment{{JournalOrdinal: 4, Entry: entry, Mode: domain.Incremental}}}, batch
}
func testCheckpoint(value string) *string { return &value }

func TestFetchWorkerWaitsForExactDurableAck(t *testing.T) {
	request, batch := workerFixtureRequest(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	inputReader, inputWriter := io.Pipe()
	outputReader, outputWriter := io.Pipe()
	input := bufio.NewWriter(inputWriter)
	output := bufio.NewWriter(outputWriter)
	defer inputReader.Close()
	defer inputWriter.Close()
	defer outputReader.Close()
	defer outputWriter.Close()
	go func() { <-ctx.Done(); inputReader.CloseWithError(ctx.Err()); outputReader.CloseWithError(ctx.Err()) }()
	var calls atomic.Int32
	var closed atomic.Bool
	implementation := workerTestProvider(func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
		call := calls.Add(1)
		result := batch
		if call == 1 {
			result.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("next")}
		} else {
			if fetch.TraversalCheckpoint == nil || *fetch.TraversalCheckpoint != "next" {
				return domain.ProviderBatch{}, errors.New("wrong resumed checkpoint")
			}
			result.Progress = domain.ProviderProgress{State: domain.Complete}
		}
		return result, nil
	})
	done := make(chan error, 1)
	go func() {
		done <- RunFetchWorker(ctx, request, inputReader, output, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
			return implementation, func() { closed.Store(true) }, nil
		})
		outputWriter.Close()
	}()
	if err := WriteProtocol(input, WorkerBootstrap{ProtocolVersion: 8, WorkerId: 0}); err != nil {
		t.Fatal(err)
	}
	reader := NewProtocolReader(outputReader)
	for page := uint64(0); page < 2; page++ {
		var message WorkerMessage
		if err := reader.Read(&message); err != nil {
			t.Fatal(err)
		}
		if message.Type != "batch" || message.Sequence != page || message.PageIndex != page || calls.Load() != int32(page+1) {
			t.Fatalf("message=%+v calls=%d", message, calls.Load())
		}
		if err := WriteProtocol(input, ParentMessage{"committed", 8, 0, page, 4, page, page == 1}); err != nil {
			t.Fatal(err)
		}
	}
	var terminal WorkerMessage
	if err := reader.Read(&terminal); err != nil || terminal.Type != "succeeded" || terminal.Sequence != 2 {
		t.Fatalf("terminal=%+v err=%v", terminal, err)
	}
	if err := <-done; err != nil || !closed.Load() {
		t.Fatalf("cleanup=%t err=%v", closed.Load(), err)
	}
}

func TestFetchWorkerRejectsEveryAckCorrelationMismatch(t *testing.T) {
	for _, field := range []string{"version", "worker", "sequence", "ordinal", "page", "complete"} {
		t.Run(field, func(t *testing.T) {
			request, batch := workerFixtureRequest(t)
			ack := ParentMessage{"committed", 8, 0, 0, 4, 0, true}
			switch field {
			case "version":
				ack.ProtocolVersion++
			case "worker":
				ack.WorkerId++
			case "sequence":
				ack.Sequence++
			case "ordinal":
				ack.JournalOrdinal++
			case "page":
				ack.PageIndex++
			case "complete":
				ack.IsComplete = false
			}
			var input, output bytes.Buffer
			WriteProtocol(&input, WorkerBootstrap{ProtocolVersion: 8})
			WriteProtocol(&input, ack)
			calls := 0
			err := RunFetchWorker(context.Background(), request, &input, &output, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
				return workerTestProvider(func(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error) {
					calls++
					return batch, nil
				}), nil, nil
			})
			if err != nil {
				t.Fatal(err)
			}
			reader := NewProtocolReader(&output)
			var page, terminal WorkerMessage
			if reader.Read(&page) != nil || reader.Read(&terminal) != nil || terminal.Type != "failed" || terminal.Sequence != 0 || terminal.Failure.Operation != "worker_protocol" || calls != 1 {
				t.Fatalf("terminal=%+v calls=%d", terminal, calls)
			}
		})
	}
}

func TestRepeatedCheckpointFailsOnlyAfterAck(t *testing.T) {
	request, batch := workerFixtureRequest(t)
	request.Assignments[0].TraversalCheckpoint = testCheckpoint("repeat")
	batch.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("repeat")}
	var input, output bytes.Buffer
	WriteProtocol(&input, WorkerBootstrap{ProtocolVersion: 8})
	WriteProtocol(&input, ParentMessage{"committed", 8, 0, 0, 4, 0, false})
	if err := RunFetchWorker(context.Background(), request, &input, &output, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
		return workerTestProvider(func(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error) {
			return batch, nil
		}), nil, nil
	}); err != nil {
		t.Fatal(err)
	}
	reader := NewProtocolReader(&output)
	var page, terminal WorkerMessage
	if reader.Read(&page) != nil || reader.Read(&terminal) != nil || terminal.Type != "failed" || terminal.Sequence != 1 || terminal.Failure.Class != "invalid_config" {
		t.Fatalf("terminal=%+v", terminal)
	}
}

func TestWorkerBootstrapValidationPrecedesProviderConstruction(t *testing.T) {
	request, _ := workerFixtureRequest(t)
	for _, bootstrap := range []WorkerBootstrap{{ProtocolVersion: 7}, {ProtocolVersion: 8, WorkerId: 2}, {ProtocolVersion: 8, ProviderProxyUrl: testCheckpoint("http://user:password@bad/path")}, {ProtocolVersion: 8, ScholarlyWorksetDir: testCheckpoint("relative")}} {
		var input, output bytes.Buffer
		WriteProtocol(&input, bootstrap)
		called := false
		if err := RunFetchWorker(context.Background(), request, &input, &output, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
			called = true
			return nil, nil, errors.New("must not construct")
		}); err != nil {
			t.Fatal(err)
		}
		var terminal WorkerMessage
		if NewProtocolReader(&output).Read(&terminal) != nil || terminal.Type != "failed" || terminal.Failure.Class != "invalid_config" || called {
			t.Fatalf("terminal=%+v called=%t", terminal, called)
		}
	}
}

func parentWriterFixture(t *testing.T) (*ParentWriter, WorkerRequest, domain.ProviderBatch, *storage.Connection, *storage.Connection) {
	t.Helper()
	ctx := context.Background()
	root := t.TempDir()
	content, err := storage.OpenContent(ctx, filepath.Join(root, "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { content.Close() })
	control, err := storage.OpenControl(ctx, filepath.Join(root, "control.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { control.Close() })
	request, batch := workerFixtureRequest(t)
	writerContext := WriterContext{"catalog", "cnki", "batch", "run", "epoch"}
	if err := storage.AcquireLease(ctx, control.Conn, "catalog", "cnki", "run", time.Now().Unix()); err != nil {
		t.Fatal(err)
	}
	if _, err := storage.PrepareJournalSync(ctx, control.Conn, writerContext.syncRun(request.Assignments[0]), true, "epoch"); err != nil {
		t.Fatal(err)
	}
	writer, err := NewParentWriter(content.Conn, control.Conn, writerContext, true, []WorkerRequest{request}, RunMetrics{JournalsTotal: 1})
	if err != nil {
		t.Fatal(err)
	}
	return writer, request, batch, content, control
}

type rejectedAck struct{}

func (rejectedAck) Write([]byte) (int, error) { return 0, errors.New("closed ACK pipe") }

func TestParentNeverAcknowledgesFailedCheckpoint(t *testing.T) {
	writer, _, batch, content, control := parentWriterFixture(t)
	ctx := context.Background()
	batch.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("next")}
	if _, err := control.ExecContext(ctx, `CREATE TRIGGER reject_progress BEFORE UPDATE OF traversal_checkpoint ON provider_run_checkpoints BEGIN SELECT RAISE(FAIL,'injected checkpoint failure'); END`); err != nil {
		t.Fatal(err)
	}
	message := WorkerMessage{Type: "batch", ProtocolVersion: 8, WorkerId: 0, Sequence: 0, JournalOrdinal: 4, PageIndex: 0, Batch: &batch}
	var acknowledgements bytes.Buffer
	if err := writer.Handle(ctx, 0, message, &acknowledgements); err == nil {
		t.Fatal("failed checkpoint acknowledged")
	}
	if acknowledgements.Len() != 0 || writer.Metrics.PagesCommitted != 0 || writer.progress[0].nextSequence != 0 {
		t.Fatal("failure advanced parent")
	}
	var articles int
	if err := content.QueryRowContext(ctx, "SELECT COUNT(*) FROM articles").Scan(&articles); err != nil || articles == 0 {
		t.Fatalf("content not committed: %d %v", articles, err)
	}
	if _, err := control.ExecContext(ctx, "DROP TRIGGER reject_progress"); err != nil {
		t.Fatal(err)
	}
	if err := writer.Handle(ctx, 0, message, &acknowledgements); err != nil {
		t.Fatal(err)
	}
	if writer.Metrics.PagesCommitted != 1 || writer.Metrics.ArticlesChanged != 0 || acknowledgements.Len() == 0 {
		t.Fatalf("replay duplicated or lost content: %+v", writer.Metrics)
	}
}

func TestParentAckFailureKeepsMemoryBehindDurableProgress(t *testing.T) {
	writer, _, batch, _, control := parentWriterFixture(t)
	ctx := context.Background()
	batch.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("next")}
	message := WorkerMessage{Type: "batch", ProtocolVersion: 8, WorkerId: 0, Sequence: 0, JournalOrdinal: 4, PageIndex: 0, Batch: &batch}
	if err := writer.Handle(ctx, 0, message, rejectedAck{}); err == nil {
		t.Fatal("closed ACK succeeded")
	}
	checkpoint, err := storage.ReadRunCheckpoint(ctx, control.Conn, storage.SyncScope{CatalogName: "catalog", ProviderName: "cnki", CatalogId: batch.CatalogId})
	if err != nil || checkpoint.TraversalCheckpoint == nil || *checkpoint.TraversalCheckpoint != "next" || writer.progress[0].nextSequence != 0 || writer.Metrics.PagesCommitted != 1 {
		t.Fatalf("checkpoint=%+v metrics=%+v err=%v", checkpoint, writer.Metrics, err)
	}
}

func TestParentPipeIdentityAndTerminalRules(t *testing.T) {
	for _, field := range []string{"worker", "sequence", "ordinal", "page", "catalog", "version"} {
		t.Run(field, func(t *testing.T) {
			writer, _, batch, content, _ := parentWriterFixture(t)
			message := WorkerMessage{Type: "batch", ProtocolVersion: 8, JournalOrdinal: 4, Batch: &batch}
			switch field {
			case "worker":
				message.WorkerId++
			case "sequence":
				message.Sequence++
			case "ordinal":
				message.JournalOrdinal++
			case "page":
				message.PageIndex++
			case "catalog":
				batch.CatalogId = "wrong"
			case "version":
				message.ProtocolVersion++
			}
			var ack bytes.Buffer
			if writer.Handle(context.Background(), 0, message, &ack) == nil {
				t.Fatal("invalid message accepted")
			}
			var count int
			content.QueryRowContext(context.Background(), "SELECT COUNT(*) FROM articles").Scan(&count)
			if count != 0 || ack.Len() != 0 {
				t.Fatal("invalid pipe message mutated content")
			}
		})
	}
	writer, _, batch, _, _ := parentWriterFixture(t)
	ctx := context.Background()
	terminal := WorkerMessage{Type: "succeeded", ProtocolVersion: 8}
	if writer.Handle(ctx, 0, terminal, io.Discard) == nil || writer.End(0, nil) == nil {
		t.Fatal("premature success accepted")
	}
	if err := writer.Handle(ctx, 0, WorkerMessage{Type: "batch", ProtocolVersion: 8, JournalOrdinal: 4, Batch: &batch}, io.Discard); err != nil {
		t.Fatal(err)
	}
	terminal.Sequence = 1
	if err := writer.Handle(ctx, 0, terminal, io.Discard); err != nil {
		t.Fatal(err)
	}
	if writer.End(0, errors.New("nonzero")) == nil || writer.Handle(ctx, 0, terminal, io.Discard) == nil {
		t.Fatal("bad terminal lifecycle accepted")
	}
	if err := writer.End(0, nil); err != nil {
		t.Fatal(err)
	}
}

func TestWorkerPartitionSkipsCompletedBeforeRoundRobin(t *testing.T) {
	_, request, _, _, control := parentWriterFixture(t)
	ctx := context.Background()
	writerContext := WriterContext{"catalog", "cnki", "batch", "run", "epoch"}
	first := request.Assignments[0].Entry
	entries := []domain.JournalCatalogEntry{first, first, first}
	entries[1].CatalogId = "journal-second"
	entries[2].CatalogId = "journal-third"
	if err := storage.CompleteSyncRun(ctx, control.Conn, writerContext.syncRun(request.Assignments[0]), nil, "epoch"); err != nil {
		t.Fatal(err)
	}
	requests, metrics, err := PrepareWorkerRequests(ctx, control.Conn, writerContext, entries, domain.Incremental, true, indexdomain.Concurrency{WorkerCount: 1, ProcessCount: 3, AggregateCapacity: 3}, 0, 30)
	if err != nil || len(requests) != 2 || metrics.JournalsResumed != 1 || requests[0].Assignments[0].JournalOrdinal != 1 || requests[1].Assignments[0].JournalOrdinal != 2 {
		t.Fatalf("requests=%+v metrics=%+v err=%v", requests, metrics, err)
	}
	onlyDone, metrics, err := PrepareWorkerRequests(ctx, control.Conn, writerContext, entries[:1], domain.Incremental, true, indexdomain.Concurrency{WorkerCount: 1, ProcessCount: 3}, 0, 30)
	if err != nil || len(onlyDone) != 0 || metrics.JournalsResumed != 1 {
		t.Fatal("completed journal spawned executor", err)
	}
}

func TestWorkerFailureDoesNotExposeProviderMessage(t *testing.T) {
	request, _ := workerFixtureRequest(t)
	var input, output bytes.Buffer
	WriteProtocol(&input, WorkerBootstrap{ProtocolVersion: 8})
	err := RunFetchWorker(context.Background(), request, &input, &output, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
		return workerTestProvider(func(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error) {
			return domain.ProviderBatch{}, errors.New("secret-provider-response")
		}), nil, nil
	})
	if err != nil || strings.Contains(output.String(), "secret-provider-response") {
		t.Fatal("failure leaked", err)
	}
	var terminal WorkerMessage
	if NewProtocolReader(&output).Read(&terminal) != nil || terminal.Type != "failed" || terminal.Failure.Class != "provider" {
		t.Fatalf("terminal=%+v", terminal)
	}
}
