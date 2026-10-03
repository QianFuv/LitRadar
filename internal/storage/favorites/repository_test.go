package favorites

import (
	"context"
	"database/sql"
	"encoding/base64"
	"errors"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func fixtureRepository(t *testing.T) (*Repository, config.Config, identity.Id) {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	ctx := context.Background()
	if _, err := migration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		t.Fatal(err)
	}
	repository, err := auth.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	owner, err := repository.Bootstrap(ctx, "owner", "hash", "salt", 10, nil)
	if err != nil {
		t.Fatal(err)
	}
	return New(repository), configuration, owner.Id
}
func runSql(t *testing.T, repository *Repository, statement string) {
	t.Helper()
	if err := repository.auth.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), statement)
		return err
	}); err != nil {
		t.Fatal(err)
	}
}
func createFolder(t *testing.T, repository *Repository, owner identity.Id, name string, tracking bool) Folder {
	t.Helper()
	folder, err := repository.CreateFolder(context.Background(), owner, name, tracking)
	if err != nil {
		t.Fatal(err)
	}
	return folder
}
func addFavorite(t *testing.T, repository *Repository, owner identity.Id, folder int64, id identity.Id, note string) Favorite {
	t.Helper()
	favorite, err := repository.AddFavorite(context.Background(), owner, folder, Add{Reference{id, "metadata"}, note})
	if err != nil {
		t.Fatal(err)
	}
	return favorite
}
func installMetadata(t *testing.T, configuration config.Config) {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "..", "..", "tests", "migration", "storage", "fixtures", "metadata.sqlite.fixture"))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(configuration.IndexDir, "metadata.sqlite"), raw, 0600); err != nil {
		t.Fatal(err)
	}
}

func TestFolderTrackingOwnershipAndFailedInsertRollback(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	first := createFolder(t, repository, owner, " first ", true)
	second := createFolder(t, repository, owner, "second", false)
	if first.Name != "first" {
		t.Fatal(first)
	}
	if _, err := repository.CreateFolder(ctx, owner, "second", true); !errors.Is(err, ErrDuplicateFolder) {
		t.Fatal(err)
	}
	selected, err := repository.TrackingFolder(ctx, owner)
	if err != nil || selected == nil || selected.Id != first.Id {
		t.Fatalf("%+v %v", selected, err)
	}
	if changed, err := repository.SetTrackingFolder(ctx, owner+1, second.Id); err != nil || changed {
		t.Fatalf("cross-owner %v %v", changed, err)
	}
	if changed, err := repository.SetTrackingFolder(ctx, owner, second.Id); err != nil || !changed {
		t.Fatalf("tracking %v %v", changed, err)
	}
	if changed, err := repository.RenameFolder(ctx, owner+1, second.Id, "hidden"); err != nil || changed {
		t.Fatalf("rename %v %v", changed, err)
	}
	for _, name := range []string{" ", strings.Repeat("文", 101)} {
		if _, err := repository.CreateFolder(ctx, owner, name, false); err == nil || err.Error() != "Folder name must be 1-100 characters" {
			t.Fatal(err)
		}
	}
}

func TestIgnoredTrackingUpdateRollsBackPreviousSelection(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	first := createFolder(t, repository, owner, "first", true)
	second := createFolder(t, repository, owner, "second", false)
	runSql(t, repository, `CREATE TRIGGER ignore_tracking BEFORE UPDATE OF is_tracking ON folders WHEN NEW.name='second' AND NEW.is_tracking=1 BEGIN SELECT RAISE(IGNORE); END;`)
	if changed, err := repository.SetTrackingFolder(context.Background(), owner, second.Id); err != nil || changed {
		t.Fatalf("%v %v", changed, err)
	}
	selected, err := repository.TrackingFolder(context.Background(), owner)
	if err != nil || selected == nil || selected.Id != first.Id {
		t.Fatalf("lost original tracking selection: %+v %v", selected, err)
	}
}

func TestRepeatedAddAndBulkMovePreserveInitialNotes(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	source := createFolder(t, repository, owner, "source", false)
	target := createFolder(t, repository, owner, "target", false)
	initial := addFavorite(t, repository, owner, source.Id, 1001, "initial")
	repeated := addFavorite(t, repository, owner, source.Id, 1001, "replacement")
	if !reflect.DeepEqual(initial, repeated) {
		t.Fatalf("duplicate changed %+v %+v", initial, repeated)
	}
	addFavorite(t, repository, owner, source.Id, 1002, "second")
	existing := addFavorite(t, repository, owner, target.Id, 1001, "target note")
	count, err := repository.BulkMove(ctx, owner, source.Id, target.Id, []Reference{{1001, "metadata"}, {1001, "metadata"}, {1002, "metadata"}, {999, "metadata"}})
	if err != nil || count != 2 {
		t.Fatalf("move %d %v", count, err)
	}
	items, err := repository.ListFavorites(ctx, owner, &target.Id, -1, 0)
	if err != nil || len(items) != 2 {
		t.Fatalf("%+v %v", items, err)
	}
	for _, item := range items {
		if item.ArticleId == 1001 && !reflect.DeepEqual(item, existing) {
			t.Fatalf("target overwritten %+v", item)
		}
	}
	if count, err := repository.CountFavorites(ctx, owner, &source.Id); err != nil || count != 0 {
		t.Fatalf("source=%d %v", count, err)
	}
}

