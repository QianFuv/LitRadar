package scheduler

import (
	"context"
	"errors"
	"reflect"
	"sync"
	"testing"

	auditdomain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
)

func taskInput() domain.Create {
	return domain.Create{Name: "fixture", Job: domain.Job{Kind: "index"}, Cron: "* * * * *", Timezone: "UTC", TimeoutSeconds: 60, Coalesce: true, Enabled: true}
}

func TestSchedulerConcurrentTaskAdmission(t *testing.T) {
	for _, automatic := range []bool{false, true} {
		t.Run(map[bool]string{false: "manual-manual", true: "manual-scheduled"}[automatic], func(t *testing.T) {
			repository, path := fixture(t)
			competitor, err := Open(path)
			if err != nil {
				t.Fatal(err)
			}
			defer competitor.Close()
			ctx := context.Background()
			task, err := repository.Create(ctx, taskInput(), nil, nil)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := repository.Enqueue(ctx, *task, []int64{60}); err != nil {
				t.Fatal(err)
			}
			barrier := make(chan struct{})
			results := make(chan int, 2)
			failures := make(chan error, 2)
			var workers sync.WaitGroup
			for index, repository := range []*Repository{repository, competitor} {
				workers.Go(func() {
					<-barrier
					if automatic && index == 1 {
						claims, err := repository.ClaimReady(ctx, "automatic", 100, 90, 1)
						failures <- err
						results <- len(claims)
					} else {
						admission, err := repository.ClaimManual(ctx, task.Id, "manual", 100, 90)
						failures <- err
						count := 0
						if admission.Claim != nil {
							count = 1
						}
						results <- count
					}
				})
			}
			close(barrier)
			workers.Wait()
			if count := <-results + <-results; count != 1 {
				t.Fatalf("%d competing admissions succeeded", count)
			}
			for range 2 {
				if err := <-failures; err != nil {
					t.Fatal(err)
				}
			}
		})
	}
}

func TestSchedulerAuditedMutationsFailClosed(t *testing.T) {
	repository, _ := fixture(t)
	ctx := context.Background()
	applyFixtureSql(t, repository, `INSERT INTO users(id,username,password_hash,salt,is_admin,created_at,updated_at) VALUES(1,'admin','hash','salt',1,1,1)`)
	actor := identity.Id(1)
	actorValue := int64(1)
	event := auditdomain.AuditEvent{ActorId: &actorValue, Action: "scheduler_create", Outcome: "completed", OccurredAt: 100}
	task, err := repository.Create(ctx, taskInput(), &actor, &event)
	if err != nil {
		t.Fatal(err)
	}
	events, err := repository.auth.ListAudit(ctx)
	if err != nil || len(events) != 1 || events[0].TargetId == nil || *events[0].TargetId != task.Id {
		t.Fatalf("missing target audit %#v %v", events, err)
	}
	before := snapshot(t, repository)
	applyFixtureSql(t, repository, `CREATE TRIGGER fail_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'private audit detail');END`)
	name := "updated"
	operations := []func() error{func() error { _, err := repository.Create(ctx, taskInput(), &actor, &event); return err }, func() error {
		_, err := repository.Update(ctx, domain.Update{TaskId: task.Id, Name: &name}, &actor, &event)
		return err
	}, func() error { _, err := repository.Delete(ctx, task.Id, &actor, &event); return err }}
	for _, operation := range operations {
		if err := operation(); !errors.Is(err, auditdomain.ErrAudit) {
			t.Fatalf("audit failure not propagated: %v", err)
		}
		if !reflect.DeepEqual(before, snapshot(t, repository)) {
			t.Fatal("failed audit mutated scheduler state")
		}
	}
	applyFixtureSql(t, repository, `DROP TRIGGER fail_audit;UPDATE users SET is_admin=0 WHERE id=1`)
	for _, operation := range operations {
		if err := operation(); !errors.Is(err, auditdomain.ErrAdminForbidden) {
			t.Fatalf("revoked admin mutation: %v", err)
		}
		if !reflect.DeepEqual(before, snapshot(t, repository)) {
			t.Fatal("revoked admin mutated scheduler state")
		}
	}
}

func TestSchedulerPostClaimTaskRead(t *testing.T) {
	for _, mutation := range []string{"DELETE FROM scheduled_tasks WHERE id=NEW.task_id", "UPDATE scheduled_tasks SET enabled=0 WHERE id=NEW.task_id", "UPDATE scheduled_tasks SET job_spec=NULL,legacy_command='legacy',enabled=0 WHERE id=NEW.task_id", "UPDATE scheduled_tasks SET job_spec='{}' WHERE id=NEW.task_id"} {
		t.Run(mutation, func(t *testing.T) {
			repository, _ := fixture(t)
			ctx := context.Background()
			task, err := repository.Create(ctx, taskInput(), nil, nil)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := repository.Enqueue(ctx, *task, []int64{60}); err != nil {
				t.Fatal(err)
			}
			applyFixtureSql(t, repository, "CREATE TRIGGER change_task AFTER UPDATE OF status ON scheduled_task_runs WHEN NEW.status='claimed' BEGIN "+mutation+";END")
			claims, err := repository.ClaimReady(ctx, "worker", 100, 90, 1)
			isCorrupt := mutation == "UPDATE scheduled_tasks SET job_spec='{}' WHERE id=NEW.task_id"
			if (err != nil) != isCorrupt || len(claims) != 0 {
				t.Fatalf("claims=%v error=%v", claims, err)
			}
			status, err := repository.Status(ctx, 100, 90, 10)
			if err != nil || len(status.RecentRuns) != 1 {
				t.Fatalf("%#v %v", status, err)
			}
			expected := domain.Error
			if isCorrupt {
				expected = domain.Claimed
			}
			if status.RecentRuns[0].Status != expected {
				t.Fatalf("committed run = %s", status.RecentRuns[0].Status)
			}
		})
	}
}
