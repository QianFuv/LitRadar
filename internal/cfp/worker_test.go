package cfp

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"

	"path/filepath"

	"testing"
	"time"

	assets "github.com/QianFuv/LitRadar/assets/cfp"

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
