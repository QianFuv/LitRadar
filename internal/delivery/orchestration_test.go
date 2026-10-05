package delivery

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/recommend"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func workflowFixture(t *testing.T, workflow store.Workflow) (*deliveryEngine, *sql.DB, RunConfig) {
	t.Helper()
	repository, database, config := durableFixture(t)
	codec, err := secrets.NewCodec(make([]byte, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { codec.Close() })
	config.SecretCodec = codec
	token, err := codec.Encrypt("synthetic-token", secrets.NotificationContext(1, "pushplus_token"))
	if err != nil {
		t.Fatal(err)
	}
	method := "pushplus"
	if workflow == store.WorkflowPush {
		method = "folder"
	}
	config.Workflow = workflow
	if _, err = database.Exec(`INSERT INTO folders(id,user_id,name,is_tracking,created_at,updated_at) VALUES(1,1,'tracking',1,1,1); INSERT INTO notification_settings(user_id,keywords,delivery_method,pushplus_token,sync_to_tracking_folder,created_at,updated_at) VALUES(1,'["science"]',?,?,1,1,1)`, method, token); err != nil {
		t.Fatal(err)
	}
	config.IndexDbPath = filepath.Join(t.TempDir(), "index.sqlite")
	index, err := platform.Open(platform.Config{Filename: config.IndexDbPath, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	_, err = index.Exec(`CREATE TABLE journals(journal_id INTEGER PRIMARY KEY,title TEXT); CREATE TABLE articles(article_id INTEGER PRIMARY KEY,journal_id INTEGER,issue_id INTEGER,title TEXT,abstract_text TEXT,date TEXT,open_access INTEGER,in_press INTEGER,doi TEXT); INSERT INTO journals VALUES(1,'Journal'); INSERT INTO articles VALUES(7,1,2,'science title','abstract','2026-10-01',1,0,NULL),(8,1,2,'science second','abstract','2026-10-02',0,0,NULL);`)
	index.Close()
	if err != nil {
		t.Fatal(err)
	}
	accounts, err := auth.Open(config.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { accounts.Close() })
	folders := favorites.New(accounts)
	engine := &deliveryEngine{repository: repository, settings: settings.New(accounts, codec), selectArticles: func(_ context.Context, request recommend.SelectionRequest) (recommend.SelectionOutcome, error) {
		accepted := []domain.RankedSelection{}
		for _, candidate := range request.CandidatesForModel {
			if _, exists := request.Dedupe[recommend.DeliveryKey(request.Subscriber, candidate.ArticleId)]; !exists {
				accepted = append(accepted, domain.RankedSelection{ArticleId: candidate.ArticleId, Score: 1})
			}
		}
		return recommend.SelectionOutcome{Accepted: accepted}, nil
	}, send: func(context.Context, PushplusMessage) (string, error) { return "synthetic-message", nil }, writeFavorites: func(ctx context.Context, writes []FavoriteWritePlan) error {
		return executeFavoriteWrites(ctx, folders, writes)
	}}
	return engine, database, config
}

func manifestFor(label string, ids ...int64) *recommend.ChangeManifest {
	return &recommend.ChangeManifest{RunId: &label, PendingArticleIds: ids}
}
func countRows(t *testing.T, database *sql.DB, table string) int {
	t.Helper()
	var count int
	if err := database.QueryRow("SELECT COUNT(*) FROM " + table).Scan(&count); err != nil {
		t.Fatal(err)
	}
	return count
}

func TestWorkflowDryRunPersistsProgressWithoutFavoritesDedupeOrSend(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowNotify)
	config.Mode = store.RunModeDryRun
	engine.send = func(context.Context, PushplusMessage) (string, error) {
		t.Fatal("dry run sent a notification")
		return "", nil
	}
	engine.writeFavorites = func(context.Context, []FavoriteWritePlan) error { t.Fatal("dry run wrote favorites"); return nil }
	outcome, err := engine.execute(context.Background(), config, nil, manifestFor("dry", 7))
	if err != nil || outcome.Status != "completed" || len(outcome.Subscribers) != 1 || outcome.Subscribers[0].FolderSyncedCount != 1 || !outcome.Subscribers[0].WouldSendPushplus {
		t.Fatalf("dry run: %+v %v", outcome, err)
	}
	if countRows(t, database, "favorites") != 0 || countRows(t, database, "delivery_dedupe") != 0 || countRows(t, database, "delivery_run_items") != 2 || countRows(t, database, "delivery_checkpoints") != 1 {
		t.Fatal("dry run durable/effect boundary changed")
	}
}

func TestWorkflowTerminalIdentityAndDedupePreventRepeatedEffects(t *testing.T) {
	for _, workflow := range []store.Workflow{store.WorkflowNotify, store.WorkflowPush} {
		t.Run(string(workflow), func(t *testing.T) {
			engine, database, config := workflowFixture(t, workflow)
			sent := 0
			engine.send = func(context.Context, PushplusMessage) (string, error) { sent++; return "message", nil }
			first, err := engine.execute(context.Background(), config, nil, manifestFor("first", 7))
			if err != nil || first.Status != "completed" {
				t.Fatal(err)
			}
			replay, err := engine.execute(context.Background(), config, nil, manifestFor("first", 7, 8))
			if err != nil || replay.DeliveryRunId != first.DeliveryRunId || len(replay.Subscribers) != 0 {
				t.Fatal("terminal rerun", err)
			}
			next, err := engine.execute(context.Background(), config, nil, manifestFor("next", 7))
			if err != nil || next.Subscribers[0].Status != "skipped" {
				t.Fatal("dedupe rerun", err)
			}
			wantSent := 0
			if workflow == store.WorkflowNotify {
				wantSent = 1
				if first.Subscribers[0].MessageId == nil || *first.Subscribers[0].MessageId != "message" {
					t.Fatal("successful message ID lost")
				}
			}
			if sent != wantSent || countRows(t, database, "favorites") != 1 || countRows(t, database, "delivery_dedupe") != 1 {
				t.Fatal("repeated side effect")
			}
		})
	}
}

func TestWorkflowAmbiguousSendQuarantinesAllReservationsAndPreservesCheckpoint(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowNotify)
	sent := 0
	engine.send = func(context.Context, PushplusMessage) (string, error) {
		sent++
		return "", errors.New("synthetic timeout")
	}
	outcome, err := engine.execute(context.Background(), config, nil, manifestFor("ambiguous", 7))
	if err != nil || outcome.Status != "unknown" || outcome.Subscribers[0].Status != "unknown" {
		t.Fatalf("ambiguous: %v %v", outcome, err)
	}
	checkpoint, err := engine.repository.LoadCheckpoint(context.Background(), config.Workflow, config.DbName)
	if err != nil || checkpoint.Status != store.CheckpointStatusUnknown || checkpoint.SnapshotJson != `{"issue_article_counts":{},"inpress_article_counts":{}}` {
		t.Fatal("ambiguous checkpoint advanced", err)
	}
	dedupe, err := engine.repository.LoadDedupe(context.Background(), config.Workflow, config.DbName, 1, 7)
	if err != nil || dedupe.Status != store.DedupeStatusUnknown {
		t.Fatal("reservation not quarantined", err)
	}
	_, err = engine.execute(context.Background(), config, nil, manifestFor("later", 7))
	if err != nil || sent != 1 || countRows(t, database, "favorites") != 1 {
		t.Fatal("unknown send retried", err)
	}
}

