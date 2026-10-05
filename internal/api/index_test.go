package api

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/auth"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	storageauth "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

func TestIndexRoutesMatchOriginalQueryAndAuthenticationOrder(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/api/http-query-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		ExporterSha256 string `json:"exporter_sha256"`
		Fixtures       []struct{ Name, Path, Sha256 string }
		Cases          []struct {
			Url           string
			Authenticated bool
			Status        int
			ContentType   string `json:"content_type"`
			Body          string
		}
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	exporter, err := os.ReadFile("../../tests/migration/api/export-mcp.mjs")
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(exporter)
	if hex.EncodeToString(digest[:]) != corpus.ExporterSha256 {
		t.Fatal("stale original HTTP observations")
	}
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	for _, fixture := range corpus.Fixtures {
		data, err := os.ReadFile(filepath.Join("../..", fixture.Path))
		if err != nil {
			t.Fatal(err)
		}
		digest := sha256.Sum256(data)
		if hex.EncodeToString(digest[:]) != fixture.Sha256 {
			t.Fatal("changed original content fixture")
		}
		if err := os.WriteFile(filepath.Join(configuration.IndexDir, fixture.Name), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	ctx := context.Background()
	if _, err := migration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		t.Fatal(err)
	}
	repository, err := storageauth.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	service := auth.New(repository, 2)
	user, err := service.Bootstrap(ctx, "index_fixture", "fixture password long", nil)
	if err != nil {
		t.Fatal(err)
	}
	token, err := service.CreateTrustedToken(ctx, user.Id, "fixture", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	pool := executor.New(8, 30*time.Second)
	handlers := indexHandlers{configuration, NewAuthenticator(service, pool), pool, &weekly.Cache{}}
	router := http.NewServeMux()
	routes := handlers.routes()
	if len(routes) != 14 {
		t.Fatal("index route inventory changed")
	}
	for _, route := range routes {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	if len(corpus.Cases) != 56 || len(corpus.Fixtures) == 0 {
		t.Fatal("incomplete original HTTP corpus")
	}
	for index, scenario := range corpus.Cases {
		t.Run(strconv.Itoa(index), func(t *testing.T) {
			request := httptest.NewRequest("GET", scenario.Url, nil)
			if scenario.Authenticated {
				request.Header.Set("Authorization", "Bearer "+token.Token)
			}
			recorder := httptest.NewRecorder()
			router.ServeHTTP(recorder, request)
			if recorder.Code != scenario.Status || recorder.Header().Get("Content-Type") != scenario.ContentType || recorder.Body.String() != scenario.Body {
				t.Fatalf("%s auth=%v: got %d %s %s; want %d %s %s", scenario.Url, scenario.Authenticated, recorder.Code, recorder.Header().Get("Content-Type"), recorder.Body.String(), scenario.Status, scenario.ContentType, scenario.Body)
			}
		})
	}
}

func TestIndexCancellationStopsActualQueryWorker(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	database, err := platform.Open(platform.Config{Filename: filepath.Join(configuration.IndexDir, "slow.sqlite"), Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec(`PRAGMA busy_timeout=1; CREATE TABLE marker(value INTEGER); INSERT INTO marker VALUES(0); CREATE VIEW journals AS WITH RECURSIVE numbers(value) AS (SELECT value FROM marker UNION ALL SELECT value+1 FROM numbers WHERE value<20000000) SELECT 'area' AS area FROM numbers`); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	if _, err := migration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		t.Fatal(err)
	}
	repository, err := storageauth.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	service := auth.New(repository, 2)
	user, err := service.Bootstrap(ctx, "cancel_fixture", "fixture password long", nil)
	if err != nil {
		t.Fatal(err)
	}
	token, err := service.CreateTrustedToken(ctx, user.Id, "fixture", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	pool := executor.New(1, 30*time.Second)
	defer pool.Wait()
	handlers := indexHandlers{configuration, NewAuthenticator(service, pool), pool, &weekly.Cache{}}
	requestContext, cancel := context.WithCancel(ctx)
	defer cancel()
	request := httptest.NewRequest("GET", "/api/meta/areas?db=slow.sqlite", nil).WithContext(requestContext)
	request.Header.Set("Authorization", "Bearer "+token.Token)
	done := make(chan struct{})
	go func() { defer close(done); handlers.handle(httptest.NewRecorder(), request, "list_areas") }()
	waitForIndexReader(t, database)
	cancel()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("caller did not cancel")
	}
	nextContext, stop := context.WithTimeout(ctx, time.Second)
	defer stop()
	_, err = executor.Run(nextContext, pool, func() (int, error) { return 1, nil })
	if errors.Is(err, context.DeadlineExceeded) {
		t.Fatal("canceled query kept the actual worker occupied")
	}
	if err != nil {
		t.Fatal(err)
	}
}

func waitForIndexReader(t *testing.T, database *sql.DB) {
	t.Helper()
	for deadline := time.Now().Add(3 * time.Second); time.Now().Before(deadline); {
		if _, err := database.Exec("UPDATE marker SET value=1-value"); err != nil {
			t.Fatal(err)
		}
		var busy, frames, checkpointed int
		if err := database.QueryRow("PRAGMA wal_checkpoint(RESTART)").Scan(&busy, &frames, &checkpointed); err != nil {
			t.Fatal(err)
		}
		if busy != 0 {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("query never held a database read snapshot")
}
