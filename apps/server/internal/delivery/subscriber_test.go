package delivery

import (
	"context"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/delivery"
	store "github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
)

func TestUnknownRecoveryUsesOnlyMatchingRunAndUserMessage(t *testing.T) {
	for _, scenario := range []struct {
		name                        string
		otherUser, oldRun, matching *string
	}{
		{"other user only", new("other-user"), nil, nil},
		{"old run only", nil, new("old-run"), nil},
		{"foreign and matching", new("other-user"), new("old-run"), new("matching")},
		{"later matching record", nil, nil, new("matching")},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			repository, database, config := durableFixture(t)
			ctx := context.Background()
			if _, err := database.Exec("INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(2,'second','hash','salt',1,1)"); err != nil {
				t.Fatal(err)
			}
			runs := make([]*store.RunRecord, 0, 2)
			for _, externalID := range []string{"old", "current"} {
				run, err := repository.EnqueueRun(ctx, store.RunCreate{ExternalID: externalID, Workflow: config.Workflow, ScopeKey: config.DBName, DBName: &config.DBName, TriggerKind: config.Trigger, Mode: config.Mode, CreatedAt: 100})
				if err != nil {
					t.Fatal(err)
				}
				runs = append(runs, run)
			}
			for _, record := range []struct {
				runID, userID, articleID int64
				status                   store.DedupeStatus
				message                  *string
			}{
				{runs[1].ID, 1, 6, store.DedupeStatusConfirmed, scenario.otherUser},
				{runs[0].ID, 2, 5, store.DedupeStatusConfirmed, scenario.oldRun},
				{runs[1].ID, 2, 7, store.DedupeStatusUnknown, nil},
				{runs[1].ID, 2, 8, store.DedupeStatusUnknown, scenario.matching},
			} {
				reserved, err := repository.ReserveDedupe(ctx, config.Workflow, config.DBName, record.userID, record.articleID, record.runID, "fixture", 101)
				if err != nil || reserved.Kind != "reserved" {
					t.Fatal("reservation", err)
				}
				if _, err := repository.ResolveDedupe(ctx, reserved.Record.ID, record.runID, "fixture", reserved.Record.Revision, record.status, record.message, 102); err != nil {
					t.Fatal(err)
				}
			}
			before, err := repository.ListDedupe(ctx, config.Workflow, config.DBName)
			if err != nil {
				t.Fatal(err)
			}
			engine := &deliveryEngine{repository: repository}
			plan, err := engine.terminalPlan(ctx, config, domain.Subscriber{SubscriberID: "2", UserID: 2, DeliveryMethod: "pushplus"}, store.RunItemRecord{DeliveryRunID: runs[1].ID, UserID: new(int64(2)), Status: store.ItemStatusUnknown})
			if err != nil || plan.Status != "unknown" || plan.WouldSendPushplus || !reflect.DeepEqual(plan.SelectedArticleIds, []int64{7, 8}) || !reflect.DeepEqual(plan.MessageID, scenario.matching) {
				t.Fatalf("incorrect unknown attribution: %+v error=%v", plan, err)
			}
			after, err := repository.ListDedupe(ctx, config.Workflow, config.DBName)
			if err != nil || !reflect.DeepEqual(before, after) {
				t.Fatal("recovery changed durable delivery protection", err)
			}
		})
	}
}