func TestWorkflowReservationConflictReleasesOnlyNewReservations(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowPush)
	engine.selectArticles = func(ctx context.Context, request recommend.SelectionRequest) (recommend.SelectionOutcome, error) {
		return recommend.SelectionOutcome{Accepted: []domain.RankedSelection{{ArticleId: 7}, {ArticleId: 8}, {ArticleId: 7}}}, nil
	}
	outcome, err := engine.execute(context.Background(), config, nil, manifestFor("duplicates", 7, 8))
	if err != nil || outcome.Subscribers[0].Status != "skipped" || countRows(t, database, "delivery_dedupe") != 0 || countRows(t, database, "favorites") != 0 {
		t.Fatal("partial reservations survived conflicting selection", err)
	}
}

func TestWorkflowFavoriteFailureReleasesReservationsWithoutSending(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowNotify)
	engine.writeFavorites = func(context.Context, []FavoriteWritePlan) error { return errors.New("synthetic folder failure") }
	engine.send = func(context.Context, PushplusMessage) (string, error) {
		t.Fatal("sent after failed favorite write")
		return "", nil
	}
	outcome, err := engine.execute(context.Background(), config, nil, manifestFor("favorite-failure", 7))
	if err != nil || outcome.Status != "failed" || outcome.Subscribers[0].FolderSyncedCount != 1 || countRows(t, database, "delivery_dedupe") != 0 {
		t.Fatal("favorite failure", err)
	}
}

