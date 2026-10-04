package recommend

import (
	"context"
	"errors"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
)

type scriptedSelection struct {
	selections   []domain.SelectionResult
	errorAt      int
	batches      [][]int64
	summary      string
	summaryError error
	summarized   []int64
}

func (client *scriptedSelection) SelectArticles(ctx context.Context, config AiRuntimeConfig, subscriber domain.Subscriber, defaults Defaults, candidates []storage.ArticleCandidate) (domain.SelectionResult, error) {
	ids := []int64{}
	for _, candidate := range candidates {
		ids = append(ids, candidate.ArticleId)
	}
	client.batches = append(client.batches, ids)
	if client.errorAt == len(client.batches) {
		return domain.SelectionResult{}, errors.New("endpoint failed")
	}
	if len(client.selections) == 0 {
		return domain.SelectionResult{Selections: []domain.RankedSelection{}}, nil
	}
	result := client.selections[0]
	client.selections = client.selections[1:]
	return result, nil
}

func (client *scriptedSelection) SummarizeSelectedArticles(ctx context.Context, config AiRuntimeConfig, subscriber domain.Subscriber, candidates []storage.ArticleCandidate) (string, error) {
	for _, candidate := range candidates {
		client.summarized = append(client.summarized, candidate.ArticleId)
	}
	return client.summary, client.summaryError
}

func selectorFixture(clients ...*scriptedSelection) (*Selector, SelectionRequest, *[]int) {
	backup := "https://backup.test/v1/"
	request := SelectionRequest{Subscriber: domain.Subscriber{SubscriberId: "1", Keywords: []string{"match"}, AiBackupBaseUrl: &backup}, Global: GlobalConfig{AiBaseUrl: "https://primary.test/v1/", AiAllowedBaseUrls: []string{"https://primary.test/v1/", backup}, AiApiKey: "key"}, Defaults: Defaults{AiModel: "model", Temperature: 0.2}, Dedupe: map[string]string{}}
	for _, id := range []int64{1, 2, 3} {
		request.CandidatesForModel = append(request.CandidatesForModel, storage.ArticleCandidate{ArticleId: id, Title: "match"})
	}
	request.CandidatesById = CandidatesById(request.CandidatesForModel)
	builds := []int{}
	selector := &Selector{maxRounds: 5, build: func(config AiRuntimeConfig, retries int, temperature float64) (selectionClient, error) {
		builds = append(builds, retries)
		if len(clients) == 0 {
			return nil, errors.New("unexpected endpoint")
		}
		client := clients[0]
		clients = clients[1:]
		return client, nil
	}}
	return selector, request, &builds
}

func TestSelectorRemovesModelIdsAndUsesFirstSummaryOnSummaryFailure(t *testing.T) {
	client := &scriptedSelection{selections: []domain.SelectionResult{{Summary: " first ", Selections: []domain.RankedSelection{{ArticleId: 1, Score: 9}}}, {Summary: "second", Selections: []domain.RankedSelection{{ArticleId: 2, Score: 8}}}, {Selections: []domain.RankedSelection{{ArticleId: 3, Score: 7}}}}, summaryError: errors.New("summary failed")}
	selector, request, _ := selectorFixture(client)
	request.Dedupe["1:1"] = "delivered"
	result := selector.SelectForSubscriber(context.Background(), request)
	if !reflect.DeepEqual(client.batches, [][]int64{{1, 2, 3}, {2, 3}, {3}}) || result.Summary != " first " || result.SkipReason != nil || !reflect.DeepEqual(client.summarized, []int64{2, 3}) {
		t.Fatalf("round/summary contract: batches=%v outcome=%+v summarized=%v", client.batches, result, client.summarized)
	}
}

func TestSelectorBackupStartsFreshAfterPartialPrimaryFailure(t *testing.T) {
	primary := &scriptedSelection{selections: []domain.SelectionResult{{Summary: "partial", Selections: []domain.RankedSelection{{ArticleId: 1, Score: 999}}}}, errorAt: 2}
	backup := &scriptedSelection{selections: []domain.SelectionResult{{Summary: "backup", Selections: []domain.RankedSelection{{ArticleId: 2, Score: 9}, {ArticleId: 3, Score: 8}, {ArticleId: 1, Score: 1}}}}, summary: " final "}
	selector, request, builds := selectorFixture(primary, backup)
	request.Subscriber.AiRetryAttempts = 999
	result := selector.SelectForSubscriber(context.Background(), request)
	if !reflect.DeepEqual(*builds, []int{10, 10}) || !reflect.DeepEqual(backup.batches, [][]int64{{1, 2, 3}}) || result.Summary != " final " || len(result.Accepted) != 3 || result.Accepted[0].ArticleId != 2 {
		t.Fatalf("backup retained primary state: %v %v %+v", *builds, backup.batches, result)
	}
}

func TestSelectorSuccessfulEmptyResultDoesNotFallback(t *testing.T) {
	primary := &scriptedSelection{}
	selector, request, builds := selectorFixture(primary)
	for id, candidate := range request.CandidatesById {
		candidate.Title = "unrelated"
		request.CandidatesById[id] = candidate
	}
	result := selector.SelectForSubscriber(context.Background(), request)
	if len(*builds) != 1 || len(primary.batches) != 5 || len(result.Accepted) != 0 || result.SkipReason != nil {
		t.Fatalf("empty selection treated as failure: builds=%v rounds=%d %+v", *builds, len(primary.batches), result)
	}
}

func TestSelectorSupplementCanFinishFromCandidatesOutsideModelBatch(t *testing.T) {
	primary := &scriptedSelection{summary: "complete"}
	selector, request, _ := selectorFixture(primary)
	request.CandidatesForModel = request.CandidatesForModel[:1]
	for id := int64(4); id <= 25; id++ {
		request.CandidatesById[id] = storage.ArticleCandidate{ArticleId: id, Title: "match"}
	}
	result := selector.SelectForSubscriber(context.Background(), request)
	if len(primary.batches) != 1 || len(result.Accepted) != 20 || len(primary.summarized) != 20 || result.Accepted[0].ArticleId != 25 {
		t.Fatalf("full candidate supplementation lost: rounds=%d count=%d", len(primary.batches), len(result.Accepted))
	}
}

func TestSelectorSkipsWithoutPreferencesOrApprovedConfiguration(t *testing.T) {
	selector, request, builds := selectorFixture()
	request.Subscriber.Keywords = nil
	result := selector.SelectForSubscriber(context.Background(), request)
	if result.SkipReason == nil || *result.SkipReason != "No keywords or directions configured" {
		t.Fatal(result)
	}
	request.Subscriber.Keywords = []string{"match"}
	request.Global.AiAllowedBaseUrls = nil
	result = selector.SelectForSubscriber(context.Background(), request)
	if result.SkipReason == nil || *result.SkipReason != "AI configuration is unavailable" || len(*builds) != 0 {
		t.Fatal(result)
	}
}
