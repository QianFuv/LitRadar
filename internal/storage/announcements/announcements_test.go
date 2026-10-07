package announcements

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func testRepository(t *testing.T) (*auth.Repository, identity.Id) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	repository, err := auth.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	admin, err := repository.Bootstrap(context.Background(), "admin", "hash", "salt", 10, nil)
	if err != nil {
		t.Fatal(err)
	}
	return repository, admin.Id
}

func execute(t *testing.T, repository *auth.Repository, statement string) {
	t.Helper()
	if err := repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), statement)
		return err
	}); err != nil {
		t.Fatal(err)
	}
}

func TestActiveVisibilityUsesExactlyOneAndPriorityThenCreation(t *testing.T) {
	repository, _ := testRepository(t)
	execute(t, repository, `INSERT INTO announcements(title,message,priority,enabled,created_at,updated_at) VALUES
 ('Normal newer','body','normal',1,20.5,21.5),('High older','body','high',1,10.25,11.25),
 ('Other newest','body','unexpected',1,30,31),('Disabled','body','high',0,40,41),('Noncanonical enabled','body','high',2,50,51);`)
	var filename string
	if err := repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		var sequence int
		var name string
		return connection.QueryRowContext(context.Background(), "PRAGMA database_list").Scan(&sequence, &name, &filename)
	}); err != nil {
		t.Fatal(err)
	}
	items, err := ListActive(context.Background(), filename)
	if err != nil {
		t.Fatal(err)
	}
	titles := []string{}
	for _, item := range items {
		titles = append(titles, item.Title)
	}
	if !reflect.DeepEqual(titles, []string{"High older", "Normal newer", "Other newest"}) || items[0].CreatedAt != 10.25 {
		t.Fatalf("active=%+v", items)
	}
	all, err := ListAll(context.Background(), repository)
	if err != nil || len(all) != 5 || all[0].Title != "Noncanonical enabled" || !all[0].Enabled {
		t.Fatalf("all=%+v %v", all, err)
	}
}

func TestPublicReadPreservesPlainConnectionAndMissingParent(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "public.sqlite")
	database, err := storage.OpenPlain(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec("CREATE TABLE announcements(id INTEGER,title TEXT,message TEXT,priority TEXT,enabled INTEGER,created_at REAL,updated_at REAL)"); err != nil {
		t.Fatal(err)
	}
	if items, err := ListActive(context.Background(), filename); err != nil || len(items) != 0 {
		t.Fatalf("%v %v", items, err)
	}
	var mode string
	if err := database.QueryRow("PRAGMA journal_mode").Scan(&mode); err != nil || mode != "delete" {
		t.Fatalf("public read changed journal mode: %s %v", mode, err)
	}
	missing := filepath.Join(t.TempDir(), "missing", "auth.sqlite")
	if _, err := ListActive(context.Background(), missing); err == nil {
		t.Fatal("missing parent unexpectedly created")
	}
	if _, err := os.Stat(filepath.Dir(missing)); !os.IsNotExist(err) {
		t.Fatalf("public reader created a directory: %v", err)
	}
}

// TestAuditedCrudNormalizesFieldsAndRetainsCreation checks normalization, retained creation and transactional audits.
func TestAuditedCrudNormalizesFieldsAndRetainsCreation(t *testing.T) {
	repository, actor := testRepository(t)
	ctx := context.Background()
	actorValue := int64(actor)
	audit := domain.AuditEvent{ActorId: &actorValue, Action: "announcement_create", Outcome: "completed", OccurredAt: 100.5}
	created := createNormalizedAnnouncement(t, ctx, repository, &actor, &audit)
	assertAnnouncementUpdateRetainsCreation(t, ctx, repository, &actor, created, &audit)
	audit.Action = "announcement_delete"
	assertDeletedAnnouncementStaysMissing(t, ctx, repository, &actor, created, &audit)
	assertAnnouncementAuditTargets(t, ctx, repository, created, actorValue, &audit)
}

// TestAuditFailureRollsBackEveryAnnouncementMutation checks all mutation kinds against a rejecting audit trigger.
func TestAuditFailureRollsBackEveryAnnouncementMutation(t *testing.T) {
	repository, actor := testRepository(t)
	ctx := context.Background()
	created, err := Create(ctx, repository, &actor, "Original", "Body", "normal", true, nil)
	if err != nil {
		t.Fatal(err)
	}
	execute(t, repository, `CREATE TRIGGER reject_announcement_audit BEFORE INSERT ON security_audit_events BEGIN SELECT RAISE(ABORT,'private diagnostic'); END;`)
	audit := domain.AuditEvent{Action: "announcement_update", Outcome: "completed", OccurredAt: 100}
	assertAnnouncementAuditCreateAndUpdateFailures(t, ctx, repository, &actor, created, &audit)
	if deleted, err := Delete(ctx, repository, &actor, created.Id, &audit); !errors.Is(err, domain.ErrAudit) || deleted {
		t.Fatalf("delete failure=%t %v", deleted, err)
	}
	items, err := ListAll(ctx, repository)
	if err != nil || len(items) != 1 || !reflect.DeepEqual(items[0], created) {
		t.Fatalf("mutation escaped failed audit: %+v %v", items, err)
	}
}