func TestWorkflowRecoversSendingItemFromDurableInputsWithoutCallingProviders(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowNotify)
	ctx := context.Background()
	run, _, err := admitDurableRun(ctx, engine.repository, config, nil, "old", 100)
	if err != nil {
		t.Fatal(err)
	}
	items, err := engine.repository.EnsureRunItems(ctx, run.run.Id, []store.RunItemCreate{{ItemKind: store.ItemKindArticle, ItemKey: "7", ArticleId: pointer(int64(7))}, {ItemKind: store.ItemKindSubscriber, ItemKey: "1", UserId: pointer(int64(1))}}, 100)
	if err != nil {
		t.Fatal(err)
	}
	item, err := engine.repository.ClaimItem(ctx, run.run.Id, run.owner, run.run.Revision, items[1].Id, run.owner, 100, 3600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = engine.repository.ReserveDedupe(ctx, config.Workflow, config.DbName, 1, 7, run.run.Id, run.owner, 100); err != nil {
		t.Fatal(err)
	}
	if _, err = engine.repository.MarkItemSending(ctx, item.Id, run.owner, item.Revision, 100); err != nil {
		t.Fatal(err)
	}
	engine.selectArticles = func(context.Context, recommend.SelectionRequest) (recommend.SelectionOutcome, error) {
		t.Fatal("reselected ambiguous subscriber")
		return recommend.SelectionOutcome{}, nil
	}
	engine.send = func(context.Context, PushplusMessage) (string, error) {
		t.Fatal("resent ambiguous item")
		return "", nil
	}
	outcome, err := engine.execute(ctx, config, nil, manifestFor("new", 8))
	if err != nil || outcome.DeliveryRunId != run.run.Id || outcome.Status != "unknown" || !reflect.DeepEqual(outcome.CandidateArticleIds, []int64{7}) || !reflect.DeepEqual(outcome.Subscribers[0].SelectedArticleIds, []int64{7}) || outcome.Subscribers[0].WouldSendPushplus || countRows(t, database, "favorites") != 0 {
		t.Fatalf("recovery: %v %v", outcome, err)
	}
}