func TestBulkMutationsRollbackLateFailures(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	source := createFolder(t, repository, owner, "source", false)
	target := createFolder(t, repository, owner, "target", false)
	runSql(t, repository, `CREATE TRIGGER reject_second BEFORE INSERT ON favorites WHEN NEW.article_id=1002 BEGIN SELECT RAISE(ABORT,'late failure'); END;`)
	if count, err := repository.BulkAdd(ctx, owner, source.Id, []Add{{Reference{1001, "metadata"}, "one"}, {Reference{1002, "metadata"}, "two"}}); err == nil || count != 0 {
		t.Fatalf("bulk=%d %v", count, err)
	}
	if count, err := repository.CountFavorites(ctx, owner, nil); err != nil || count != 0 {
		t.Fatalf("partial insert=%d %v", count, err)
	}
	runSql(t, repository, "DROP TRIGGER reject_second")
	addFavorite(t, repository, owner, source.Id, 1001, "one")
	addFavorite(t, repository, owner, source.Id, 1002, "two")
	runSql(t, repository, `CREATE TRIGGER reject_delete BEFORE DELETE ON favorites WHEN OLD.article_id=1002 BEGIN SELECT RAISE(ABORT,'late failure'); END;`)
	refs := []Reference{{1001, "metadata"}, {1002, "metadata"}}
	if count, err := repository.BulkMove(ctx, owner, source.Id, target.Id, refs); err == nil || count != 0 {
		t.Fatalf("move=%d %v", count, err)
	}
	if count, err := repository.CountFavorites(ctx, owner, &target.Id); err != nil || count != 0 {
		t.Fatalf("partial target=%d %v", count, err)
	}
	if count, err := repository.BulkRemove(ctx, owner, source.Id, refs); err == nil || count != 0 {
		t.Fatalf("remove=%d %v", count, err)
	}
	if count, err := repository.CountFavorites(ctx, owner, &source.Id); err != nil || count != 2 {
		t.Fatalf("partial deletion=%d %v", count, err)
	}
}

func TestBatchValidationAndEmptyOperationOrdering(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	folder := createFolder(t, repository, owner, "folder", false)
	addFavorite(t, repository, owner, folder.Id, 1001, "one")
	items, err := repository.BatchIsFavorited(ctx, owner, []int64{1001, -1, 1001, 1002, 0}, "metadata")
	if err != nil || len(items) != 2 || len(items[0].Folders) != 1 || items[1].ArticleId != 1002 || items[1].Folders == nil {
		t.Fatalf("%+v %v", items, err)
	}
	if _, err := repository.BatchIsFavorited(ctx, owner, make([]int64, 501), ""); err == nil || err.Error() != "article_ids must contain at most 500 items" {
		t.Fatal(err)
	}
	if count, err := repository.BulkRemove(ctx, owner, 999, nil); err != nil || count != 0 {
		t.Fatalf("%d %v", count, err)
	}
	if count, err := repository.BulkMove(ctx, owner, 998, 999, nil); err != nil || count != 0 {
		t.Fatalf("%d %v", count, err)
	}
	if _, err := repository.BulkAdd(ctx, owner, 999, nil); !errors.Is(err, ErrFolderNotFound) {
		t.Fatal(err)
	}
	if _, err := repository.AddFavorite(ctx, owner+1, folder.Id, Add{Reference{1001, "metadata"}, ""}); !errors.Is(err, ErrFolderNotFound) {
		t.Fatal(err)
	}
}

func TestDeleteTrackingFolderChecksNotificationDependencies(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	folder := createFolder(t, repository, owner, "tracking", true)
	runSql(t, repository, `INSERT INTO notification_settings(user_id,delivery_method,pushplus_token,sync_to_tracking_folder,created_at,updated_at) SELECT id,'folder','',0,1,1 FROM users;`)
	if changed, err := repository.DeleteFolder(ctx, owner, folder.Id); err == nil || changed || err.Error() != "A tracking folder is required when delivery_method is 'folder'" {
		t.Fatalf("%v %v", changed, err)
	}
	runSql(t, repository, `UPDATE notification_settings SET delivery_method='pushplus',pushplus_token='encrypted-token',sync_to_tracking_folder=1`)
	if changed, err := repository.DeleteFolder(ctx, owner, folder.Id); err == nil || changed {
		t.Fatalf("%v %v", changed, err)
	}
	runSql(t, repository, `UPDATE notification_settings SET sync_to_tracking_folder=0`)
	if changed, err := repository.DeleteFolder(ctx, owner, folder.Id); err != nil || !changed {
		t.Fatalf("%v %v", changed, err)
	}
}

