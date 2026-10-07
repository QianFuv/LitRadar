package index

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/platform/process"
	"github.com/QianFuv/LitRadar/internal/provider"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// TestIndexChildFixture runs the task-owned child protocol and lifecycle fixtures.
func TestIndexChildFixture(t *testing.T) {
	mode := os.Getenv("LITRADAR_INDEX_CHILD_FIXTURE")
	if mode == "" {
		return
	}
	if mode == "stderr" {
		fmt.Fprint(os.Stderr, "inherited-index-diagnostic")
		os.Exit(0)
	}
	if mode == "stderr-owner" {
		runIndexStderrOwnerFixture()
	}
	stop, err := process.StartParentGuard()
	if err != nil {
		os.Exit(20)
	}
	defer stop()
	request := readIndexChildFixtureRequest()
	runEarlyIndexChildFixture(mode, request)
	implementation := indexChildFixtureProvider(mode)
	err = RunFetchWorker(context.Background(), request, os.Stdin, os.Stdout, func(request WorkerRequest, bootstrap WorkerBootstrap) (provider.IndexContent, func(), error) {
		if bootstrap.CnkiCaptchaToken == nil || *bootstrap.CnkiCaptchaToken != "private-bootstrap-secret" {
			return nil, nil, errors.New("bootstrap missing")
		}
		return implementation, nil, nil
	})
	if err != nil {
		os.Exit(24)
	}
	finishIndexChildFixture(mode)
}

// runIndexStderrOwnerFixture verifies inherited diagnostics stay out of the captured output.
func runIndexStderrOwnerFixture() {
	executable, _ := os.Executable()
	environment := []string{}
	for _, value := range os.Environ() {
		if !strings.HasPrefix(value, "LITRADAR_INDEX_CHILD_FIXTURE=") {
			environment = append(environment, value)
		}
	}
	environment = append(environment, "LITRADAR_INDEX_CHILD_FIXTURE=stderr")
	child, err := process.Start(context.Background(), process.Config{Path: executable, Args: []string{"-test.run=^TestIndexChildFixture$"}, Environment: environment, InheritStderr: true, OutputLimit: 1024})
	if err != nil {
		os.Exit(30)
	}
	if child.Wait(context.Background()) != nil {
		child.Close()
		os.Exit(31)
	}
	if child.Close() != nil {
		os.Exit(32)
	}
	diagnostics, truncated := child.Diagnostics()
	if len(diagnostics) != 0 || truncated {
		os.Exit(33)
	}
	os.Exit(0)
}

// readIndexChildFixtureRequest rejects bootstrap secrets before strict request decoding.
func readIndexChildFixtureRequest() WorkerRequest {
	body, err := os.ReadFile(os.Getenv("LITRADAR_INDEX_CHILD_REQUEST"))
	if err != nil {
		os.Exit(21)
	}
	if strings.Contains(string(body), "private-bootstrap-secret") || os.Getenv("LITRADAR_CNKI_CAPTCHA_TOKEN") != "" {
		os.Exit(22)
	}
	var request WorkerRequest
	if json.Unmarshal(body, &request) != nil {
		os.Exit(23)
	}
	return request
}

// runEarlyIndexChildFixture emits the original malformed and terminal lifecycle scenarios.
func runEarlyIndexChildFixture(mode string, request WorkerRequest) {
	if mode == "malformed" {
		fmt.Fprint(os.Stdout, "!")
		os.Exit(0)
	}
	if mode == "eof" {
		os.Exit(0)
	}
	if mode == "wait" {
		io.Copy(io.Discard, os.Stdin)
		os.Exit(0)
	}
	if mode == "failed" {
		WriteProtocol(os.Stdout, WorkerMessage{Type: "failed", ProtocolVersion: 8, WorkerId: request.WorkerId, Failure: &WorkerFailure{Class: "provider", Operation: "provider_request"}})
		os.Exit(0)
	}
	if mode == "premature" {
		WriteProtocol(os.Stdout, WorkerMessage{Type: "succeeded", ProtocolVersion: 8, WorkerId: request.WorkerId})
		os.Exit(0)
	}
}

