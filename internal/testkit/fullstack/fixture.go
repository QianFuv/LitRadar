// Package fullstack seeds marker-guarded temporary databases for real-backend browser tests.
package fullstack

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/auth"
	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	cfpdomain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/announcements"
	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	cfpstorage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	indexstorage "github.com/QianFuv/LitRadar/internal/storage/index"
	"github.com/QianFuv/LitRadar/internal/storage/maintenance"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	indexmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/index"
)

const markerFile = ".litradar-e2e-root"
const markerContent = "litradar-full-stack-e2e-v1\n"
const databaseName = "full-stack.sqlite"
const articleTitle = "Evidence Graphs for Living Literature Reviews"
const articleDoi = "10.5555/litradar.fullstack"
const cfpUrl = "http://cfp-fixture.example/calls"

// Run preserves the original standalone fixture arguments and JSON report.
func Run(ctx context.Context, values []string, output io.Writer) error {
	args := slices.Clone(values)
	position := slices.Index(args, "--project-root")
	if position < 0 {
		return errors.New("--project-root is required")
	}
	if position+1 == len(args) {
		return errors.New("--project-root requires a path")
	}
	root := args[position+1]
	args = slices.Delete(args, position, position+2)
	position = slices.Index(args, "--refresh-cfp")
	shouldRefresh := position >= 0
	if shouldRefresh {
		args = slices.Delete(args, position, position+1)
	}
	if len(args) != 0 {
		return fmt.Errorf("unexpected fixture arguments: %s", strings.Join(args, " "))
	}
	var report any
	var err error
	if shouldRefresh {
		report, err = refresh(ctx, root)
	} else {
		report, err = seed(ctx, root)
	}
	if err != nil {
		return err
	}
	encoded, err := jsonvalue.EncodeJson(report)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintln(output, encoded)
	return err
}

func validateRoot(root string) (string, error) {
	metadata, err := os.Lstat(root)
	if err != nil {
		return "", err
	}
	if !metadata.IsDir() || metadata.Mode()&os.ModeSymlink != 0 {
		return "", errors.New("fixture root must be a real directory")
	}
	canonical, err := filepath.EvalSymlinks(root)
	if err != nil {
		return "", err
	}
	canonical, err = filepath.Abs(canonical)
	if err != nil {
		return "", err
	}
	temporary, err := filepath.EvalSymlinks(os.TempDir())
	if err != nil {
		return "", err
	}
	temporary, err = filepath.Abs(temporary)
	if err != nil {
		return "", err
	}
	relative, err := filepath.Rel(temporary, canonical)
	if err != nil || relative == "." || !filepath.IsLocal(relative) {
		return "", errors.New("fixture root must be below the OS temporary directory")
	}
	marker := filepath.Join(canonical, markerFile)
	metadata, err = os.Lstat(marker)
	if err != nil {
		return "", errors.New("fixture marker is missing")
	}
	if !metadata.Mode().IsRegular() {
		return "", errors.New("fixture marker must be a regular file")
	}
	content, err := os.ReadFile(marker)
	if err != nil {
		return "", err
	}
	if string(content) != markerContent {
		return "", errors.New("fixture marker content is invalid")
	}
	return canonical, nil
}