func TestWorkflowPostSendCommitFailureCannotCauseReplay(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowNotify)
	sent := 0
	engine.send = func(context.Context, PushplusMessage) (string, error) {
		sent++
		if _, err := database.Exec(`CREATE TRIGGER reject_success BEFORE UPDATE OF status ON delivery_run_items WHEN NEW.status='succeeded' AND NEW.item_kind='subscriber' BEGIN SELECT RAISE(ABORT,'synthetic finalizer failure'); END;`); err != nil {
			t.Fatal(err)
		}
		return "delivered", nil
	}
	if _, err := engine.execute(context.Background(), config, nil, manifestFor("commit-failure", 7)); err == nil {
		t.Fatal("finalizer failure hidden")
	}
	records, err := engine.repository.ListDedupe(context.Background(), config.Workflow, config.DbName)
	if err != nil || len(records) != 1 || records[0].Status != store.DedupeStatusUnknown {
		t.Fatal("sent but uncommitted was not quarantined", err)
	}
	if _, err = database.Exec("DROP TRIGGER reject_success"); err != nil {
		t.Fatal(err)
	}
	if _, err = engine.execute(context.Background(), config, nil, manifestFor("retry", 7)); err != nil || sent != 1 {
		t.Fatal("resent committed external effect", err)
	}
}

func TestWorkflowMissingSubscriberDoesNotConsumeSnapshot(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowPush)
	if _, err := database.Exec("UPDATE notification_settings SET enabled=0"); err != nil {
		t.Fatal(err)
	}
	outcome, err := engine.execute(context.Background(), config, nil, nil)
	if err != nil || outcome.Status != "skipped" {
		t.Fatal(err)
	}
	checkpoint, err := engine.repository.LoadCheckpoint(context.Background(), config.Workflow, config.DbName)
	if err != nil || checkpoint.SnapshotJson != `{"issue_article_counts":{},"inpress_article_counts":{}}` {
		t.Fatal("snapshot consumed without subscribers", err)
	}
	if _, err = database.Exec("UPDATE notification_settings SET enabled=1"); err != nil {
		t.Fatal(err)
	}
	outcome, err = engine.execute(context.Background(), config, nil, nil)
	if err != nil || outcome.Status != "completed" || len(outcome.CandidateArticleIds) != 2 {
		t.Fatal("missed previously skipped additions", err)
	}
	outcome, err = engine.execute(context.Background(), config, nil, nil)
	if err != nil || outcome.Status != "idle" {
		t.Fatal("unchanged snapshot not idle", err)
	}
}

