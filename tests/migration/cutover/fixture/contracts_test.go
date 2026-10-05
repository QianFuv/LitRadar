package main

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"log/slog"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"

	"github.com/QianFuv/LitRadar/internal/cfp"
	authdomain "github.com/QianFuv/LitRadar/internal/domain/auth"
	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	"github.com/QianFuv/LitRadar/internal/recommend"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

func TestReviewedPublisherRegistryContracts(t *testing.T) {
	registry := map[string]cfp.SourceConfig{}
	for _, source := range cfp.Registry() {
		registry[source.JournalTitle] = source
	}
	if !slices.Contains(registry["Accident Analysis and Prevention"].IdentityTexts, "Accident Analysis & Prevention") {
		t.Fatal("reviewed ampersand identity missing")
	}
	location, err := whatwg.NewParser().Parse("https://www.sciencedirect.com/special-issue/336912/original")
	if err != nil || !registry["Journal of Econometrics"].PermitsUrl(location) {
		t.Fatal("reviewed preview scope rejected", err)
	}
	if !registry["Fundamental Research"].RetainsPreviousNotices {
		t.Fatal("incremental publisher may drop earlier notices")
	}
}

func TestRecommendationConfigurationFormattingNeverDisclosesSecrets(t *testing.T) {
	sentinel := "migration-sensitive-config"
	global := recommend.GlobalConfig{AiApiKey: sentinel, AiBaseUrl: sentinel, AiSystemPrompt: &sentinel}
	runtime := recommend.AiRuntimeConfig{BaseUrl: sentinel, ApiKey: sentinel, Model: sentinel, SystemPrompt: sentinel}
	for _, value := range []any{global, &global, runtime, &runtime} {
		var log bytes.Buffer
		slog.New(slog.NewJSONHandler(&log, nil)).Info("configuration", "configuration", value)
		for _, output := range []string{fmt.Sprintf("%v", value), fmt.Sprintf("%+v", value), fmt.Sprintf("%#v", value), log.String()} {
			if strings.Contains(output, sentinel) || !strings.Contains(output, "REDACTED") {
				t.Fatal("configuration diagnostics disclosed sensitive values")
			}
		}
	}
}

func TestConcurrencyDefaultsAndExplicitCountsPreserveOriginalLimits(t *testing.T) {
	for _, scholarly := range []bool{false, true} {
		for _, workers := range []uint64{0, 1, 6, 16, 32, 33} {
			for _, processes := range []uint64{0, 1, 2, 3, 4, 32, 33} {
				actual, err := indexdomain.ValidateConcurrency(workers, processes, scholarly)
				limit := uint64(32)
				if scholarly {
					limit = 96
				}
				valid := workers >= 1 && workers <= 32 && processes >= 1 && processes <= 32 && (!scholarly || processes <= 3) && workers*processes <= limit
				if (err == nil) != valid {
					t.Fatalf("workers=%d processes=%d scholarly=%t err=%v", workers, processes, scholarly, err)
				}
				if valid && (actual.WorkerCount != workers || actual.ProcessCount != processes || actual.AggregateCapacity != workers*processes) {
					t.Fatal("explicit counts were clamped")
				}
			}
		}
		defaults, err := indexdomain.ResolveConcurrency(nil, nil, scholarly)
		expectedProcesses := uint64(1)
		if scholarly {
			expectedProcesses = 3
		}
		if err != nil || defaults.WorkerCount != 6 || defaults.ProcessCount != expectedProcesses {
			t.Fatal(defaults, err)
		}
		workers, processes := uint64(2), uint64(2)
		first, err := indexdomain.ResolveConcurrency(&workers, nil, scholarly)
		if err != nil || first.WorkerCount != 2 || first.ProcessCount != expectedProcesses {
			t.Fatal(first, err)
		}
		second, err := indexdomain.ResolveConcurrency(nil, &processes, scholarly)
		if err != nil || second.WorkerCount != 6 || second.ProcessCount != 2 {
			t.Fatal(second, err)
		}
	}
}

func TestConcurrentAuditWritesSurviveOrdinaryLogPressure(t *testing.T) {
	ctx := context.Background()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := authmigration.Migrate(ctx, filename); err != nil {
		t.Fatal(err)
	}
	repository, err := auth.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	logger := slog.New(slog.NewJSONHandler(io.Discard, nil))
	failures := make(chan error, 4)
	var workers sync.WaitGroup
	for worker := range 4 {
		workers.Go(func() {
			for ordinal := range 50 {
				logger.Info("ordinary concurrent event", "worker", worker, "ordinal", ordinal)
				_, err := repository.AppendAudit(ctx, authdomain.AuditEvent{Action: "login", Outcome: "completed", RequestId: fmt.Sprintf("%d-%d", worker, ordinal), OccurredAt: 100})
				if err != nil {
					failures <- err
					return
				}
			}
		})
	}
	workers.Wait()
	close(failures)
	for err := range failures {
		t.Error(err)
	}
	events, err := repository.ListAudit(ctx)
	if err != nil || len(events) != 200 {
		t.Fatalf("events=%d err=%v", len(events), err)
	}
	identities := map[string]bool{}
	for _, event := range events {
		identities[event.RequestId] = true
	}
	if len(identities) != 200 {
		t.Fatal("concurrent event identities were lost")
	}
}
