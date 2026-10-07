package api

import (
	"context"
	"database/sql"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/executor"
	storageauth "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	schedulerstorage "github.com/QianFuv/LitRadar/internal/storage/scheduler"
)

func TestPublicHealthAndAnnouncementsRemainUnauthenticated(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	ctx := context.Background()
	if _, err := migration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		t.Fatal(err)
	}
	scheduler, err := schedulerstorage.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer scheduler.Close()
	repository, err := storageauth.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	pool := executor.New(1, 30*time.Second)
	defer pool.Close()
	handlers := publicHandlers{configuration, pool, scheduler}
	router := http.NewServeMux()
	for _, route := range handlers.routes() {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	check := func(path string, status int, body string) {
		t.Helper()
		response := authRequest(router, "GET", path, "", "")
		if response.Code != status || response.Body.String() != body {
			t.Fatal(path, response.Code, response.Body.String())
		}
	}
	check("/health/live", 200, `{"status":"ok"}`)
	check("/health/ready", 503, `{"status":"unhealthy"}`)
	if err := scheduler.RecordHeartbeat(ctx, "fixture", currentTimestamp()); err != nil {
		t.Fatal(err)
	}
	check("/health/ready", 200, `{"status":"ok"}`)
	if err := scheduler.RecordHeartbeat(ctx, "fixture", currentTimestamp()-100); err != nil {
		t.Fatal(err)
	}
	check("/health/ready", 503, `{"status":"unhealthy"}`)
	assertPublicAnnouncements(t, ctx, repository, router)
	pool.Close()
	check("/health/live", 200, `{"status":"ok"}`)
	check("/health/ready", 503, `{"status":"unhealthy"}`)
	response := authRequest(router, "GET", "/api/announcements", "", "")
	if response.Code != 503 || response.Header().Get("Retry-After") != "5" {
		t.Fatal(response.Code, response.Body.String())
	}
}

// assertPublicAnnouncements checks public visibility while excluding disabled announcements.
func assertPublicAnnouncements(t *testing.T, ctx context.Context, repository *storageauth.Repository, router *http.ServeMux) {
	t.Helper()
	err := repository.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "INSERT INTO announcements(title,message,priority,enabled,created_at,updated_at) VALUES('public','message','normal',1,1,1),('hidden','secret','high',0,2,2)")
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	response := authRequest(router, "GET", "/api/announcements", "", "")
	if response.Code != 200 || !strings.Contains(response.Body.String(), "public") || strings.Contains(response.Body.String(), "hidden") {
		t.Fatal(response.Code, response.Body.String())
	}
}