func TestRevokedActorCannotMutateAndValidationPrecedesAuthority(t *testing.T) {
	repository, actor := testRepository(t)
	ctx := context.Background()
	created, err := Create(ctx, repository, &actor, "Original", "Body", "normal", true, nil)
	if err != nil {
		t.Fatal(err)
	}
	execute(t, repository, "UPDATE users SET is_admin=0")
	if _, err := Create(ctx, repository, &actor, "New", "Body", "normal", true, nil); !errors.Is(err, domain.ErrAdminForbidden) {
		t.Fatal(err)
	}
	if _, err := Modify(ctx, repository, &actor, created.Id, Update{}, nil); !errors.Is(err, domain.ErrAdminForbidden) {
		t.Fatal(err)
	}
	if _, err := Delete(ctx, repository, &actor, created.Id, nil); !errors.Is(err, domain.ErrAdminForbidden) {
		t.Fatal(err)
	}
	if _, err := Create(ctx, repository, &actor, " ", "Body", "normal", true, nil); err == nil || err.Error() != "Title must be 1-200 characters" {
		t.Fatal(err)
	}
	if item, err := Get(ctx, repository, created.Id); err != nil || item == nil || item.Title != "Original" {
		t.Fatal("revoked actor changed announcement")
	}
}

func TestUnicodeFieldLimitsApplyAfterTrim(t *testing.T) {
	repository, actor := testRepository(t)
	ctx := context.Background()
	if _, err := Create(ctx, repository, &actor, " "+strings.Repeat("公", 200)+" ", strings.Repeat("告", 10000), " normal ", true, nil); err != nil {
		t.Fatal(err)
	}
	for _, input := range []struct{ title, message, priority, expected string }{
		{strings.Repeat("公", 201), "body", "normal", "Title must be at most 200 characters"},
		{"title", strings.Repeat("告", 10001), "normal", "Message must be at most 10000 characters"},
		{"title", "body", "ſormal", "Priority must be high, normal, or low"},
	} {
		if _, err := Create(ctx, repository, &actor, input.title, input.message, input.priority, true, nil); err == nil || err.Error() != input.expected {
			t.Fatalf("validation=%v expected=%s", err, input.expected)
		}
	}
}

// createNormalizedAnnouncement checks normalized fields and equal initial timestamps.
func createNormalizedAnnouncement(t *testing.T, ctx context.Context, repository *auth.Repository, actor *identity.Id, audit *domain.AuditEvent) Announcement {
	t.Helper()
	created, err := Create(ctx, repository, actor, "  公告  ", " body ", " HIGH ", true, audit)
	if err != nil {
		t.Fatal(err)
	}
	if created.Title != "公告" || created.Message != "body" || created.Priority != "high" || created.CreatedAt != created.UpdatedAt {
		t.Fatalf("created=%+v", created)
	}
	return created
}

// assertAnnouncementUpdateRetainsCreation checks explicit field updates without replacing creation time.
func assertAnnouncementUpdateRetainsCreation(t *testing.T, ctx context.Context, repository *auth.Repository, actor *identity.Id, created Announcement, audit *domain.AuditEvent) {
	t.Helper()
	title := " changed "
	isEnabled := false
	audit.Action = "announcement_update"
	updated, err := Modify(ctx, repository, actor, created.Id, Update{Title: &title, Enabled: &isEnabled}, audit)
	if err != nil {
		t.Fatal(err)
	}
	if updated.Title != "changed" || updated.Message != "body" || updated.Enabled || updated.CreatedAt != created.CreatedAt {
		t.Fatalf("updated=%+v", updated)
	}
}

// assertDeletedAnnouncementStaysMissing checks deletion and subsequent absent reads and mutations.
func assertDeletedAnnouncementStaysMissing(t *testing.T, ctx context.Context, repository *auth.Repository, actor *identity.Id, created Announcement, audit *domain.AuditEvent) {
	t.Helper()
	deleted, err := Delete(ctx, repository, actor, created.Id, audit)
	if err != nil || !deleted {
		t.Fatalf("delete=%t %v", deleted, err)
	}
	if item, err := Get(ctx, repository, created.Id); err != nil || item != nil {
		t.Fatalf("deleted record=%+v %v", item, err)
	}
	if item, err := Modify(ctx, repository, actor, created.Id, Update{}, audit); err != nil || item != nil {
		t.Fatalf("missing update=%+v %v", item, err)
	}
	if deleted, err := Delete(ctx, repository, actor, created.Id, audit); err != nil || deleted {
		t.Fatalf("missing delete=%t %v", deleted, err)
	}
}

// assertAnnouncementAuditTargets checks exactly three target-bound events without mutating the caller template.
func assertAnnouncementAuditTargets(t *testing.T, ctx context.Context, repository *auth.Repository, created Announcement, actorValue int64, audit *domain.AuditEvent) {
	t.Helper()
	events, err := repository.ListAudit(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 3 {
		t.Fatalf("audit count=%d", len(events))
	}
	for _, event := range events {
		if event.TargetId == nil || *event.TargetId != created.Id || event.ActorId == nil || *event.ActorId != actorValue {
			t.Fatalf("audit=%+v", event)
		}
	}
	if audit.TargetId != nil {
		t.Fatal("caller audit template mutated")
	}
}

// assertAnnouncementAuditCreateAndUpdateFailures checks failed create and update with sanitized diagnostics.
func assertAnnouncementAuditCreateAndUpdateFailures(t *testing.T, ctx context.Context, repository *auth.Repository, actor *identity.Id, created Announcement, audit *domain.AuditEvent) {
	t.Helper()
	title := "Changed"
	if result, err := Create(ctx, repository, actor, "New", "Body", "normal", true, audit); !errors.Is(err, domain.ErrAudit) || result.Id != 0 || strings.Contains(err.Error(), "private") {
		t.Fatalf("create failure=%+v %v", result, err)
	}
	if result, err := Modify(ctx, repository, actor, created.Id, Update{Title: &title}, audit); !errors.Is(err, domain.ErrAudit) || result != nil {
		t.Fatalf("update failure=%+v %v", result, err)
	}
}
