package auth

import (
	"context"
	"database/sql"
	"errors"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	sqlite3 "github.com/mattn/go-sqlite3"
)

func TestAdministratorCountsExactlyFlagOneInLegacyRows(t *testing.T) {
	repository := testRepository(t)
	owner := testAdmin(t, repository)
	runSql(t, repository, `INSERT INTO users(id,username,password_hash,salt,is_admin,created_at,updated_at) VALUES(2,'legacy','hash','salt',2,1,1)`)
	if err := repository.SetAdministrator(context.Background(), owner.User.Id, owner.User.Id, false, nil); !errors.Is(err, ErrLastAdministrator) {
		t.Fatal("noncanonical legacy flag counted as replacement administrator")
	}
}

func TestAdministratorInvitePolicyAuthorityAndPublicProjections(t *testing.T) {
	repository := testRepository(t)
	owner := testAdmin(t, repository)
	ctx := context.Background()
	quota := int64(3)
	audit := testAudit("invite_create")
	invite, err := repository.CreateAdministratorInvite(ctx, &owner.User.Id, nil, &quota, &audit)
	if err != nil || invite.CreatedBy != nil || invite.MaxUses != 3 || invite.Status != "active" {
		t.Fatalf("%v %v", invite, err)
	}
	if _, err := repository.CreateAdministratorInvite(ctx, &owner.User.Id, nil, nil, nil); err != nil {
		t.Fatal("ownerless invitations incorrectly share creator quota")
	}
	member, err := repository.Register(ctx, "member", "hash", "salt", &invite.Code, invite.CreatedAt+1, nil)
	if err != nil {
		t.Fatal(err)
	}
	expired := float64(0)
	if _, err := repository.CreateAdministratorInvite(ctx, &member.Id, &expired, nil, nil); !errors.Is(err, domain.ErrAdminForbidden) {
		t.Fatal("invalid policy preempted actor recheck")
	}
	if _, err := repository.CreateAdministratorInvite(ctx, &owner.User.Id, &expired, nil, nil); !errors.Is(err, ErrAdminInvitePolicy) {
		t.Fatal(err)
	}
	if revoked, err := repository.RevokeAdministratorInvite(ctx, &member.Id, invite.Id, nil); err == nil || revoked {
		t.Fatal("non-administrator revoked invite")
	}
	if revoked, err := repository.RevokeAdministratorInvite(ctx, &owner.User.Id, invite.Id, nil); err != nil || !revoked {
		t.Fatal(err)
	}
	items, err := repository.ListInvites(ctx, invite.CreatedAt+2)
	if err != nil || len(items) != 2 {
		t.Fatal(err)
	}
	var found bool
	for _, item := range items {
		if item.Id == invite.Id {
			found = true
			if item.Status != "revoked" || item.UsedBy == nil || *item.UsedBy != member.Id || item.UsedByName == nil || *item.UsedByName != "member" {
				t.Fatal("administrator invite history lost")
			}
		}
	}
	if !found {
		t.Fatal("invite omitted")
	}
	users, err := repository.ListUsers(ctx)
	if err != nil || len(users) != 2 || users[0].Id != identity.Id(1) || users[1].FolderCount != 1 || users[1].FavoriteCount != 0 || users[1].NotifyEnabled {
		t.Fatalf("%+v %v", users, err)
	}
}

func TestAuditRetentionDrainsBoundedBatchesBeforeAdvancingWindow(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	runSql(t, repository, `WITH RECURSIVE sequence(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM sequence WHERE value<10005) INSERT INTO security_audit_events(action,outcome,occurred_at) SELECT 'login','completed',1 FROM sequence`)
	first, err := repository.CleanupAudit(ctx, 1, 100000)
	if err != nil || !first.DidRun || first.DeletedCount != 10000 || !first.HasMoreExpired {
		t.Fatalf("%+v %v", first, err)
	}
	if isNull := queryScalar[bool](t, repository, "SELECT last_retention_at IS NULL FROM security_audit_maintenance"); !isNull {
		t.Fatal("window advanced before backlog drained")
	}
	second, err := repository.CleanupAudit(ctx, 1, 100000)
	if err != nil || !second.DidRun || second.DeletedCount != 5 || second.HasMoreExpired {
		t.Fatalf("%+v %v", second, err)
	}
	third, err := repository.CleanupAudit(ctx, 1, 100001)
	if err != nil || third.DidRun || third.DeletedCount != 0 {
		t.Fatalf("%+v %v", third, err)
	}
	if _, err := repository.AppendAudit(ctx, testAudit("login")); err != nil {
		t.Fatal(err)
	}
	events, err := repository.ListAudit(ctx)
	if err != nil || len(events) != 1 || events[0].Id != 10006 || events[0].Action != "login" {
		t.Fatalf("%+v %v", events, err)
	}
}