func seed(ctx context.Context, root string) (any, error) {
	root, err := validateRoot(root)
	if err != nil {
		return nil, err
	}
	if _, err := os.Stat(filepath.Join(root, "data")); err == nil {
		return nil, errors.New("fixture data already exists")
	}
	storage := config.FromProjectRoot(root)
	if err := maintenance.CheckInterrupted(storage); err != nil {
		return nil, err
	}
	if _, err := authmigration.Migrate(ctx, storage.AuthDbPath); err != nil {
		return nil, err
	}
	if _, err := delivery.ImportLegacyFiles(ctx, storage, float64(time.Now().Unix())); err != nil {
		return nil, err
	}
	if err := indexmigration.MigrateExisting(ctx, storage); err != nil {
		return nil, err
	}
	repository, err := authstorage.Open(storage.AuthDbPath)
	if err != nil {
		return nil, err
	}
	defer repository.Close()
	service := auth.New(repository, 2)
	administrator, err := service.Bootstrap(ctx, "fullstack_admin", "FullStackAdmin!2026", nil)
	if err != nil {
		return nil, err
	}
	invite, err := service.IssueInvite(ctx, administrator.Id, false, nil)
	if err != nil {
		return nil, err
	}
	member, err := service.Register(ctx, "fullstack_member", "FullStackMember!2026", &invite.Code, nil)
	if err != nil {
		return nil, err
	}
	if _, err := favorites.New(repository).CreateFolder(ctx, member.Id, "Reading", false); err != nil {
		return nil, err
	}
	if _, err := announcements.Create(ctx, repository, nil, "Seeded full-stack notice", "This announcement proves the real auth database is visible to the frontend.", "normal", true, nil); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(storage.IndexDir, 0777); err != nil {
		return nil, err
	}
	connection, err := indexstorage.OpenContent(ctx, filepath.Join(storage.IndexDir, databaseName))
	if err != nil {
		return nil, err
	}
	defer connection.Close()
	catalog := fixtureCatalog()
	if err := seedCfp(ctx, storage, catalog); err != nil {
		return nil, err
	}
	var changed uint64
	for _, isSecond := range []bool{false, true} {
		catalog, batch, revision := fixtureCatalog(), fixtureBatch(), "full-stack-seed-v1"
		if isSecond {
			catalog.CatalogId, catalog.Title, catalog.Issn, catalog.AllIssns = "full-stack-rating-second", "Complementary Methods Journal", nil, []string{}
			catalog.Rankings = domain.JournalRankings{AbsRating: pointer("4"), FmsRating: pointer("B")}
			batch.CatalogId, batch.Journal.CatalogId = catalog.CatalogId, catalog.CatalogId
			batch.Journal.ObservedTitle, batch.Journal.ObservedIssns = &catalog.Title, []string{}
			batch.Issues[0].CatalogId, batch.Articles[0].CatalogId = catalog.CatalogId, catalog.CatalogId
			batch.Articles[0].Title, batch.Articles[0].Doi = "Statistical Methods for Evidence Synthesis", pointer("10.1234/full-stack-rating-second")
			revision = "full-stack-rating-seed-v1"
		}
		if err := indexstorage.ReconcileCatalogIdentities(ctx, connection.Conn, []domain.JournalCatalogEntry{catalog}); err != nil {
			return nil, err
		}
		outcome, err := indexstorage.WriteContentBatch(ctx, connection.Conn, catalog, batch, revision, "2026-07-22T00:00:00Z")
		if err != nil {
			return nil, err
		}
		changed += outcome.ArticlesChanged
	}
	var articleId int64
	if err := connection.QueryRowContext(ctx, "SELECT article_id FROM articles WHERE doi=?", articleDoi).Scan(&articleId); err != nil {
		return nil, err
	}
	if _, err := connection.ExecContext(ctx, "UPDATE articles SET authors_json=? WHERE article_id=?", `["Ada Lovelace","Grace Hopper"]`, articleId); err != nil {
		return nil, err
	}
	if err := connection.Close(); err != nil {
		return nil, err
	}
	pushState := filepath.Join(root, "data", "push_state")
	if err := os.MkdirAll(pushState, 0777); err != nil {
		return nil, err
	}
	manifest, err := json.MarshalIndent(map[string]any{"db_name": databaseName, "generated_at": strconv.FormatInt(time.Now().Unix(), 10), "run_id": "full-stack-seed-v1", "notifiable_article_ids": []int64{articleId}}, "", "  ")
	if err != nil {
		return nil, err
	}
	if err := os.WriteFile(filepath.Join(pushState, "full-stack.changes.json"), manifest, 0666); err != nil {
		return nil, err
	}
	return map[string]any{"status": "seeded", "database": databaseName, "user_count": 2, "article_count": changed, "weekly_article_count": 1}, nil
}

