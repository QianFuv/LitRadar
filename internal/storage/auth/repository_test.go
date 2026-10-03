package auth

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"path/filepath"
	"sync"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func testRepository(t *testing.T) *Repository {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	repository, err := Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	return repository
}

func testAdmin(t *testing.T, repository *Repository) domain.Authorization {
	t.Helper()
	user, err := repository.Bootstrap(context.Background(), "admin", "old-hash", "old-salt", 10, nil)
	if err != nil {
		t.Fatal(err)
	}
	return domain.Authorization{User: user}
}

func testAudit(action string) domain.AuditEvent {
	return domain.AuditEvent{Action: action, Outcome: "completed", OccurredAt: 100}
}

func queryScalar[T any](t *testing.T, repository *Repository, query string, arguments ...any) T {
	t.Helper()
	var value T
	if err := repository.database.QueryRowContext(context.Background(), query, arguments...).Scan(&value); err != nil {
		t.Fatal(err)
	}
	return value
}

func runSql(t *testing.T, repository *Repository, statement string, arguments ...any) {
	t.Helper()
	if _, err := repository.database.ExecContext(context.Background(), statement, arguments...); err != nil {
		t.Fatal(err)
	}
}

func issue(t *testing.T, repository *Repository, authorization domain.Authorization, hash string, isLogin bool) domain.TokenInfo {
	t.Helper()
	token, err := repository.IssueToken(context.Background(), authorization, hash, "personal", 1000, 100, isLogin, nil)
	if err != nil {
		t.Fatal(err)
	}
	return token
}