func TestWorkflowSelectionFailurePreservesProgressAndRetriesWithoutReplay(t *testing.T) {
	engine, database, config := workflowFixture(t, store.WorkflowNotify)
	ctx := context.Background()
	sent := 0
	engine.send = func(context.Context, PushplusMessage) (string, error) {
		sent++
		return "confirmed-message", nil
	}
	first, err := engine.execute(ctx, config, nil, nil)
	if err != nil || first.Status != "completed" || sent != 1 {
		t.Fatalf("initial delivery: %+v %v", first, err)
	}
	previous, err := engine.repository.LoadCheckpoint(ctx, config.Workflow, config.DbName)
	if err != nil {
		t.Fatal(err)
	}
	index, err := platform.Open(platform.Config{Filename: config.IndexDbPath, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	_, err = index.Exec(`INSERT INTO articles VALUES(9,1,2,'science third','abstract','2026-10-03',0,0,NULL)`)
	index.Close()
	if err != nil {
		t.Fatal(err)
	}
	selectArticles := engine.selectArticles
	engine.selectArticles = func(context.Context, recommend.SelectionRequest) (recommend.SelectionOutcome, error) {
		return recommend.SelectionOutcome{}, errors.New("all endpoints failed")
	}
	failed, err := engine.execute(ctx, config, nil, nil)
	if err != nil || failed.Status != "failed" || len(failed.Subscribers) != 1 || failed.Subscribers[0].Status != "error" {
		t.Fatalf("selection failure: %+v %v", failed, err)
	}
	checkpoint, err := engine.repository.LoadCheckpoint(ctx, config.Workflow, config.DbName)
	if err != nil || checkpoint.Status != store.CheckpointStatusFailed || checkpoint.SnapshotJson != previous.SnapshotJson || !reflect.DeepEqual(checkpoint.LastCompletedRunAt, previous.LastCompletedRunAt) {
		t.Fatal("failed selection advanced progress", err)
	}
	items, err := engine.repository.ListRunItems(ctx, failed.DeliveryRunId)
	if err != nil {
		t.Fatal(err)
	}
	hasFailedSubscriber := false
	for _, item := range items {
		if item.ItemKind == store.ItemKindSubscriber {
			hasFailedSubscriber = item.Status == store.ItemStatusFailed && item.ErrorCode != nil && *item.ErrorCode == "selection_failed"
		}
	}
	if !hasFailedSubscriber || sent != 1 || countRows(t, database, "favorites") != 2 || countRows(t, database, "delivery_dedupe") != 2 {
		t.Fatal("failed selection changed durable effects or lost failure classification")
	}
	engine.selectArticles = selectArticles
	retried, err := engine.execute(ctx, config, nil, nil)
	if err != nil || retried.Status != "completed" || len(retried.Subscribers) != 1 || !reflect.DeepEqual(retried.Subscribers[0].SelectedArticleIds, []int64{9}) || sent != 2 || countRows(t, database, "favorites") != 3 || countRows(t, database, "delivery_dedupe") != 3 {
		t.Fatalf("retry lost pending article or replayed confirmed delivery: %+v %v", retried, err)
	}
	idle, err := engine.execute(ctx, config, nil, nil)
	if err != nil || idle.Status != "idle" || sent != 2 {
		t.Fatal("successful retry did not advance progress", err)
	}
}

func TestWorkflowMalformedManifestPrecedesFilesystemAdmission(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "invalid.json")
	if err := os.WriteFile(filename, []byte("{"), 0600); err != nil {
		t.Fatal(err)
	}
	authDirectory := filepath.Join(root, "missing")
	_, err := RunRecommendationDelivery(context.Background(), RunConfig{ChangesFile: &filename, AuthDbPath: filepath.Join(authDirectory, "auth.sqlite"), DbName: "fixture.sqlite"})
	if !errors.Is(err, recommend.ErrManifestJson) {
		t.Fatal("manifest error was displaced", err)
	}
	if _, err := os.Stat(authDirectory); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("invalid manifest touched storage", err)
	}
}

func TestWorkflowFailureLogsExcludeProviderAndSubscriberPayloads(t *testing.T) {
	engine, _, config := workflowFixture(t, store.WorkflowNotify)
	var output bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&output, nil)))
	defer slog.SetDefault(previous)
	engine.send = func(context.Context, PushplusMessage) (string, error) {
		return "", errors.New("private-provider-response-marker")
	}
	outcome, err := engine.execute(context.Background(), config, nil, manifestFor("private-run-marker", 7))
	if err != nil || outcome.Status != "unknown" {
		t.Fatal(err)
	}
	for _, secret := range []string{"private-provider-response-marker", "private-run-marker", "synthetic-token", "science title", "abstract", config.AuthDbPath} {
		if strings.Contains(output.String(), secret) {
			t.Fatal("workflow log exposed private input")
		}
	}
	if !strings.Contains(output.String(), `"event":"delivery.workflow.failed"`) || !strings.Contains(output.String(), `"status":"unknown"`) || !strings.Contains(output.String(), `"folder_synced_count":1`) {
		t.Fatal("failure classification/counters missing")
	}
}

func TestWorkflowEmptyUpstreamMessageIdRetainsOriginalStorageRejection(t *testing.T) {
	engine, _, config := workflowFixture(t, store.WorkflowNotify)
	engine.send = func(context.Context, PushplusMessage) (string, error) { return "", nil }
	if _, err := engine.execute(context.Background(), config, nil, manifestFor("empty-message", 7)); err == nil || err.Error() != "Delivery message id is invalid" {
		t.Fatal("empty message ID finalization changed", err)
	}
	record, err := engine.repository.LoadDedupe(context.Background(), config.Workflow, config.DbName, 1, 7)
	if err != nil || record.Status != store.DedupeStatusUnknown {
		t.Fatal("uncommitted successful send was not quarantined", err)
	}
}
