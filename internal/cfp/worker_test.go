package cfp

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	assets "github.com/QianFuv/LitRadar/assets/cfp"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func TestBundledSeedResumesHistoricalLineEndings(t *testing.T) {
	canonical := bytes.ReplaceAll(assets.Seed(), []byte("\r\n"), []byte("\n"))
	for _, scenario := range []struct {
		name        string
		payload     []byte
		shouldMatch bool
	}{{"unix", canonical, true}, {"windows", bytes.ReplaceAll(canonical, []byte("\n"), []byte("\r\n")), true}, {"different-content", append(append([]byte{}, canonical...), ' '), false}} {
		t.Run(scenario.name, func(t *testing.T) {
			repository := workerRepository(t, "")
			original, err := repository.ImportSeed(context.Background(), assets.SeedId, scenario.payload)
			if err != nil {
				t.Fatal(err)
			}
			result, err := EnsureSeed(context.Background(), repository)
			if scenario.shouldMatch {
				if err != nil || result.DidImport || result.ContentHash != original.ContentHash || result.ContentHash != fmt.Sprintf("%x", sha256.Sum256(scenario.payload)) {
					t.Fatal("existing seed identity not preserved", result, err)
				}
			} else if err == nil {
				t.Fatal("arbitrary changed bytes accepted")
			}
			repeated, err := repository.ImportSeed(context.Background(), assets.SeedId, scenario.payload)
			if err != nil || repeated.DidImport || repeated.ContentHash != original.ContentHash {
				t.Fatal("historical marker rewritten", repeated, err)
			}
		})
	}
}

func TestRustGoCaptureHandoffs(t *testing.T) {
	binary := os.Getenv("LITRADAR_CFP_WORKER_ORACLE")
	if binary == "" {
		t.Skip("unified CFP runner supplies the verified original worker")
	}
	data, err := os.ReadFile("../../tests/migration/cfp/worker-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Name     string
			Input    json.RawMessage
			Expected struct{ Outcome FullTextResult }
		}
	}
	if json.Unmarshal(data, &fixture) != nil {
		t.Fatal("invalid worker corpus")
	}
	for _, entry := range fixture.Cases {
		if entry.Name != "full-complete" && entry.Name != "full-partial" && entry.Name != "full-mixed-resume" {
			continue
		}
		for _, isRustFirst := range []bool{true, false} {
			direction := "go-to-rust"
			if isRustFirst {
				direction = "rust-to-go"
			}
			t.Run(entry.Name+"/"+direction, func(t *testing.T) {
				directory := t.TempDir()
				path := filepath.Join(directory, "state.sqlite")
				var input workerInput
				json.Unmarshal(entry.Input, &input)
				var request map[string]any
				json.Unmarshal(entry.Input, &request)
				request["directory"] = directory
				callRust := func(isResume bool) FullTextResult {
					t.Helper()
					request["resume"] = isResume
					if isResume {
						delete(request, "capture")
					}
					encoded, _ := json.Marshal(request)
					ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
					defer cancel()
					command := exec.CommandContext(ctx, binary)
					command.Stdin = bytes.NewReader(append(encoded, '\n'))
					var diagnostic bytes.Buffer
					command.Stderr = &diagnostic
					output, err := command.Output()
					if err != nil {
						t.Fatal(err, diagnostic.String())
					}
					var observation struct{ Outcome FullTextResult }
					if err := json.Unmarshal(output, &observation); err != nil {
						t.Fatal(err)
					}
					return observation.Outcome
				}
				if isRustFirst {
					if result := callRust(false); result.Status != entry.Expected.Outcome.Status {
						t.Fatal(result)
					}
				} else {
					if _, err := migration.Migrate(context.Background(), path); err != nil {
						t.Fatal(err)
					}
				}
				repository, err := storage.Open(path)
				if err != nil {
					t.Fatal(err)
				}
				defer repository.Close()
				if !isRustFirst {
					if _, err = repository.ImportSeed(context.Background(), "fixture", []byte(input.Seed)); err != nil {
						t.Fatal(err)
					}
					if err = os.WriteFile(filepath.Join(directory, "journal_a.json"), input.Capture, 0600); err != nil {
						t.Fatal(err)
					}
				}
				result, err := RefreshFullTextSource(context.Background(), repository, input.Config, DefaultRefreshOptions(), time.Now().Add(20*time.Second), &directory, true)
				if err != nil || !reflect.DeepEqual(result, entry.Expected.Outcome) {
					t.Fatal("Go could not resume original evidence", result, err)
				}
				if !isRustFirst {
					if result := callRust(true); !reflect.DeepEqual(result, entry.Expected.Outcome) {
						t.Fatal("Rust could not resume Go evidence", result)
					}
				}
				journals, err := repository.LoadJournals(context.Background())
				if err != nil || journals[0].Sources[0].Revision != 3 {
					t.Fatal("handoff did not publish twice", journals, err)
				}
			})
		}
	}
}