func TestRepositoryOpenDoesNotMigrate(t *testing.T) {
	repository, err := Open(filepath.Join(t.TempDir(), "auth.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	if _, err := repository.BootstrapRequired(context.Background()); err == nil {
		t.Fatal("read unexpectedly created schema")
	}
	if version := queryScalar[int](t, repository, "PRAGMA user_version"); version != 0 {
		t.Fatal(version)
	}
}

func TestBootstrapHasOneConcurrentWinnerAndRegistrationCannotBootstrap(t *testing.T) {
	repository := testRepository(t)
	if _, err := repository.Register(context.Background(), "member", "hash", "salt", nil, 10, nil); !errors.Is(err, domain.ErrBootstrapRequired) {
		t.Fatal(err)
	}
	results := make(chan error, 8)
	var workers sync.WaitGroup
	for index := 0; index < 8; index++ {
		workers.Add(1)
		go func(index int) {
			defer workers.Done()
			_, err := repository.Bootstrap(context.Background(), fmt.Sprintf("admin%d", index), "hash", "salt", 10, nil)
			results <- err
		}(index)
	}
	workers.Wait()
	close(results)
	successes := 0
	for err := range results {
		if err == nil {
			successes++
		} else if !errors.Is(err, domain.ErrBootstrapComplete) {
			t.Fatal(err)
		}
	}
	if successes != 1 {
		t.Fatal(successes)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM folders WHERE is_tracking=1 AND name='默认收藏'"); count != 1 {
		t.Fatal(count)
	}
}

func TestRegistrationErrorPriorityAndAlwaysRequiredRedemptionAudit(t *testing.T) {
	repository := testRepository(t)
	owner := testAdmin(t, repository)
	if _, err := repository.Register(context.Background(), "admin", "hash", "salt", nil, 100, nil); !errors.Is(err, domain.ErrInviteRequired) {
		t.Fatal(err)
	}
	bad := "missing"
	if _, err := repository.Register(context.Background(), "ADMIN", "hash", "salt", &bad, 100, nil); !errors.Is(err, domain.ErrUsernameExists) {
		t.Fatal(err)
	}
	if _, err := repository.Register(context.Background(), "member", "hash", "salt", &bad, 100, nil); !errors.Is(err, domain.ErrInvite) {
		t.Fatal(err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM users"); count != 1 {
		t.Fatal("failed redemption persisted user")
	}
	code := "invite"
	if _, err := repository.IssueInvite(context.Background(), owner.User.Id, code, 100, 200, 2, false, nil); err != nil {
		t.Fatal(err)
	}
	for _, username := range []string{"member", "member2"} {
		if _, err := repository.Register(context.Background(), username, "hash", "salt", &code, 101, nil); err != nil {
			t.Fatal(err)
		}
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM security_audit_events WHERE action='invite_redeem'"); count != 2 {
		t.Fatal(count)
	}
	if first := queryScalar[int](t, repository, "SELECT used_by FROM invite_codes"); first != 2 {
		t.Fatal("first redemption overwritten")
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM invite_code_uses"); count != 2 {
		t.Fatal(count)
	}
}

func TestFinalInviteUseAndConcurrentIssuanceSerialize(t *testing.T) {
	repository := testRepository(t)
	owner := testAdmin(t, repository)
	ctx := context.Background()
	var workers sync.WaitGroup
	results := make(chan error, 8)
	for index := 0; index < 8; index++ {
		workers.Add(1)
		go func(index int) {
			defer workers.Done()
			_, err := repository.IssueInvite(ctx, owner.User.Id, fmt.Sprint(index), 100, 200, 1, false, nil)
			results <- err
		}(index)
	}
	workers.Wait()
	close(results)
	successes := 0
	for err := range results {
		if err == nil {
			successes++
		} else if !errors.Is(err, domain.ErrActiveInvite) {
			t.Fatal(err)
		}
	}
	if successes != 1 {
		t.Fatal(successes)
	}
	invite, err := repository.Invite(ctx, owner.User.Id)
	if err != nil {
		t.Fatal(err)
	}
	results = make(chan error, 8)
	for index := 0; index < 8; index++ {
		workers.Add(1)
		go func(index int) {
			defer workers.Done()
			_, err := repository.Register(ctx, fmt.Sprintf("member%d", index), "hash", "salt", &invite.Code, 101, nil)
			results <- err
		}(index)
	}
	workers.Wait()
	close(results)
	successes = 0
	for err := range results {
		if err == nil {
			successes++
		} else if !errors.Is(err, domain.ErrInvite) {
			t.Fatal(err)
		}
	}
	if successes != 1 || queryScalar[int](t, repository, "SELECT count(*) FROM users") != 2 || queryScalar[int](t, repository, "SELECT count(*) FROM invite_code_uses") != 1 {
		t.Fatal("final redemption was not atomic")
	}
}

func TestPersonalTokenQuotaAndLoginReplacementSerialize(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	for index := 0; index < 48; index++ {
		issue(t, repository, authorization, fmt.Sprint(index), false)
	}
	results := make(chan error, 8)
	var workers sync.WaitGroup
	for index := 0; index < 8; index++ {
		workers.Add(1)
		go func(index int) {
			defer workers.Done()
			_, err := repository.IssueToken(ctx, authorization, fmt.Sprintf("race%d", index), "personal", 1000, 100, false, nil)
			results <- err
		}(index)
	}
	workers.Wait()
	close(results)
	successes := 0
	for err := range results {
		if err == nil {
			successes++
		} else if !errors.Is(err, domain.ErrTokenLimit) {
			t.Fatal(err)
		}
	}
	if successes != 2 {
		t.Fatal(successes)
	}
	results = make(chan error, 8)
	for index := 0; index < 8; index++ {
		workers.Add(1)
		go func(index int) {
			defer workers.Done()
			_, err := repository.IssueToken(ctx, authorization, fmt.Sprintf("login%d", index), "ignored", 1000, 100, true, nil)
			results <- err
		}(index)
	}
	workers.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal(err)
		}
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM access_tokens WHERE name='login'"); count != 1 {
		t.Fatal(count)
	}
	tokens, err := repository.ListTokens(ctx, authorization.User.Id, 100)
	if err != nil || len(tokens) != 50 {
		t.Fatalf("%d %v", len(tokens), err)
	}
}