func TestCursorGrammarAndOwnerBinding(t *testing.T) {
	encode := func(raw string) string { return base64.RawURLEncoding.EncodeToString([]byte(raw)) }
	for _, raw := range []string{"1|+1|02|+0|+3", "1|1|2|8000000000000000|3", "1|1|2|00000000000000000000000000|3"} {
		if stamp, id, err := decodeCursor(encode(raw), 1, 2); err != nil || stamp != 0 || id != 3 {
			t.Fatalf("%s: %v %d %v", raw, stamp, id, err)
		}
	}
	valid := encodeCursor(1, Favorite{Id: 3, FolderId: 2, CreatedAt: 0.5})
	for _, cursor := range []string{valid + "=", valid + "\r\n", encode("1|2|2|0|3"), encode("1|1|3|0|3"), encode("1|1|2|7ff0000000000000|3"), encode("1|1|2|7ff8000000000001|3"), encode("1|1|2|bff0000000000000|3"), encode("1|1|2|0x0|3"), encode("1|1|2|0|0"), strings.Repeat("A", 129)} {
		if _, _, err := decodeCursor(cursor, 1, 2); !errors.Is(err, ErrCursor) {
			t.Fatalf("accepted %q", cursor)
		}
	}
	stamp, id, err := decodeCursor(valid, 1, 2)
	if err != nil || math.Float64bits(stamp) != math.Float64bits(0.5) || id != 3 {
		t.Fatalf("%v %d %v", stamp, id, err)
	}
}

func TestFavoritePageAndCitationSnapshotKeepStableOwnershipAndOrder(t *testing.T) {
	repository, configuration, owner := fixtureRepository(t)
	ctx := context.Background()
	folder := createFolder(t, repository, owner, "folder", false)
	installMetadata(t, configuration)
	addFavorite(t, repository, owner, folder.Id, 1001, "one")
	addFavorite(t, repository, owner, folder.Id, 1012, "two")
	addFavorite(t, repository, owner, folder.Id, 999, "missing")
	runSql(t, repository, "UPDATE favorites SET created_at=12.25")
	page, err := repository.ArticlePage(ctx, configuration, owner, folder.Id, 1, nil)
	if err != nil || len(page.Items) != 1 || page.Items[0].ArticleId != 999 || page.Items[0].MetadataStatus != "missing" || page.Page.NextCursor == nil {
		t.Fatalf("%+v %v", page, err)
	}
	second, err := repository.ArticlePage(ctx, configuration, owner, folder.Id, 1, page.Page.NextCursor)
	if err != nil || len(second.Items) != 1 || second.Items[0].ArticleId != 1012 || second.Items[0].MetadataStatus != "available" {
		t.Fatalf("%+v %v", second, err)
	}
	bad := "invalid"
	if _, err := repository.ArticlePage(ctx, configuration, owner+1, folder.Id, 1, &bad); !errors.Is(err, ErrFolderNotFound) {
		t.Fatal(err)
	}
	snapshot, err := repository.LoadCitationSnapshot(ctx, owner, folder.Id, 0)
	if err != nil || !snapshot.HasMore || len(snapshot.References) != 0 || snapshot.FolderName != "folder" {
		t.Fatalf("%+v %v", snapshot, err)
	}
	snapshot, err = repository.LoadCitationSnapshot(ctx, owner, folder.Id, 3)
	if err != nil || snapshot.HasMore || len(snapshot.References) != 3 || snapshot.References[0].ArticleId != 999 {
		t.Fatalf("%+v %v", snapshot, err)
	}
}

func TestMetadataFailureIsGroupedButCitationFailsClosed(t *testing.T) {
	_, configuration, _ := fixtureRepository(t)
	ctx := context.Background()
	installMetadata(t, configuration)
	favorites := []Favorite{{Reference: Reference{1001, "metadata"}}, {Reference: Reference{1012, "metadata"}}, {Reference: Reference{999, "missing"}}}
	database, err := storage.OpenPlain(filepath.Join(configuration.IndexDir, "metadata.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.ExecContext(ctx, "UPDATE articles SET authors_json='broken' WHERE article_id=1012"); err != nil {
		t.Fatal(err)
	}
	database.Close()
	enriched := Enrich(ctx, configuration, favorites)
	if enriched[0].MetadataStatus != "unavailable" || enriched[1].MetadataStatus != "unavailable" || enriched[2].MetadataStatus != "missing" || enriched[0].Title != nil {
		t.Fatalf("%+v", enriched)
	}
	if _, err := LoadCitationRecords(ctx, configuration, []Reference{{1001, "metadata"}, {1012, "metadata"}}); err == nil {
		t.Fatal("partial citation accepted")
	}
	records, err := LoadCitationRecords(ctx, configuration, []Reference{{1001, "metadata"}, {999, "missing"}, {1001, "metadata"}})
	if err != nil || len(records) != 3 || records[1].Authors == nil || records[1].Title != nil || !reflect.DeepEqual(records[0], records[2]) {
		t.Fatalf("%+v %v", records, err)
	}
}