type workerInput struct {
	Op        string
	Config    SourceConfig
	Payload   string
	Source    domain.Source
	Documents []struct {
		Url, Text string
		Error     bool
	}
	Seed, Html         string
	Capture            json.RawMessage
	Cancelled, Expired bool
}

func workerRepository(t *testing.T, seed string) *storage.Repository {
	t.Helper()
	path := filepath.Join(t.TempDir(), "state.sqlite")
	if _, err := migration.Migrate(context.Background(), path); err != nil {
		t.Fatal(err)
	}
	repository, err := storage.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	if seed != "" {
		if _, err := repository.ImportSeed(context.Background(), "fixture", []byte(seed)); err != nil {
			t.Fatal(err)
		}
	}
	return repository
}

type fixedDocument Document

func (document fixedDocument) Fetch(context.Context, SourceConfig, string, time.Time) (Document, error) {
	return Document(document), nil
}

func workerObservation(t *testing.T, input workerInput) any {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if input.Cancelled {
		cancel()
	}
	deadline := time.Now().Add(20 * time.Second)
	if input.Expired {
		deadline = time.Now()
	}
	var result any
	var err error
	switch input.Op {
	case "envelope":
		result, err = decodeObscura(input.Config, []byte(input.Payload))
	case "recover":
		cache := newCaptureCache()
		cache.create = func(RefreshOptions) captureTransport {
			t.Fatal("unexpected live capture from frozen observation")
			return nil
		}
		for _, record := range input.Documents {
			captured := capturedDocument{document: Document{FinalUrl: record.Url, Text: record.Text, Format: "html"}}
			if record.Error {
				captured.err = ErrChallenge
			}
			cache.documents[record.Url] = captured
			cache.browserAttempts[record.Url] = true
		}
		result, err = recoverOriginal(ctx, input.Source, input.Config, cache, DefaultRefreshOptions(), deadline)
	default:
		repository := workerRepository(t, input.Seed)
		if input.Op == "full" {
			directory := t.TempDir()
			name := strings.ReplaceAll(input.Config.SourceKey, ":", "_") + ".json"
			if err := os.WriteFile(filepath.Join(directory, name), input.Capture, 0600); err != nil {
				t.Fatal(err)
			}
			result, err = RefreshFullTextSource(ctx, repository, input.Config, DefaultRefreshOptions(), deadline, &directory, true)
		} else {
			result, err = RefreshSource(ctx, repository, input.Config, fixedDocument{FinalUrl: input.Config.DiscoveryUrl, Text: input.Html, Format: "html"}, deadline)
		}
		if err != nil {
			result = map[string]string{"error": err.Error()}
		}
		originals, loadErr := repository.LoadOriginals(context.Background(), input.Config.SourceKey)
		if loadErr != nil {
			t.Fatal(loadErr)
		}
		journals, loadErr := repository.LoadJournals(context.Background())
		if loadErr != nil {
			t.Fatal(loadErr)
		}
		state := []map[string]any{}
		for _, journal := range journals {
			for _, source := range journal.Sources {
				state = append(state, map[string]any{"sourceKey": source.SourceKey, "status": source.Status, "revision": source.Revision, "hasAttempt": source.LastAttempt != nil, "hasSuccess": source.LastSuccess != nil, "hasLease": source.LeaseExpiresAt != nil, "error": source.LastError})
			}
		}
		return map[string]any{"outcome": result, "originals": originals, "state": state}
	}
	if err != nil {
		return map[string]string{"error": err.Error()}
	}
	return result
}

