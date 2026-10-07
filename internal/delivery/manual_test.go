package delivery

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/recommend"
	storageconfig "github.com/QianFuv/LitRadar/internal/storage/config"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

func manualWeeklyFixture(t *testing.T, names []string) ManualWeeklyPushConfig {
	t.Helper()
	_, _, config := workflowFixture(t, store.WorkflowPush)
	storage := storageconfig.FromProjectRoot(t.TempDir()).WithAuthDbPath(config.AuthDbPath)
	if err := os.MkdirAll(storage.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	directory := filepath.Join(storage.ProjectRoot, "data", "push_state")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	for _, name := range names {
		index, err := platform.Open(platform.Config{Filename: filepath.Join(storage.IndexDir, name+".sqlite"), Mode: "rwc", MaxConnections: 1})
		if err != nil {
			t.Fatal(err)
		}
		_, err = index.Exec(`CREATE TABLE journals(journal_id INTEGER PRIMARY KEY); CREATE TABLE article_listing(article_id INTEGER PRIMARY KEY,journal_id INTEGER); INSERT INTO journals VALUES(1); INSERT INTO article_listing VALUES(7,1),(8,1);`)
		index.Close()
		if err != nil {
			t.Fatal(err)
		}
		body := `{"db_name":"` + name + `.sqlite","run_id":" source-` + name + ` ","generated_at":"2026-10-04T12:00:00.250Z","notifiable_article_ids":[7,7,999],"backfill_article_ids":[8]}`
		if err = os.WriteFile(filepath.Join(directory, name+".changes.json"), []byte(body), 0600); err != nil {
			t.Fatal(err)
		}
	}
	stamp, ok := weekly.ParseTimestamp("2026-10-04T12:00:00.250Z")
	if !ok {
		t.Fatal("invalid fixture time")
	}
	return ManualWeeklyPushConfig{WindowEnd: stamp, StorageConfig: storage, SecretCodec: config.SecretCodec, UserId: 1, AttemptId: "parent-attempt", TimeoutSeconds: 120, RetryAttempts: 3, DedupeRetentionDays: 60}
}

func TestManualWeeklyUsesFixedMembershipAndStableAttempt(t *testing.T) {
	config := manualWeeklyFixture(t, []string{"first", "second"})
	seen := []string{}
	execute := func(ctx context.Context, child RunConfig, user *int64, manifest *recommend.ChangeManifest) (RunOutcome, error) {
		assertManualSourceContext(t, child, user, manifest)
		seen = append(seen, child.DbName)
		return runOutcome(child, 1, "completed", []int64{7}, []SubscriberPlan{{SelectedArticleIds: []int64{7}, FolderSyncedCount: 1}}), nil
	}
	outcome, err := runManualWeeklyPush(context.Background(), config, execute)
	if err != nil || outcome.Selected != 2 || outcome.Pushed != 2 || outcome.TotalCandidates == nil || *outcome.TotalCandidates != 2 || len(seen) != 2 {
		t.Fatal("manual aggregate", err)
	}
	assertManualCapturedWindow(t, config, &seen, execute)

}

func TestManualWeeklySharesBudgetAndStopsAfterFailure(t *testing.T) {
	config := manualWeeklyFixture(t, []string{"first", "second", "third"})
	config.ExecutionControl = domain.NewExecutionControl(unixNow()+60, 8, func() (bool, error) { return false, nil })
	calls := 0
	_, err := runManualWeeklyPush(context.Background(), config, func(ctx context.Context, child RunConfig, _ *int64, _ *recommend.ChangeManifest) (RunOutcome, error) {
		calls++
		if child.ExecutionControl != config.ExecutionControl {
			t.Fatal("child received a separate budget")
		}
		for range 5 {
			if _, err := child.ExecutionControl.BeginAiRequest(time.Second); err != nil {
				return RunOutcome{}, err
			}
		}
		return runOutcome(child, 1, "completed", nil, nil), nil
	})
	if !errors.Is(err, domain.ControlBudgetExhausted) || calls != 2 {
		t.Fatal("manual budget restarted or later database ran", calls, err)
	}
}

func TestManualAggregationPreservesUnknownPriorityAndPublicMessages(t *testing.T) {
	folder := &favorites.Folder{Id: 4, Name: "tracking"}
	outcomes := []RunOutcome{{DbName: "first.sqlite", Status: "completed", CandidateArticleIds: []int64{7}, Subscribers: []SubscriberPlan{{SelectedArticleIds: []int64{7}, FolderSyncedCount: 1, MessageId: pointer("message")}}}}
	result := manualOutcomeFromDelivery("pushplus", folder, outcomes)
	if result.Message != "PushPlus sent successfully (1 message); selected 1 article across 1 database; synced 1 article to the tracking folder" || result.FolderId == nil || *result.FolderId != 4 {
		t.Fatal("public message changed")
	}
	outcomes = append(outcomes, RunOutcome{DbName: "second.sqlite", Status: "failed"}, RunOutcome{DbName: "third.sqlite", Status: "unknown"})
	result = manualOutcomeFromDelivery("pushplus", folder, outcomes)
	if result.Status != "unknown" || result.Message != "second.sqlite delivery failed; third.sqlite delivery outcome is unknown" || result.Selected != 1 || result.Pushed != 1 {
		t.Fatal("unknown aggregation changed")
	}
	assertManualSkipSummary(t)

}

func assertManualSourceContext(t *testing.T, child RunConfig, user *int64, manifest *recommend.ChangeManifest) {
	t.Helper()
	if user == nil || *user != 1 || child.Trigger != store.TriggerKindScheduled || child.Mode != store.RunModeExecute || child.AttemptId == nil || *child.AttemptId != "parent-attempt" || !reflect.DeepEqual(manifest.PendingArticleIds, []int64{7}) || manifest.RunId == nil || *manifest.RunId != "source-"+child.DbName[:len(child.DbName)-7] {
		t.Fatal("manual source/context changed")
	}
}

func assertManualSkipSummary(t *testing.T) {
	t.Helper()
	result := manualOutcomeFromDelivery("folder", nil, []RunOutcome{{Subscribers: []SubscriberPlan{{Error: pointer("first skip")}, {Error: pointer("second skip")}}}})
	if result.Message != "first skip" || result.TotalCandidates == nil || *result.TotalCandidates != 0 {
		t.Fatal("skip summary changed")
	}
}

func assertManualCapturedWindow(t *testing.T, config ManualWeeklyPushConfig, seen *[]string, execute func(context.Context, RunConfig, *int64, *recommend.ChangeManifest) (RunOutcome, error)) {
	t.Helper()
	*seen = nil
	config.WindowEnd.Nanoseconds--
	outcome, err := runManualWeeklyPush(context.Background(), config, execute)
	if err != nil || outcome.Message != "No new weekly articles available" || outcome.TotalCandidates != nil || len(*seen) != 0 {
		t.Fatal("publication beyond captured window included", err)
	}
}
