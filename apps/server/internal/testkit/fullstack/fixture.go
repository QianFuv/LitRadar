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
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	cfpdomain "github.com/QianFuv/LitRadar/apps/server/internal/domain/cfp"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/announcements"
	authstorage "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	cfpstorage "github.com/QianFuv/LitRadar/apps/server/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
	indexstorage "github.com/QianFuv/LitRadar/apps/server/internal/storage/index"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/maintenance"
	authmigration "github.com/QianFuv/LitRadar/apps/server/internal/storage/migrations/auth"
	indexmigration "github.com/QianFuv/LitRadar/apps/server/internal/storage/migrations/index"
)

const markerFile = ".litradar-e2e-root"
const markerContent = "litradar-full-stack-e2e-v1\n"
const databaseName = "full-stack.sqlite"
const articleTitle = "Evidence Graphs for Living Literature Reviews"
const articleDoi = "10.5555/litradar.fullstack"
const cfpURL = "http://cfp-fixture.example/calls"

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
	encoded, err := jsonvalue.EncodeJSON(report)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintln(output, encoded)
	return err
}

// validateRoot admits the original directory, canonical temporary containment and exact marker.
func validateRoot(root string) (string, error) {
	metadata, err := os.Lstat(root)
	if err != nil {
		return "", err
	}
	if !metadata.IsDir() || metadata.Mode()&os.ModeSymlink != 0 {
		return "", errors.New("fixture root must be a real directory")
	}
	canonical, err := canonicalFixturePath(root)
	if err != nil {
		return "", err
	}
	if err := validateFixtureContainment(canonical); err != nil {
		return "", err
	}
	if err := validateFixtureMarker(canonical); err != nil {
		return "", err
	}
	return canonical, nil
}

// seed owns both database lifetimes through ordered fixture writes and manifest publication.
func seed(ctx context.Context, root string) (resultValue1 any, resultError error) {
	storage, err := prepareSeedStorage(ctx, root)
	if err != nil {
		return nil, err
	}
	repository, err := authstorage.Open(storage.AuthDBPath)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := repository.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close repository: %w", closeError))
		}
	}()
	if err := seedFixtureAccounts(ctx, repository); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(storage.IndexDir, 0777); err != nil {
		return nil, err
	}
	connection, err := indexstorage.OpenContent(ctx, filepath.Join(storage.IndexDir, databaseName))
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeError := connection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	catalog := fixtureCatalog()
	if err := seedCfp(ctx, storage, catalog); err != nil {
		return nil, err
	}
	changed, articleID, err := seedFixtureArticles(ctx, connection)
	if err != nil {
		return nil, err
	}
	if err := connection.Close(); err != nil {
		return nil, err
	}
	if err := writeFixtureManifest(storage.ProjectRoot, articleID); err != nil {
		return nil, err
	}
	return map[string]any{"status": "seeded", "database": databaseName, "user_count": 2, "article_count": changed, "weekly_article_count": 1}, nil
}

func seedCfp(ctx context.Context, storage config.Config, catalog domain.JournalCatalogEntry) (resultError error) {
	if err := os.MkdirAll(storage.MetaDir, 0777); err != nil {
		return err
	}
	row := make([]string, 16)
	row[0], row[2], row[3], row[5], row[7], row[9], row[11], row[13] = catalog.CatalogID, catalog.Title, *catalog.Issn, strings.Join(catalog.AllIssns, ";"), *catalog.Area, *catalog.Rankings.UtdRating, *catalog.Rankings.AbsRating, *catalog.Rankings.FmsRating
	header := "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating"
	if err := os.WriteFile(filepath.Join(storage.MetaDir, "full-stack.csv"), []byte(header+"\n"+strings.Join(row, ",")+"\n"), 0666); err != nil {
		return err
	}
	source := cfpdomain.Source{CatalogIds: []string{catalog.CatalogID}, JournalTitle: catalog.Title, Title: "Initial original CFP", Scope: "Original research on reproducible evidence synthesis.", Requirements: "Original manuscripts are welcome.", TypeText: "Special Issue", DateText: "Submission deadline: 30 November 2099", SourceURL: cfpURL, CheckedOn: "2026-09-15"}
	seed := cfpdomain.Seed{FormatVersion: 1, Sources: []cfpdomain.Source{source}, EmptyJournals: []cfpdomain.EmptyJournal{}, ExpectedJournals: new(uint64(1)), ExpectedNotices: new(uint64(1))}
	encoded, err := jsonvalue.EncodeJSON(seed)
	if err != nil {
		return err
	}
	repository, err := cfpstorage.Open(storage.AuthDBPath)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := repository.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close repository: %w", closeError))
		}
	}()
	_, err = repository.ImportSeed(ctx, "full-stack-cfp-v1", []byte(encoded))
	return err
}