func TestOriginalWorkerObservations(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/cfp/worker-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		ObservedDate string
		Cases        []struct {
			Name     string
			Input    workerInput
			Expected json.RawMessage
		}
	}
	if err = json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Cases) != 43 {
		t.Fatal("missing worker observations")
	}
	for _, entry := range fixture.Cases {
		t.Run(entry.Name, func(t *testing.T) {
			actual, err := json.Marshal(workerObservation(t, entry.Input))
			if err != nil {
				t.Fatal(err)
			}
			expected := entry.Expected
			if entry.Input.Op == "refresh" {
				expected = []byte(strings.ReplaceAll(string(expected), fixture.ObservedDate, time.Now().UTC().Format("2006-01-02")))
			}
			var actualValue, expectedValue any
			json.Unmarshal(actual, &actualValue)
			json.Unmarshal(expected, &expectedValue)
			if !reflect.DeepEqual(actualValue, expectedValue) {
				t.Fatalf("got %s\nwant %s", actual, expected)
			}
		})
	}
}

func TestEvidenceFailurePreservesPublishedResult(t *testing.T) {
	seed := `{"formatVersion":1,"sources":[{"catalogIds":["a"],"journalTitle":"Example Journal","title":"Original topic","typeText":"Special Issue","dateText":"Submission deadline: 31 December 2026","sourceUrl":"https://link.springer.com/collections/a","scope":"Preview...","checkedOn":"2026-09-15"}],"emptyJournals":[]}`
	repository := workerRepository(t, seed)
	html := `<h1 data-test="collection-title">Original topic</h1><div data-test="collection-description"><p>Every original research topic.</p><p>Authors should prepare the complete experimental protocol.</p></div>`
	payload, _ := json.Marshal(map[string]string{"protocol": obscuraProtocol, "finalUrl": "https://link.springer.com/collections/a", "html": html})
	path := helperExecutable(t, "success", string(payload))
	options := DefaultRefreshOptions()
	options.ObscuraPath = &path
	config := SourceConfig{SourceKey: "journal:a", CatalogIds: []string{"a"}, DiscoveryUrl: "https://link.springer.com/collections/a", ConfigVersion: 1, Adapter: SnapshotOnly}
	directory := t.TempDir()
	if err := os.Mkdir(filepath.Join(directory, "journal_a.json"), 0700); err != nil {
		t.Fatal(err)
	}
	result, err := RefreshFullTextSource(context.Background(), repository, config, options, time.Now().Add(20*time.Second), &directory, false)
	var failure *EvidenceError
	if !errors.As(err, &failure) || result.Status != "success" || result.Updated != 1 || failure.Result.Status != "success" {
		t.Fatal("lost durable publication outcome", result, err)
	}
	journals, err := repository.LoadJournals(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if journals[0].Sources[0].Revision != 2 || journals[0].Sources[0].Status != "success" || journals[0].Notices[0].Scope != "Every original research topic." {
		t.Fatal("evidence failure rolled back publication", journals)
	}
}

func TestInvalidWorkerBudgetsDoNotImportSeed(t *testing.T) {
	repository := workerRepository(t, "")
	for _, options := range []RefreshOptions{{}, {SourceTimeout: 601 * time.Second, OverallTimeout: time.Second}, {SourceTimeout: time.Second, OverallTimeout: 3601 * time.Second}} {
		if _, err := RefreshSources(context.Background(), repository, nil, options); err == nil {
			t.Fatal("invalid discovery budget")
		}
		if _, err := RefreshFullTexts(context.Background(), repository, nil, options, nil, false); err == nil {
			t.Fatal("invalid full text budget")
		}
	}
	journals, err := repository.LoadJournals(context.Background())
	if err != nil || len(journals) != 0 {
		t.Fatal("invalid budget imported seed", err)
	}
}