func TestAuditRetentionFailureRollsBackDeletionAndCountsOnce(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	if _, err := repository.AppendAudit(ctx, testAudit("login")); err != nil {
		t.Fatal(err)
	}
	runSql(t, repository, `CREATE TRIGGER fail_retention BEFORE UPDATE ON security_audit_maintenance BEGIN SELECT RAISE(ABORT,'synthetic failure'); END`)
	before := AuditFailureCount()
	if _, err := repository.CleanupAudit(ctx, 1, 100000); err == nil {
		t.Fatal("retention ignored marker failure")
	}
	if AuditFailureCount() != before+1 {
		t.Fatal("failure counted more than once")
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM security_audit_events"); count != 1 {
		t.Fatal("deletion escaped rollback")
	}
	if _, err := repository.CleanupAudit(ctx, 0, 100000); !errors.Is(err, ErrAuditRetention) {
		t.Fatal(err)
	}
}

func TestAuditFailuresRemainRedactedAndClassifiable(t *testing.T) {
	failure := AuditFailure{cause: sqlite3.Error{Code: sqlite3.ErrBusy}}
	if !errors.Is(failure, domain.ErrAudit) || !IsTransientContention(failure) || failure.Error() != domain.ErrAudit.Error() {
		t.Fatal("audit wrapper lost safe retry classification")
	}
	if IsTransientContention(AuditFailure{cause: sql.ErrNoRows}) {
		t.Fatal("non-contention error became retryable")
	}
	repository := testRepository(t)
	before := AuditFailureCount()
	audit := testAudit("unsafe secret text")
	if _, err := repository.AppendAudit(context.Background(), audit); !errors.Is(err, ErrAuditEvent) {
		t.Fatal(err)
	}
	if AuditFailureCount() != before+1 {
		t.Fatal("invalid standalone event not counted exactly once")
	}
}

func TestAuditTransactionBoundaryErrorsAreSafeAndAtomic(t *testing.T) {
	for _, stage := range []string{"begin", "commit"} {
		t.Run(stage, func(t *testing.T) {
			repository := testRepository(t)
			repository.database.SetMaxOpenConns(1)
			if stage == "begin" {
				runSql(t, repository, "PRAGMA query_only=ON")
			} else {
				runSql(t, repository, `CREATE TABLE audit_fault_parent(id INTEGER PRIMARY KEY);
CREATE TABLE audit_fault_child(parent_id INTEGER REFERENCES audit_fault_parent(id) DEFERRABLE INITIALLY DEFERRED);
CREATE TRIGGER audit_commit_fault AFTER INSERT ON security_audit_events BEGIN INSERT INTO audit_fault_child VALUES(1); END;
CREATE TRIGGER retention_commit_fault AFTER UPDATE ON security_audit_maintenance BEGIN INSERT INTO audit_fault_child VALUES(1); END;`)
			}
			check := func(err error, before uint64) {
				t.Helper()
				var failure sqlite3.Error
				if !errors.Is(err, domain.ErrAudit) || err.Error() != domain.ErrAudit.Error() || !errors.As(err, &failure) {
					t.Fatalf("unsafe or unclassifiable error: %v", err)
				}
				if AuditFailureCount() != before+1 {
					t.Fatal("failure counted more than once")
				}
			}
			before := AuditFailureCount()
			_, err := repository.AppendAudit(context.Background(), testAudit("login"))
			check(err, before)
			if count := queryScalar[int](t, repository, "SELECT count(*) FROM security_audit_events"); count != 0 {
				t.Fatal("failed commit retained audit")
			}
			before = AuditFailureCount()
			_, err = repository.CleanupAudit(context.Background(), 1, 100000)
			check(err, before)
			if isNull := queryScalar[bool](t, repository, "SELECT last_retention_at IS NULL FROM security_audit_maintenance"); !isNull {
				t.Fatal("failed commit advanced retention")
			}
		})
	}
}