func fixtureCatalog() domain.JournalCatalogEntry {
	return domain.JournalCatalogEntry{CatalogID: "full-stack-journal", CatalogAliases: []string{}, Title: "Journal of Reproducible Literature", Issn: new("1234-5679"), AllIssns: []string{"1234-5679"}, TitleAliases: []string{}, Area: new("Information Science"), Rankings: domain.JournalRankings{UtdRating: new("UTD24"), AbsRating: new("4*"), FmsRating: new("A")}}
}

func fixtureBatch() domain.ProviderBatch {
	return domain.ProviderBatch{CatalogID: "full-stack-journal", Journal: domain.JournalDraft{CatalogID: "full-stack-journal", ObservedTitle: new("Journal of Reproducible Literature"), ObservedIssns: []string{"1234-5679"}, ObservedTitleAliases: []string{}}, Issues: []domain.IssueDraft{{CatalogID: "full-stack-journal", PublicationYear: new(int64(2026)), Title: new("Full-stack verification issue"), Volume: new("12"), Number: new("3"), Date: new("2026-07")}}, Articles: []domain.ArticleDraft{{CatalogID: "full-stack-journal", Title: articleTitle, PublicationYear: new(int64(2026)), Date: new("2026-07-21"), IssueTitle: new("Full-stack verification issue"), Volume: new("12"), IssueNumber: new("3"), Authors: []domain.ArticleAuthorDraft{{DisplayName: "Ada Lovelace"}, {DisplayName: "Grace Hopper"}}, StartPage: new("101"), EndPage: new("118"), AbstractText: new("A deterministic nonempty article used to verify SQLite, search, detail, weekly, and favorite persistence."), Doi: new(articleDoi), OpenAccess: new(true), InPress: new(false), RetractionDois: []string{}}}, Progress: domain.ProviderProgress{State: domain.Complete}}
}

// canonicalFixturePath resolves links before obtaining the absolute path.
func canonicalFixturePath(root string) (string, error) {
	canonical, err := filepath.EvalSymlinks(root)
	if err != nil {
		return "", err
	}
	canonical, err = filepath.Abs(canonical)
	if err != nil {
		return "", err
	}
	return canonical, nil
}

// validateFixtureContainment admits only local descendants of the canonical OS temporary directory.
func validateFixtureContainment(canonical string) error {
	temporary, err := filepath.EvalSymlinks(os.TempDir())
	if err != nil {
		return err
	}
	temporary, err = filepath.Abs(temporary)
	if err != nil {
		return err
	}
	relative, err := filepath.Rel(temporary, canonical)
	if err != nil || relative == "." || !filepath.IsLocal(relative) {
		return errors.New("fixture root must be below the OS temporary directory")
	}
	return nil
}

// validateFixtureMarker preserves missing, nonregular, unreadable and invalid marker errors.
func validateFixtureMarker(canonical string) error {
	marker := filepath.Join(canonical, markerFile)
	metadata, err := os.Lstat(marker)
	if err != nil {
		return errors.New("fixture marker is missing")
	}
	if !metadata.Mode().IsRegular() {
		return errors.New("fixture marker must be a regular file")
	}
	content, err := os.ReadFile(marker)
	if err != nil {
		return err
	}
	if string(content) != markerContent {
		return errors.New("fixture marker content is invalid")
	}
	return nil
}