func TestTokenExpiryAndExactAuthorizationFences(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	issue(t, repository, authorization, "expired", false)
	if found, err := repository.VerifyToken(ctx, "expired", 1000); err != nil || found != nil {
		t.Fatalf("%v %v", found, err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM access_tokens"); count != 0 {
		t.Fatal(count)
	}
	token := issue(t, repository, authorization, "authorizer", false)
	observed, err := repository.VerifyToken(ctx, "authorizer", 100)
	if err != nil || observed == nil {
		t.Fatal(err)
	}
	if _, err := repository.RevokeTokenId(ctx, authorization.User.Id, token.Id, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.IssueToken(ctx, *observed, "denied", "new", 1000, 100, false, nil); !errors.Is(err, domain.ErrStaleAuthorization) {
		t.Fatal(err)
	}
	issue(t, repository, authorization, "new-authorizer", false)
	if _, err := repository.RevokeAll(ctx, authorization.User.Id, testAudit("logout_all")); err != nil {
		t.Fatal(err)
	}
	for _, isLogin := range []bool{false, true} {
		if _, err := repository.IssueToken(ctx, authorization, "stale", "new", 1000, 100, isLogin, nil); !errors.Is(err, domain.ErrStaleAuthorization) {
			t.Fatal(err)
		}
	}
	if _, err := repository.RevokeAll(ctx, authorization.User.Id, testAudit("logout_all")); err != nil {
		t.Fatal(err)
	}
	if generation := queryScalar[int](t, repository, "SELECT token_generation FROM users"); generation != 2 {
		t.Fatal(generation)
	}
}

func TestCredentialCasHasOneWinnerAndLegacyUpgradeDoesNotRevoke(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	issue(t, repository, authorization, "existing", false)
	observed, err := repository.CredentialsById(ctx, authorization.User.Id)
	if err != nil {
		t.Fatal(err)
	}
	if changed, err := repository.UpgradeLegacy(ctx, *observed, "upgraded", 101); err != nil || !changed {
		t.Fatalf("%t %v", changed, err)
	}
	if changed, err := repository.UpgradeLegacy(ctx, *observed, "loser", 102); err != nil || changed {
		t.Fatalf("%t %v", changed, err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM access_tokens"); count != 1 {
		t.Fatal("upgrade revoked token")
	}
	if generation := queryScalar[int](t, repository, "SELECT token_generation FROM users"); generation != 0 {
		t.Fatal(generation)
	}
	observed, err = repository.CredentialsById(ctx, authorization.User.Id)
	if err != nil {
		t.Fatal(err)
	}
	var workers sync.WaitGroup
	results := make(chan bool, 8)
	failures := make(chan error, 8)
	for index := 0; index < 8; index++ {
		workers.Add(1)
		go func(index int) {
			defer workers.Done()
			changed, err := repository.ChangePassword(ctx, *observed, fmt.Sprint(index), "", 103, testAudit("password_change"))
			results <- changed
			failures <- err
		}(index)
	}
	workers.Wait()
	close(results)
	close(failures)
	successes := 0
	for changed := range results {
		if changed {
			successes++
		}
	}
	for err := range failures {
		if err != nil {
			t.Fatal(err)
		}
	}
	if successes != 1 || queryScalar[int](t, repository, "SELECT token_generation FROM users") != 1 || queryScalar[int](t, repository, "SELECT count(*) FROM access_tokens") != 0 || queryScalar[int](t, repository, "SELECT count(*) FROM security_audit_events") != 1 {
		t.Fatal("CAS loser had side effects")
	}
}

func TestProtectedMutationsRollBackAuditAndTokenDeleteFailures(t *testing.T) {
	for _, failure := range []string{"audit", "delete"} {
		t.Run(failure, func(t *testing.T) {
			repository := testRepository(t)
			authorization := testAdmin(t, repository)
			ctx := context.Background()
			issue(t, repository, authorization, "existing", false)
			observed, err := repository.CredentialsById(ctx, authorization.User.Id)
			if err != nil {
				t.Fatal(err)
			}
			if failure == "audit" {
				runSql(t, repository, `CREATE TRIGGER fail_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'synthetic audit failure'); END`)
			} else {
				runSql(t, repository, `CREATE TRIGGER fail_delete BEFORE DELETE ON access_tokens BEGIN SELECT RAISE(ABORT,'synthetic delete failure'); END`)
			}
			if changed, err := repository.ChangePassword(ctx, *observed, "replacement", "", 101, testAudit("password_change")); err == nil || changed {
				t.Fatalf("%t %v", changed, err)
			}
			current, err := repository.CredentialsById(ctx, authorization.User.Id)
			if err != nil {
				t.Fatal(err)
			}
			if current.PasswordHash != observed.PasswordHash || current.Salt != observed.Salt || current.TokenGeneration != 0 || queryScalar[int](t, repository, "SELECT count(*) FROM access_tokens") != 1 {
				t.Fatal("credential change escaped rollback")
			}
			if _, err := repository.RevokeAll(ctx, authorization.User.Id, testAudit("logout_all")); err == nil {
				t.Fatal("logout ignored persistence failure")
			}
			if generation := queryScalar[int](t, repository, "SELECT token_generation FROM users"); generation != 0 {
				t.Fatal("failed logout advanced generation")
			}
		})
	}
}

func TestInviteAuditFailureRollsBackRedemptionAndRotation(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	code := "old"
	if _, err := repository.IssueInvite(ctx, authorization.User.Id, code, 100, 200, 1, false, nil); err != nil {
		t.Fatal(err)
	}
	runSql(t, repository, `CREATE TRIGGER fail_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'synthetic audit failure'); END`)
	if _, err := repository.Register(ctx, "member", "hash", "salt", &code, 101, nil); !errors.Is(err, domain.ErrAudit) {
		t.Fatal(err)
	}
	audit := testAudit("invite_rotate")
	if _, err := repository.IssueInvite(ctx, authorization.User.Id, "replacement", 102, 202, 1, true, &audit); !errors.Is(err, domain.ErrAudit) {
		t.Fatal(err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM users"); count != 1 {
		t.Fatal(count)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM invite_code_uses"); count != 0 {
		t.Fatal(count)
	}
	invite, err := repository.Invite(ctx, authorization.User.Id)
	if err != nil || invite.Code != code || invite.RevokedAt != nil || invite.UseCount != 0 {
		t.Fatalf("%v %v", invite, err)
	}
}

func TestRevocationAuditDistinguishesRowIdAndHashAbsence(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	audit := testAudit("logout")
	if deleted, err := repository.RevokeTokenId(ctx, authorization.User.Id, 999, &audit); err != nil || deleted {
		t.Fatalf("%t %v", deleted, err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM security_audit_events"); count != 0 {
		t.Fatal(count)
	}
	if deleted, err := repository.RevokeTokenHash(ctx, "absent", &audit); err != nil || deleted {
		t.Fatalf("%t %v", deleted, err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM security_audit_events"); count != 1 {
		t.Fatal(count)
	}
}

func TestAdministratorMutationRechecksActorAndPreservesLastAdmin(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	if err := repository.SetAdministrator(ctx, authorization.User.Id, authorization.User.Id, false, nil); !errors.Is(err, ErrLastAdministrator) {
		t.Fatal(err)
	}
	if err := repository.DeleteUser(ctx, authorization.User.Id, authorization.User.Id, nil); !errors.Is(err, ErrLastAdministrator) {
		t.Fatal(err)
	}
	runSql(t, repository, `INSERT INTO users(id,username,password_hash,salt,is_admin,created_at,updated_at) VALUES (2,'second','hash','salt',1,1,1)`)
	var workers sync.WaitGroup
	results := make(chan error, 2)
	for _, pair := range [][2]identity.Id{{1, 2}, {2, 1}} {
		workers.Add(1)
		go func(pair [2]identity.Id) {
			defer workers.Done()
			results <- repository.SetAdministrator(ctx, pair[0], pair[1], false, nil)
		}(pair)
	}
	workers.Wait()
	close(results)
	successes := 0
	for err := range results {
		if err == nil {
			successes++
		} else if !errors.Is(err, domain.ErrAdminForbidden) {
			t.Fatal(err)
		}
	}
	if successes != 1 || queryScalar[int](t, repository, "SELECT count(*) FROM users WHERE is_admin!=0") != 1 {
		t.Fatal("last administrator lost")
	}
	actor := identity.Id(queryScalar[int64](t, repository, "SELECT id FROM users WHERE is_admin=0"))
	audit := testAudit("user_password_reset")
	if _, err := repository.ResetPassword(ctx, &actor, authorization.User.Id, "replacement", "", 200, &audit); !errors.Is(err, domain.ErrAdminForbidden) {
		t.Fatal(err)
	}
}

func TestRoleChangeKeepsTokensAndDeleteRevokesCreatorInvites(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	runSql(t, repository, `INSERT INTO users(id,username,password_hash,salt,is_admin,created_at,updated_at) VALUES (2,'second','hash','salt',1,1,1)`)
	issue(t, repository, authorization, "existing", false)
	if _, err := repository.IssueInvite(ctx, authorization.User.Id, "invite", 100, 200, 1, false, nil); err != nil {
		t.Fatal(err)
	}
	if err := repository.SetAdministrator(ctx, 2, authorization.User.Id, false, nil); err != nil {
		t.Fatal(err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM access_tokens"); count != 1 {
		t.Fatal("role change revoked tokens")
	}
	if generation := queryScalar[int](t, repository, "SELECT token_generation FROM users WHERE id=1"); generation != 0 {
		t.Fatal(generation)
	}
	if err := repository.DeleteUser(ctx, 2, authorization.User.Id, nil); err != nil {
		t.Fatal(err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM access_tokens"); count != 0 {
		t.Fatal("delete did not cascade")
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM invite_codes WHERE revoked_at IS NOT NULL AND created_by IS NULL"); count != 1 {
		t.Fatal("creator invite remained redeemable")
	}
}

func TestExpiredUnrevokedInviteRequiresRotationAndAuditsBothIds(t *testing.T) {
	repository := testRepository(t)
	authorization := testAdmin(t, repository)
	ctx := context.Background()
	if _, err := repository.IssueInvite(ctx, authorization.User.Id, "old", 100, 101, 1, false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.IssueInvite(ctx, authorization.User.Id, "new", 200, 300, 1, false, nil); !errors.Is(err, domain.ErrActiveInvite) {
		t.Fatal(err)
	}
	audit := testAudit("invite_rotate")
	if _, err := repository.IssueInvite(ctx, authorization.User.Id, "new", 200, 300, 1, true, &audit); err != nil {
		t.Fatal(err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(DISTINCT target_id) FROM security_audit_events WHERE action='invite_rotate'"); count != 2 {
		t.Fatal(count)
	}
}

func TestAuditFailureRollsBackBootstrapAndLoginReplacement(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	audit := testAudit("admin_bootstrap")
	runSql(t, repository, `CREATE TRIGGER fail_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'synthetic audit failure'); END`)
	if _, err := repository.Bootstrap(ctx, "admin", "hash", "salt", 10, &audit); !errors.Is(err, domain.ErrAudit) {
		t.Fatal(err)
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM users"); count != 0 {
		t.Fatal(count)
	}
	authorization := testAdmin(t, repository)
	issue(t, repository, authorization, "existing", true)
	audit = testAudit("login")
	if _, err := repository.IssueToken(ctx, authorization, "replacement", "login", 1000, 100, true, &audit); !errors.Is(err, domain.ErrAudit) {
		t.Fatal(err)
	}
	if hash := queryScalar[string](t, repository, "SELECT token_hash FROM access_tokens"); hash != "existing" {
		t.Fatal("login lost original token")
	}
}

func TestAuditValidationRejectsSensitiveFreeTextWithoutMutation(t *testing.T) {
	repository := testRepository(t)
	audit := testAudit("bad action with spaces")
	err := repository.Immediate(context.Background(), false, func(connection *sql.Conn) error { return InsertAudit(context.Background(), connection, &audit) })
	if !errors.Is(err, domain.ErrAudit) || queryScalar[int](t, repository, "SELECT count(*) FROM security_audit_events") != 0 {
		t.Fatal(err)
	}
}
