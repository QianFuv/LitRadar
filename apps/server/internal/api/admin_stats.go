package api

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	storageauth "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/querydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
)

type authStats struct {
	TotalUsers              int64 `json:"total_users" nullable:"false" format:"int64"`
	AdminCount              int64 `json:"admin_count" nullable:"false" format:"int64"`
	TotalFolders            int64 `json:"total_folders" nullable:"false" format:"int64"`
	TotalFavorites          int64 `json:"total_favorites" nullable:"false" format:"int64"`
	TotalInviteCodes        int64 `json:"total_invite_codes" nullable:"false" format:"int64"`
	UsedInviteCodes         int64 `json:"used_invite_codes" nullable:"false" format:"int64"`
	UnusedInviteCodes       int64 `json:"unused_invite_codes" nullable:"false" format:"int64"`
	ActiveTokens            int64 `json:"active_tokens" nullable:"false" format:"int64"`
	NotificationSubscribers int64 `json:"notification_subscribers" nullable:"false" format:"int64"`
	ScheduledTasks          int64 `json:"scheduled_tasks" nullable:"false" format:"int64"`
	ActiveAnnouncements     int64 `json:"active_announcements" nullable:"false" format:"int64"`
}
type indexDatabaseStats struct {
	DbName   string `json:"db_name" nullable:"false"`
	Articles int64  `json:"articles" nullable:"false" format:"int64"`
	Journals int64  `json:"journals" nullable:"false" format:"int64"`
	Issues   int64  `json:"issues" nullable:"false" format:"int64"`
	Error    *bool  `json:"error,omitempty" required:"false" nullable:"true"`
}
type indexStats struct {
	Databases     []indexDatabaseStats `json:"databases" nullable:"false"`
	TotalArticles int64                `json:"total_articles" nullable:"false" format:"int64"`
	TotalJournals int64                `json:"total_journals" nullable:"false" format:"int64"`
}
type pushStats struct {
	DbName         string  `json:"db_name" nullable:"false"`
	Status         string  `json:"status" nullable:"false" enum:"idle,running,completed,failed,skipped,unknown,error"`
	LastCompleted  *string `json:"last_completed" required:"false" nullable:"true"`
	DeliveredCount *int    `json:"delivered_count,omitempty" required:"false" nullable:"true" minimum:"0" format:"int64"`
	UserResults    *int    `json:"user_results,omitempty" required:"false" nullable:"true" minimum:"0" format:"int64"`
}
type adminStats struct {
	Auth  authStats   `json:"auth" nullable:"false"`
	Index indexStats  `json:"index" nullable:"false"`
	Push  []pushStats `json:"push" nullable:"false"`
}

// readAdminStats loads generated counters and per-database projections.
func readAdminStats(ctx context.Context, repository *storageauth.Repository, configuration config.Config) (adminStats, error) {
	result := adminStats{Index: indexStats{Databases: []indexDatabaseStats{}}, Push: []pushStats{}}
	err := repository.WithConnection(ctx, func(connection *sql.Conn) error {
		now := currentTimestamp()
		queries := authdb.Queries{}
		if err := queries.DeleteExpiredTokens(ctx, connection, authdb.DeleteExpiredTokensParams{ExpiresAt: now}); err != nil {
			return err
		}
		row, err := queries.AdministratorStats(ctx, connection, authdb.AdministratorStatsParams{ExpiresAt: now})
		if err != nil {
			return err
		}
		result.Auth = authStats{TotalUsers: row.TotalUsers, AdminCount: row.AdminCount, TotalFolders: row.TotalFolders, TotalFavorites: row.TotalFavorites, TotalInviteCodes: row.TotalInviteCodes, UsedInviteCodes: row.UsedInviteCodes, UnusedInviteCodes: row.TotalInviteCodes - row.UsedInviteCodes, ActiveTokens: row.ActiveTokens, NotificationSubscribers: row.NotificationSubscribers, ScheduledTasks: row.ScheduledTasks, ActiveAnnouncements: row.ActiveAnnouncements}
		return nil
	})
	if err != nil {
		return adminStats{}, err
	}
	paths, err := configuration.ListIndexDatabases()
	if err != nil {
		return adminStats{}, err
	}
	for _, path := range paths {
		item, err := readIndexDatabaseStats(ctx, path)
		if err != nil {
			failed := true
			item = indexDatabaseStats{DbName: filepath.Base(path), Error: &failed}
		} else {
			result.Index.TotalArticles += item.Articles
			result.Index.TotalJournals += item.Journals
		}
		result.Index.Databases = append(result.Index.Databases, item)
	}
	result.Push, err = readPushStats(configuration.ProjectRoot)
	if err != nil {
		return adminStats{}, err
	}
	return result, nil
}

// readIndexDatabaseStats preserves optional issue-count failures in legacy databases.
func readIndexDatabaseStats(ctx context.Context, path string) (resultValue1 indexDatabaseStats, resultError error) {
	result := indexDatabaseStats{DbName: filepath.Base(path)}
	database, err := sqlite.OpenPlain(path)
	if err != nil {
		return result, err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	queries := querydb.Queries{}
	result.Articles, err = queries.CountArticles(ctx, database)
	if err != nil {
		return result, err
	}
	result.Journals, err = queries.CountJournals(ctx, database)
	if err != nil {
		return result, err
	}
	result.Issues, _ = queries.CountIssues(ctx, database)
	return result, nil
}

// readPushStats preserves directory ordering and excludes only the legacy ignored filenames.
func readPushStats(root string) ([]pushStats, error) {
	result := []pushStats{}
	directory := filepath.Join(root, "data", "push_state")
	if _, err := os.Stat(directory); err != nil {
		return result, nil
	}
	entries, err := os.ReadDir(directory)
	if err != nil {
		return nil, err
	}
	for _, entry := range entries {
		name := entry.Name()
		if name == ".json" || filepath.Ext(name) != ".json" || strings.HasSuffix(name, ".changes.json") {
			continue
		}
		item := readPushState(directory, name)
		result = append(result, item)
	}
	return result, nil
}

// readPushState retains error status for unreadable or malformed legacy state files.
func readPushState(directory, name string) pushStats {
	item := pushStats{DbName: strings.TrimSuffix(name, ".json"), Status: "error"}
	data, err := os.ReadFile(filepath.Join(directory, name))
	if err != nil {
		return item
	}
	value, err := transport.ParseJson(data)
	if err != nil {
		return item
	}
	object, _ := value.(map[string]any)
	status, _ := object["status"].(string)
	switch status {
	case "idle", "running", "completed", "failed", "skipped", "error":
		item.Status = status
	default:
		item.Status = "unknown"
	}
	if completed, ok := object["last_completed_run_at"].(string); ok {
		item.LastCompleted = &completed
	}
	run, _ := object["run"].(map[string]any)
	if delivered, ok := run["delivered_article_ids"].([]any); ok {
		count := len(delivered)
		item.DeliveredCount = &count
	}
	if users, ok := run["user_results"].([]any); ok {
		count := len(users)
		item.UserResults = &count
	}
	return item
}
