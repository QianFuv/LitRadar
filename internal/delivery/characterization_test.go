package delivery

import (
	"context"
	"errors"
	"testing"

	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
)

func TestManualAggregationCountsDuplicatesAndEmptyMessageIds(t *testing.T) {
	outcomes := []RunOutcome{
		{DbName: "same.sqlite", Status: "failed", CandidateArticleIds: []int64{7, 7}, Subscribers: []SubscriberPlan{{SelectedArticleIds: []int64{7, 7}, FolderSyncedCount: 2, MessageId: pointer("")}}},
		{DbName: "same.sqlite", Status: "unknown", CandidateArticleIds: []int64{7}, Subscribers: []SubscriberPlan{{SelectedArticleIds: []int64{7}, FolderSyncedCount: 1}}},
	}
	result := manualOutcomeFromDelivery("pushplus", nil, outcomes)
	if result.Status != "unknown" || result.Selected != 3 || result.Pushed != 3 || result.TotalCandidates == nil || *result.TotalCandidates != 3 || result.Message != "same.sqlite delivery failed; same.sqlite delivery outcome is unknown" {
		t.Fatalf("duplicate aggregation changed: %+v", result)
	}
}

func TestManualAggregationCountsEmptyNonNilMessageId(t *testing.T) {
	result := manualOutcomeFromDelivery("pushplus", nil, []RunOutcome{{DbName: "same.sqlite", Status: "completed", Subscribers: []SubscriberPlan{{SelectedArticleIds: []int64{7}, MessageId: pointer("")}}}})
	if result.Message != "PushPlus sent successfully (1 message); selected 1 article across 1 database" {
		t.Fatal("empty non-nil message id stopped counting as a message")
	}
}

func TestDeliveryRenewRetainsUpdatedRunWhenLeaseRenewalFails(t *testing.T) {
	repository, _, config := durableFixture(t)
	ctx := context.Background()
	run, _, err := admitDurableRun(ctx, repository, config, nil, "partial-renew", 100)
	if err != nil {
		t.Fatal(err)
	}
	revision := run.run.Revision
	run.lease.Revision++
	if err := run.renew(ctx, repository, 101); !errors.Is(err, store.ErrConflict) {
		t.Fatalf("lease conflict: %v", err)
	}
	current, err := repository.LoadRun(ctx, run.run.Id)
	if err != nil || run.run.Revision != revision+1 || current.Revision != run.run.Revision {
		t.Fatal("successful run renewal was lost", err)
	}
}

func TestDeliverySameExpiredAttemptDoesNotSetCompetingTakeoverFlag(t *testing.T) {
	repository, _, config := durableFixture(t)
	ctx := context.Background()
	first, _, err := admitDurableRun(ctx, repository, config, nil, "same-attempt", 100)
	if err != nil {
		t.Fatal(err)
	}
	next, _, err := admitDurableRun(ctx, repository, config, nil, "same-attempt", 3700)
	if err != nil || next == nil || next.run.Id != first.run.Id || next.didTakeOverCompetingRun {
		t.Fatal("direct expired claim was classified as a competing takeover", err)
	}
}
