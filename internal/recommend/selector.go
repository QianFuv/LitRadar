package recommend

import (
	"context"
	"fmt"
	"log/slog"
	"sort"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

// SelectionRequest separates the bounded model batch from all candidates eligible for local supplementation.
type SelectionRequest struct {
	Subscriber         domain.Subscriber
	Global             GlobalConfig
	Defaults           Defaults
	OverrideModel      *string
	CandidatesForModel []storage.ArticleCandidate
	CandidatesById     map[int64]storage.ArticleCandidate
	Dedupe             map[string]string
}

func (request SelectionRequest) String() string       { return "SelectionRequest([REDACTED])" }
func (request SelectionRequest) GoString() string     { return request.String() }
func (request SelectionRequest) LogValue() slog.Value { return slog.StringValue(request.String()) }

// SelectionOutcome retains a successful selection or an explicit subscriber skip reason.
type SelectionOutcome struct {
	Accepted   []domain.RankedSelection `json:"accepted"`
	Summary    string                   `json:"summary"`
	SkipReason *string                  `json:"skip_reason"`
}

func (outcome SelectionOutcome) String() string       { return "SelectionOutcome([REDACTED])" }
func (outcome SelectionOutcome) GoString() string     { return outcome.String() }
func (outcome SelectionOutcome) LogValue() slog.Value { return slog.StringValue(outcome.String()) }

type selectionClient interface {
	SelectArticles(context.Context, AiRuntimeConfig, domain.Subscriber, Defaults, []storage.ArticleCandidate) (domain.SelectionResult, error)
	SummarizeSelectedArticles(context.Context, AiRuntimeConfig, domain.Subscriber, []storage.ArticleCandidate) (string, error)
}

// Selector coordinates five selection rounds and endpoint fallback under one durable job control.
type Selector struct {
	build         func(AiRuntimeConfig, int, float64) (selectionClient, error)
	retryAttempts int
	maxRounds     int
}

// NewSelector creates clients sharing the original deadline and request budget across all endpoints.
func NewSelector(repository *settings.Repository, timeout time.Duration, retryAttempts int, control *domain.ExecutionControl) *Selector {
	return &Selector{build: func(config AiRuntimeConfig, retries int, temperature float64) (selectionClient, error) {
		return NewAiClient(repository, retries, temperature, timeout, control), nil
	}, retryAttempts: retryAttempts, maxRounds: 5}
}

// SelectForSubscriber retries endpoint failures and returns an error if no endpoint succeeds.
// Successful empty selections, configuration skips and best-effort summaries remain distinct outcomes.
func (selector *Selector) SelectForSubscriber(ctx context.Context, request SelectionRequest) (SelectionOutcome, error) {
	if !HasSelectionPreferences(request.Subscriber) {
		return skippedSelection("No keywords or directions configured"), nil
	}
	configs := ResolveAiRuntimeConfigs(request.Subscriber, request.Global, request.Defaults, request.OverrideModel)
	if len(configs) == 0 {
		return skippedSelection("AI configuration is unavailable"), nil
	}
	retries := min(max(selector.retryAttempts, int(min(max(request.Subscriber.AiRetryAttempts, 0), 10))), 10)
	var lastError error
	for _, config := range configs {
		client, err := selector.build(config, retries, request.Defaults.Temperature)
		if err != nil {
			lastError = err
			continue
		}
		selection, err := selectRounds(ctx, client, config, request, max(selector.maxRounds, 1))
		if err != nil {
			lastError = err
			continue
		}
		accepted := ApplySelectionRules(selection, request.Subscriber, request.CandidatesById, request.Dedupe)
		summary := selection.Summary
		candidates := make([]storage.ArticleCandidate, 0, len(accepted))
		for _, item := range accepted {
			if candidate, exists := request.CandidatesById[item.ArticleId]; exists {
				candidates = append(candidates, candidate)
			}
		}
		if len(candidates) > 0 {
			if updated, err := client.SummarizeSelectedArticles(ctx, config, request.Subscriber, candidates); err == nil && strings.TrimSpace(updated) != "" {
				summary = updated
			}
		}
		return SelectionOutcome{Accepted: accepted, Summary: summary}, nil
	}
	return SelectionOutcome{}, fmt.Errorf("AI selection failed across configured endpoints: %w", lastError)
}

func skippedSelection(reason string) SelectionOutcome {
	return SelectionOutcome{Accepted: []domain.RankedSelection{}, SkipReason: &reason}
}

func selectRounds(ctx context.Context, client selectionClient, config AiRuntimeConfig, request SelectionRequest, rounds int) (domain.SelectionResult, error) {
	remaining := append([]storage.ArticleCandidate{}, request.CandidatesForModel...)
	aggregated := map[int64]domain.RankedSelection{}
	summary := ""
	for range rounds {
		if len(remaining) == 0 {
			break
		}
		result, err := client.SelectArticles(ctx, config, request.Subscriber, request.Defaults, remaining)
		if err != nil {
			return domain.SelectionResult{}, err
		}
		if summary == "" && strings.TrimSpace(result.Summary) != "" {
			summary = result.Summary
		}
		for _, selection := range result.Selections {
			if existing, exists := aggregated[selection.ArticleId]; !exists || !(existing.Score >= selection.Score) {
				aggregated[selection.ArticleId] = selection
			}
		}
		merged := mergedSelection(summary, aggregated)
		if len(ApplySelectionRules(merged, request.Subscriber, request.CandidatesById, request.Dedupe)) >= MaxArticlesPerPush {
			return merged, nil
		}
		next := remaining[:0]
		for _, candidate := range remaining {
			if _, exists := aggregated[candidate.ArticleId]; !exists {
				next = append(next, candidate)
			}
		}
		remaining = next
	}
	return mergedSelection(summary, aggregated), nil
}

func mergedSelection(summary string, aggregated map[int64]domain.RankedSelection) domain.SelectionResult {
	selections := make([]domain.RankedSelection, 0, len(aggregated))
	for _, selection := range aggregated {
		selections = append(selections, selection)
	}
	sort.Slice(selections, func(first, second int) bool { return selections[first].ArticleId < selections[second].ArticleId })
	sort.SliceStable(selections, func(first, second int) bool { return selections[first].Score > selections[second].Score })
	return domain.SelectionResult{Summary: summary, Selections: selections}
}
