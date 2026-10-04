package api

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/auth"
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