func seedCfp(ctx context.Context, storage config.Config, catalog domain.JournalCatalogEntry) error {
	if err := os.MkdirAll(storage.MetaDir, 0777); err != nil {
		return err
	}
	row := make([]string, 16)
	row[0], row[2], row[3], row[5], row[7], row[9], row[11], row[13] = catalog.CatalogId, catalog.Title, *catalog.Issn, strings.Join(catalog.AllIssns, ";"), *catalog.Area, *catalog.Rankings.UtdRating, *catalog.Rankings.AbsRating, *catalog.Rankings.FmsRating
	header := "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating"
	if err := os.WriteFile(filepath.Join(storage.MetaDir, "full-stack.csv"), []byte(header+"\n"+strings.Join(row, ",")+"\n"), 0666); err != nil {
		return err
	}
	source := cfpdomain.Source{CatalogIds: []string{catalog.CatalogId}, JournalTitle: catalog.Title, Title: "Initial original CFP", Scope: "Original research on reproducible evidence synthesis.", Requirements: "Original manuscripts are welcome.", TypeText: "Special Issue", DateText: "Submission deadline: 30 November 2099", SourceUrl: cfpUrl, CheckedOn: "2026-09-15"}
	seed := cfpdomain.Seed{FormatVersion: 1, Sources: []cfpdomain.Source{source}, EmptyJournals: []cfpdomain.EmptyJournal{}, ExpectedJournals: pointer(uint64(1)), ExpectedNotices: pointer(uint64(1))}
	encoded, err := jsonvalue.EncodeJson(seed)
	if err != nil {
		return err
	}
	repository, err := cfpstorage.Open(storage.AuthDbPath)
	if err != nil {
		return err
	}
	defer repository.Close()
	_, err = repository.ImportSeed(ctx, "full-stack-cfp-v1", []byte(encoded))
	return err
}

func pointer[Value any](value Value) *Value { return &value }

func fixtureCatalog() domain.JournalCatalogEntry {
	return domain.JournalCatalogEntry{CatalogId: "full-stack-journal", CatalogAliases: []string{}, Title: "Journal of Reproducible Literature", Issn: pointer("1234-5679"), AllIssns: []string{"1234-5679"}, TitleAliases: []string{}, Area: pointer("Information Science"), Rankings: domain.JournalRankings{UtdRating: pointer("UTD24"), AbsRating: pointer("4*"), FmsRating: pointer("A")}}
}

func fixtureBatch() domain.ProviderBatch {
	return domain.ProviderBatch{CatalogId: "full-stack-journal", Journal: domain.JournalDraft{CatalogId: "full-stack-journal", ObservedTitle: pointer("Journal of Reproducible Literature"), ObservedIssns: []string{"1234-5679"}, ObservedTitleAliases: []string{}}, Issues: []domain.IssueDraft{{CatalogId: "full-stack-journal", PublicationYear: pointer(int64(2026)), Title: pointer("Full-stack verification issue"), Volume: pointer("12"), Number: pointer("3"), Date: pointer("2026-07")}}, Articles: []domain.ArticleDraft{{CatalogId: "full-stack-journal", Title: articleTitle, PublicationYear: pointer(int64(2026)), Date: pointer("2026-07-21"), IssueTitle: pointer("Full-stack verification issue"), Volume: pointer("12"), IssueNumber: pointer("3"), Authors: []domain.ArticleAuthorDraft{{DisplayName: "Ada Lovelace"}, {DisplayName: "Grace Hopper"}}, StartPage: pointer("101"), EndPage: pointer("118"), AbstractText: pointer("A deterministic nonempty article used to verify SQLite, search, detail, weekly, and favorite persistence."), Doi: pointer(articleDoi), OpenAccess: pointer(true), InPress: pointer(false), RetractionDois: []string{}}}, Progress: domain.ProviderProgress{State: domain.Complete}}
}
