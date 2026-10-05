package delivery

import (
	"context"
	"reflect"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
)

func TestUnknownRecoveryUsesOnlyMatchingRunAndUserMessage(t *testing.T) {
	for _, scenario := range []struct {
		name                        string
		otherUser, oldRun, matching *string
	}{
		{"other user only", pointer("other-user"), nil, nil},
		{"old run only", nil, pointer("old-run"), nil},
		{"foreign and matching", pointer("other-user"), pointer("old-run"), pointer("matching")},
		{"later matching record", nil, nil, pointer("matching")},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			repository, database, config := durableFixture(t)
			ctx := context.Background()
			if _, err := database.Exec("INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(2,'second','hash','salt',1,1)"); err != nil {
				t.Fatal(err)
			}
			runs := make([]*store.RunRecord, 0, 2)
			for _, externalId := range []string{"old", "current"} {
				run, err := repository.EnqueueRun(ctx, store.RunCreate{ExternalId: externalId, Workflow: config.Workflow, ScopeKey: config.DbName, DbName: &config.DbName, TriggerKind: config.Trigger, Mode: config.Mode, CreatedAt: 100})
				if err != nil {
					t.Fatal(err)
				}
				runs = append(runs, run)
			}
			for _, record := range []struct {
				runId, userId, articleId int64
				status                   store.DedupeStatus
				message                  *string
			}{
				{runs[1].Id, 1, 6, store.DedupeStatusConfirmed, scenario.otherUser},
				{runs[0].Id, 2, 5, store.DedupeStatusConfirmed, scenario.oldRun},
				{runs[1].Id, 2, 7, store.DedupeStatusUnknown, nil},
				{runs[1].Id, 2, 8, store.DedupeStatusUnknown, scenario.matching},
			} {
				reserved, err := repository.ReserveDedupe(ctx, config.Workflow, config.DbName, record.userId, record.articleId, record.runId, "fixture", 101)
				if err != nil || reserved.Kind != "reserved" {
					t.Fatal("reservation", err)
				}
				if _, err := repository.ResolveDedupe(ctx, reserved.Record.Id, record.runId, "fixture", reserved.Record.Revision, record.status, record.message, 102); err != nil {
					t.Fatal(err)
				}
			}
			before, err := repository.ListDedupe(ctx, config.Workflow, config.DbName)
			if err != nil {
				t.Fatal(err)
			}
			engine := &deliveryEngine{repository: repository}
			plan, err := engine.terminalPlan(ctx, config, domain.Subscriber{SubscriberId: "2", UserId: 2, DeliveryMethod: "pushplus"}, store.RunItemRecord{DeliveryRunId: runs[1].Id, UserId: pointer(int64(2)), Status: store.ItemStatusUnknown})
			if err != nil || plan.Status != "unknown" || plan.WouldSendPushplus || !reflect.DeepEqual(plan.SelectedArticleIds, []int64{7, 8}) || !reflect.DeepEqual(plan.MessageId, scenario.matching) {
				t.Fatalf("incorrect unknown attribution: %+v error=%v", plan, err)
			}
			after, err := repository.ListDedupe(ctx, config.Workflow, config.DbName)
			if err != nil || !reflect.DeepEqual(before, after) {
				t.Fatal("recovery changed durable delivery protection", err)
			}
		})
	}
}