// indexChildFixtureProvider retains independent acknowledged page counters per journal.
func indexChildFixtureProvider(mode string) provider.IndexContent {
	pages := map[string]int{}
	implementation := workerTestProvider(func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
		return indexChildFixturePage(mode, pages, entry, fetch)
	})
	return implementation
}

// finishIndexChildFixture preserves stdin EOF, exit status and extra-frame fixture behavior.
func finishIndexChildFixture(mode string) {
	if mode == "stdin-eof" {
		os.Stdout.Close()
		io.Copy(io.Discard, os.Stdin)
	}
	if mode == "nonzero" {
		os.Exit(25)
	}
	if mode == "extra" {
		fmt.Fprintln(os.Stdout, `{"type":"succeeded","protocol_version":8,"worker_id":0,"sequence":3}`)
	}
	os.Exit(0)
}
func processWriterFixture(t *testing.T) (*ParentWriter, []WorkerRequest, WorkerProcessConfig, *storage.Connection) {
	t.Helper()
	_, base, _, content, control := parentWriterFixture(t)
	ctx := context.Background()
	writerContext := WriterContext{"catalog", "cnki", "batch", "run", "epoch"}
	entries := []domain.JournalCatalogEntry{base.Assignments[0].Entry, base.Assignments[0].Entry, base.Assignments[0].Entry}
	for ordinal := range entries {
		entries[ordinal].CatalogId = "synthetic-" + strconv.Itoa(ordinal)
		entries[ordinal].Issn = nil
		entries[ordinal].Eissn = nil
		entries[ordinal].AllIssns = nil
	}
	requests, metrics, err := PrepareWorkerRequests(ctx, control.Conn, writerContext, entries, domain.Incremental, true, indexdomain.Concurrency{WorkerCount: 1, ProcessCount: 3, AggregateCapacity: 3}, 0, 30)
	if err != nil {
		t.Fatal(err)
	}
	writer, err := NewParentWriter(content.Conn, control.Conn, writerContext, true, requests, metrics)
	if err != nil {
		t.Fatal(err)
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	config := WorkerProcessConfig{Executable: executable, RequestDirectory: filepath.Join(t.TempDir(), "requests"), Bootstrap: func(request WorkerRequest) WorkerBootstrap {
		return WorkerBootstrap{ProtocolVersion: 8, WorkerId: request.WorkerId, CnkiCaptchaToken: testCheckpoint("private-bootstrap-secret")}
	}}
	return writer, requests, config, content
}
func fixtureLauncher(mode string, started *[]*process.Child) workerLauncher {
	return func(ctx context.Context, path string, worker uint64) (*process.Child, error) {
		executable, err := os.Executable()
		if err != nil {
			return nil, err
		}
		environment := append(workerEnvironment(), "LITRADAR_INDEX_CHILD_FIXTURE="+mode, "LITRADAR_INDEX_CHILD_REQUEST="+path)
		child, err := process.Start(ctx, process.Config{Path: executable, Args: []string{"-test.run=^TestIndexChildFixture$"}, Environment: environment, StreamStdout: true, OutputLimit: 4096})
		if err == nil {
			*started = append(*started, child)
		}
		return child, err
	}
}

// TestActualThreeWorkersDurablyCommitNinePages checks durable metrics, content, file cleanup and reaping.
func TestActualThreeWorkersDurablyCommitNinePages(t *testing.T) {
	writer, requests, config, content := processWriterFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	var started []*process.Child
	t.Setenv("LITRADAR_CNKI_CAPTCHA_TOKEN", "must-not-inherit")
	metrics, err := runWorkerProcesses(ctx, writer, requests, config, 50*time.Millisecond, fixtureLauncher("normal", &started))
	if err != nil {
		t.Fatal(err)
	}
	if metrics.PagesCommitted != 9 || metrics.ArticlesChanged != 36 || metrics.JournalsSucceeded != 3 {
		t.Fatalf("metrics=%+v", metrics)
	}
	var count int
	if err := content.QueryRowContext(ctx, "SELECT count(*) FROM articles").Scan(&count); err != nil || count != 36 {
		t.Fatal(count, err)
	}
	paths, err := os.ReadDir(config.RequestDirectory)
	if err != nil || len(paths) != 0 {
		t.Fatal("request files retained", err)
	}
	assertIndexWorkersReaped(t, ctx, started)
}

func TestSuccessfulWorkerReceivesStdinEofBeforeWait(t *testing.T) {
	writer, requests, config, _ := processWriterFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	var started []*process.Child
	metrics, err := runWorkerProcesses(ctx, writer, requests, config, 50*time.Millisecond, fixtureLauncher("stdin-eof", &started))
	if err != nil || metrics.JournalsSucceeded != 3 {
		t.Fatalf("metrics=%+v err=%v", metrics, err)
	}
}

func TestActualWorkerBackpressurePreservesLargePages(t *testing.T) {
	writer, requests, config, content := processWriterFixture(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	var started []*process.Child
	metrics, err := runWorkerProcesses(ctx, writer, requests, config, 50*time.Millisecond, fixtureLauncher("pressure", &started))
	if err != nil || metrics.JournalsSucceeded != 3 || metrics.PagesCommitted != 9 || metrics.ArticlesChanged != 36 {
		t.Fatalf("metrics=%+v err=%v", metrics, err)
	}
	var count int
	if err := content.QueryRowContext(ctx, "SELECT count(*) FROM articles WHERE length(abstract_text)=?1", len("bounded-pipe-page ")*8192-1).Scan(&count); err != nil || count != 36 {
		t.Fatalf("large payload count=%d err=%v", count, err)
	}
}

func TestIndexProcessInheritsStderrWithoutProtocolMixing(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, executable, "-test.run=^TestIndexChildFixture$")
	command.Env = append(os.Environ(), "LITRADAR_INDEX_CHILD_FIXTURE=stderr-owner")
	var stdout, stderr strings.Builder
	command.Stdout = &stdout
	command.Stderr = &stderr
	if err := command.Run(); err != nil || stderr.String() != "inherited-index-diagnostic" || stdout.Len() != 0 {
		t.Fatalf("err=%v stderr=%q stdout=%q", err, stderr.String(), stdout.String())
	}
}

func TestActualWorkerFailuresCleanProcessesAndRequests(t *testing.T) {
	for _, mode := range []string{"malformed", "eof", "failed", "premature", "nonzero", "extra", "partial-start", "cancel"} {
		t.Run(mode, func(t *testing.T) {
			writer, requests, config, _ := processWriterFixture(t)
			duration := 10 * time.Second
			if mode == "cancel" {
				duration = 600 * time.Millisecond
			}
			ctx, cancel := context.WithTimeout(context.Background(), duration)
			defer cancel()
			var started []*process.Child
			selected := mode
			if mode == "partial-start" {
				selected = "normal"
			}
			if mode == "cancel" {
				selected = "wait"
			}
			launcher := fixtureLauncher(selected, &started)
			if mode == "partial-start" {
				original := launcher
				launcher = func(ctx context.Context, path string, worker uint64) (*process.Child, error) {
					if worker == 1 {
						return nil, errors.New("injected spawn failure")
					}
					return original(ctx, path, worker)
				}
			}
			_, err := runWorkerProcesses(ctx, writer, requests, config, 50*time.Millisecond, launcher)
			if err == nil {
				t.Fatal("invalid process lifecycle succeeded")
			}
			paths, readErr := os.ReadDir(config.RequestDirectory)
			if readErr != nil || len(paths) != 0 {
				t.Fatal("request cleanup failed", readErr)
			}
			verification, stop := context.WithTimeout(context.Background(), 3*time.Second)
			defer stop()
			for _, child := range started {
				err := child.Wait(verification)
				if errors.Is(err, context.DeadlineExceeded) {
					t.Fatal("worker not reaped")
				}
			}
		})
	}
}

func TestLegacyRequestCleanupKeepsUnknownAndCurrentFiles(t *testing.T) {
	directory := t.TempDir()
	now := time.Now()
	old := now.Add(-301 * time.Second)
	cases := map[string]string{"old-worker-1.json": `{"protocol_version":7,"run_id":"old","worker_id":1,"secret":"legacy"}`, "sequence-worker-2.json": `[7,"sequence",2]`, "current-worker-1.json": `{"protocol_version":8,"run_id":"current","worker_id":1}`, "duplicate-worker-1.json": `{"protocol_version":7,"protocol_version":6,"run_id":"duplicate","worker_id":1}`, "wrong.json": `{"protocol_version":7,"run_id":"old","worker_id":1}`, "invalid-worker-1.json": `{"protocol_version":7,"run_id":null,"worker_id":1}`}
	for name, body := range cases {
		path := filepath.Join(directory, name)
		if err := os.WriteFile(path, []byte(body), 0600); err != nil {
			t.Fatal(err)
		}
		if err := os.Chtimes(path, old, old); err != nil {
			t.Fatal(err)
		}
	}
	removed, err := CleanupLegacyWorkerRequests(directory, now)
	if err != nil || removed != 2 {
		t.Fatalf("removed=%d err=%v", removed, err)
	}
	for _, name := range []string{"current-worker-1.json", "duplicate-worker-1.json", "wrong.json", "invalid-worker-1.json"} {
		if _, err := os.Stat(filepath.Join(directory, name)); err != nil {
			t.Fatal(name, err)
		}
	}
}

// indexChildFixturePage verifies acknowledged traversal before constructing each synthetic page.
func indexChildFixturePage(mode string, pages map[string]int, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
	page := pages[entry.CatalogId]
	pages[entry.CatalogId]++
	progress := domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint(strconv.Itoa(page + 1))}
	if page == 2 {
		progress = domain.ProviderProgress{State: domain.Complete, NextAnchor: testCheckpoint("done")}
	}
	if page > 0 && (fetch.TraversalCheckpoint == nil || *fetch.TraversalCheckpoint != strconv.Itoa(page)) {
		return domain.ProviderBatch{}, errors.New("checkpoint did not follow acknowledged page")
	}
	articles := []domain.ArticleDraft{}
	for ordinal := 0; ordinal < 4; ordinal++ {
		articles = append(articles, domain.ArticleDraft{CatalogId: entry.CatalogId, Title: fmt.Sprintf("Synthetic %d %d", page, ordinal), Doi: testCheckpoint(fmt.Sprintf("10.1234/%s-%d-%d", entry.CatalogId, page, ordinal)), Authors: []domain.ArticleAuthorDraft{}, RetractionDois: []string{}})
		if mode == "pressure" {
			articles[len(articles)-1].AbstractText = testCheckpoint(strings.TrimSpace(strings.Repeat("bounded-pipe-page ", 8192)))
		}
	}
	return domain.ProviderBatch{CatalogId: entry.CatalogId, Journal: domain.JournalDraft{CatalogId: entry.CatalogId}, Articles: articles, Progress: progress}, nil
}

// assertIndexWorkersReaped checks every launched leader after successful request cleanup.
func assertIndexWorkersReaped(t *testing.T, ctx context.Context, started []*process.Child) {
	t.Helper()
	for _, child := range started {
		if err := child.Wait(ctx); err != nil {
			t.Fatal(err)
		}
	}
}