// prepareSeedStorage protects an unpopulated marked root before running migrations in order.
func prepareSeedStorage(ctx context.Context, root string) (config.Config, error) {
	root, err := validateRoot(root)
	if err != nil {
		return config.Config{}, err
	}
	if _, err := os.Stat(filepath.Join(root, "data")); err == nil {
		return config.Config{}, errors.New("fixture data already exists")
	}
	storage := config.FromProjectRoot(root)
	if err := maintenance.CheckInterrupted(storage); err != nil {
		return config.Config{}, err
	}
	if _, err := authmigration.Migrate(ctx, storage.AuthDBPath); err != nil {
		return config.Config{}, err
	}
	if _, err := delivery.ImportLegacyFiles(ctx, storage, float64(time.Now().Unix())); err != nil {
		return config.Config{}, err
	}
	if err := indexmigration.MigrateExisting(ctx, storage); err != nil {
		return config.Config{}, err
	}
	return storage, nil
}

// seedFixtureAccounts writes the administrator, invite, member, reading folder and notice in order.
func seedFixtureAccounts(ctx context.Context, repository *authstorage.Repository) error {
	service := auth.New(repository, 2)
	administrator, err := service.Bootstrap(ctx, "fullstack_admin", "FullStackAdmin!2026", nil)
	if err != nil {
		return err
	}
	invite, err := service.IssueInvite(ctx, administrator.ID, false, nil)
	if err != nil {
		return err
	}
	member, err := service.Register(ctx, "fullstack_member", "FullStackMember!2026", &invite.Code, nil, nil)
	if err != nil {
		return err
	}
	if _, err := favorites.New(repository).CreateFolder(ctx, member.ID, "Reading", false); err != nil {
		return err
	}
	if _, err := announcements.Create(ctx, repository, nil, "Seeded full-stack notice", "This announcement proves the real auth database is visible to the frontend.", "normal", true, nil); err != nil {
		return err
	}
	return nil
}

// seedFixtureArticles reconciles both catalogs and retains the original article and legacy author encoding.
func seedFixtureArticles(ctx context.Context, connection *indexstorage.Connection) (uint64, int64, error) {
	var changed uint64
	for _, isSecond := range []bool{false, true} {
		catalog, batch, revision := fixtureCatalog(), fixtureBatch(), "full-stack-seed-v1"
		if isSecond {
			catalog.CatalogID, catalog.Title, catalog.Issn, catalog.AllIssns = "full-stack-rating-second", "Complementary Methods Journal", nil, []string{}
			catalog.Rankings = domain.JournalRankings{AbsRating: new("4"), FmsRating: new("B")}
			batch.CatalogID, batch.Journal.CatalogID = catalog.CatalogID, catalog.CatalogID
			batch.Journal.ObservedTitle, batch.Journal.ObservedIssns = &catalog.Title, []string{}
			batch.Issues[0].CatalogID, batch.Articles[0].CatalogID = catalog.CatalogID, catalog.CatalogID
			batch.Articles[0].Title, batch.Articles[0].Doi = "Statistical Methods for Evidence Synthesis", new("10.1234/full-stack-rating-second")
			revision = "full-stack-rating-seed-v1"
		}
		if err := indexstorage.ReconcileCatalogIdentities(ctx, connection.Conn, []domain.JournalCatalogEntry{catalog}); err != nil {
			return 0, 0, err
		}
		outcome, err := indexstorage.WriteContentBatch(ctx, connection.Conn, catalog, batch, revision, "2026-07-22T00:00:00Z")
		if err != nil {
			return 0, 0, err
		}
		changed += outcome.ArticlesChanged
	}
	var articleID int64
	if err := connection.QueryRowContext(ctx, "SELECT article_id FROM articles WHERE doi=?", articleDoi).Scan(&articleID); err != nil {
		return 0, 0, err
	}
	if _, err := connection.ExecContext(ctx, "UPDATE articles SET authors_json=? WHERE article_id=?", `["Ada Lovelace","Grace Hopper"]`, articleID); err != nil {
		return 0, 0, err
	}
	return changed, articleID, nil
}

// writeFixtureManifest publishes the first article only after the owner's checked content close.
func writeFixtureManifest(root string, articleID int64) error {
	pushState := filepath.Join(root, "data", "push_state")
	if err := os.MkdirAll(pushState, 0777); err != nil {
		return err
	}
	manifest, err := json.MarshalIndent(map[string]any{"db_name": databaseName, "generated_at": time.Now().UTC().Format(time.RFC3339Nano), "run_id": "full-stack-seed-v1", "notifiable_article_ids": []int64{articleID}}, "", "  ")
	if err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(pushState, "full-stack.changes.json"), manifest, 0666); err != nil {
		return err
	}
	return nil
}
