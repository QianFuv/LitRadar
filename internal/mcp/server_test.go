package mcp_test

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/api"
	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/mcp"
	"github.com/QianFuv/LitRadar/internal/platform/mcpcompat"
	storageauth "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

type wireClient struct {
	testing           *testing.T
	endpoint, session string
	client            *http.Client
}

func (client *wireClient) post(token string, body any) (int, map[string]any) {
	client.testing.Helper()
	encoded, err := json.Marshal(body)
	if err != nil {
		client.testing.Fatal(err)
	}
	request, _ := http.NewRequest("POST", client.endpoint, bytes.NewReader(encoded))
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Accept", "application/json, text/event-stream")
	request.Header.Set("Authorization", "Bearer "+token)
	if client.session != "" {
		request.Header.Set("Mcp-Session-Id", client.session)
		request.Header.Set("Mcp-Protocol-Version", "2025-06-18")
	}
	response, err := client.client.Do(request)
	if err != nil {
		client.testing.Fatal(err)
	}
	defer response.Body.Close()
	wire, err := io.ReadAll(response.Body)
	if err != nil {
		client.testing.Fatal(err)
	}
	if session := response.Header.Get("Mcp-Session-Id"); session != "" {
		client.session = session
	}
	var payload map[string]any
	if response.StatusCode != 200 {
		_ = json.Unmarshal(wire, &payload)
		return response.StatusCode, payload
	}
	for _, line := range strings.Split(string(wire), "\n") {
		if strings.HasPrefix(line, "data: {") {
			decoder := json.NewDecoder(strings.NewReader(strings.TrimPrefix(line, "data: ")))
			decoder.UseNumber()
			if err := decoder.Decode(&payload); err != nil {
				client.testing.Fatal(err)
			}
			return response.StatusCode, payload
		}
	}
	client.testing.Fatalf("missing SSE JSON: %s", wire)
	return 0, nil
}

func (client *wireClient) call(token, name string, arguments any) map[string]any {
	client.testing.Helper()
	status, payload := client.post(token, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": name, "arguments": arguments}})
	if status != 200 {
		client.testing.Fatalf("tool %s HTTP %d: %v", name, status, payload)
	}
	result, ok := payload["result"].(map[string]any)
	if !ok {
		client.testing.Fatalf("tool %s protocol error: %v", name, payload)
	}
	if _, exists := result["structuredContent"]; exists {
		client.testing.Fatal("unexpected structuredContent")
	}
	return result
}

func toolText(t *testing.T, result map[string]any) string {
	t.Helper()
	content, ok := result["content"].([]any)
	if !ok || len(content) != 1 {
		t.Fatalf("content: %v", result)
	}
	entry := content[0].(map[string]any)
	if entry["type"] != "text" {
		t.Fatal(entry)
	}
	return entry["text"].(string)
}

func TestAuthenticatedToolsUseFrozenContractsAndCurrentIdentity(t *testing.T) {
	ctx := context.Background()
	configuration := config.FromProjectRoot(t.TempDir())
	if _, err := migration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		t.Fatal(err)
	}
	repository, err := storageauth.Open(configuration.AuthDbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer repository.Close()
	service := auth.New(repository, 2)
	admin, err := service.Bootstrap(ctx, "fixture_admin", "fixture password long", nil)
	if err != nil {
		t.Fatal(err)
	}
	invite, err := service.IssueInvite(ctx, admin.Id, false, nil)
	if err != nil {
		t.Fatal(err)
	}
	reader, err := service.Register(ctx, "fixture_reader", "fixture password long", &invite.Code, nil)
	if err != nil {
		t.Fatal(err)
	}
	adminToken, err := service.CreateTrustedToken(ctx, admin.Id, "fixture", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	readerToken, err := service.CreateTrustedToken(ctx, reader.Id, "fixture", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	folders := favorites.New(repository)
	adminFolder, err := folders.CreateFolder(ctx, admin.Id, "admin only", false)
	if err != nil {
		t.Fatal(err)
	}
	readerFolder, err := folders.CreateFolder(ctx, reader.Id, "reader only", false)
	if err != nil {
		t.Fatal(err)
	}
	pool := executor.New(8, 30*time.Second)
	server, err := mcp.NewServer(mcp.Services{configuration, folders, pool})
	if err != nil {
		t.Fatal(err)
	}
	authenticator := api.NewAuthenticator(service, pool)
	handler := mcpcompat.New(server, authenticator.McpAuthorize)
	listener := httptest.NewServer(handler)
	defer listener.Close()
	defer handler.Close()
	client := wireClient{testing: t, endpoint: listener.URL, client: &http.Client{Timeout: 10 * time.Second}}
	status, _ := client.post(adminToken.Token, map[string]any{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": map[string]any{"protocolVersion": "2025-06-18", "capabilities": map[string]any{}, "clientInfo": map[string]any{"name": "fixture", "version": "1"}}})
	if status != 200 || client.session == "" {
		t.Fatal("real authentication did not initialize")
	}
	_, listed := client.post(adminToken.Token, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": map[string]any{}})
	frozen, err := os.ReadFile("../../tests/data/migration/surfaces.json")
	if err != nil {
		t.Fatal(err)
	}
	var contract struct{ Mcp struct{ Tools any } }
	decoder := json.NewDecoder(bytes.NewReader(frozen))
	decoder.UseNumber()
	if err := decoder.Decode(&contract); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(listed["result"].(map[string]any)["tools"], contract.Mcp.Tools) {
		t.Fatal("live tool inventory/schema differs from frozen Rust")
	}
	argumentVectors, err := os.ReadFile("../../tests/migration/api/mcp-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var original struct {
		Cases []struct {
			Name          string
			ArgumentsJson string `json:"arguments_json"`
			Result        map[string]any
		}
	}
	if err := json.Unmarshal(argumentVectors, &original); err != nil {
		t.Fatal(err)
	}
	if len(original.Cases) != 77 {
		t.Fatal("original MCP argument inventory changed")
	}
	for index, scenario := range original.Cases {
		t.Run("original-arguments-"+strconv.Itoa(index), func(t *testing.T) {
			actual := client.call(adminToken.Token, scenario.Name, json.RawMessage(scenario.ArgumentsJson))
			if !reflect.DeepEqual(actual, scenario.Result) {
				t.Fatalf("%s %s: got %v; want %v", scenario.Name, scenario.ArgumentsJson, actual, scenario.Result)
			}
		})
	}
	for _, scenario := range []struct {
		name    string
		args    any
		message string
	}{
		{"get_article", map[string]any{}, "failed to deserialize parameters: missing field `article_id`"},
		{"get_article", map[string]any{"article_id": 1}, "failed to deserialize parameters: invalid type: integer `1`, expected a string"},
		{"get_article", map[string]any{"article_id": "0", "db": " "}, "db must not be empty"},
		{"search_articles", map[string]any{"journal_id": []string{"bad"}, "db": " "}, "journal_id must be a positive integer"},
		{"search_articles", map[string]any{"area": []any{1}}, "failed to deserialize parameters: data did not match any variant of untagged enum StringOrStrings"},
		{"list_journals", map[string]any{"limit": 201}, "limit must be between 1 and 200"},
		{"list_journals", map[string]any{"limit": 1.5}, "failed to deserialize parameters: invalid type: floating point `1.5`, expected i64"},
		{"list_areas", map[string]any{"db": "missing"}, "Database not found"},
	} {
		result := client.call(adminToken.Token, scenario.name, scenario.args)
		if result["isError"] != true || toolText(t, result) != scenario.message {
			t.Fatalf("%s: %v", scenario.name, result)
		}
	}
	readerFolders := client.call(readerToken.Token, "list_folders", map[string]any{"unknown": "accepted"})
	if text := toolText(t, readerFolders); !strings.Contains(text, "reader only") || strings.Contains(text, "admin only") {
		t.Fatal("session cached initialize identity")
	}
	args := map[string]any{"folder_id": readerFolder.Id, "article_id": "9007199254740993", "db_name": "fixture"}
	added := client.call(readerToken.Token, "add_favorite", args)
	if added["isError"] == true || !strings.Contains(toolText(t, added), `"article_id": "9007199254740993"`) {
		t.Fatal(added)
	}
	denied := client.call(adminToken.Token, "remove_favorite", args)
	if denied["isError"] != true || toolText(t, denied) != "Favorite not found" {
		t.Fatal("favorite mutation crossed user boundary", denied)
	}
	removed := client.call(readerToken.Token, "remove_favorite", args)
	if removed["isError"] == true || toolText(t, removed) != "{\n  \"ok\": true\n}" {
		t.Fatal(removed)
	}
	if missing := client.call(readerToken.Token, "remove_favorite", args); toolText(t, missing) != "Favorite not found" || missing["isError"] != true {
		t.Fatal(missing)
	}
	args["folder_id"] = adminFolder.Id
	if denied := client.call(readerToken.Token, "add_favorite", args); denied["isError"] != true {
		t.Fatal("foreign folder accepted")
	}
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	fixture, err := os.ReadFile("../../tests/migration/storage/fixtures/metadata.sqlite.fixture")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(configuration.IndexDir, "metadata.sqlite"), fixture, 0600); err != nil {
		t.Fatal(err)
	}
	metadata, err := os.ReadFile("../../tests/migration/storage/metadata-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var vectors struct {
		Cases []struct {
			Operation string
			Id        int64
			Params    map[string]any
			Output    json.RawMessage
			Error     *string
		}
	}
	if err := json.Unmarshal(metadata, &vectors); err != nil {
		t.Fatal(err)
	}
	toolNames := map[string]string{"areas": "list_areas", "ratings": "list_journal_ratings", "years": "list_years", "options": "list_journal_options", "journal": "get_journal", "article": "get_article", "journals": "list_journals", "articles": "search_articles"}
	checked := 0
	filtered := map[string]int{}
	for _, vector := range vectors.Cases {
		name, supported := toolNames[vector.Operation]
		if !supported || vector.Error != nil {
			continue
		}
		arguments := map[string]any{}
		if vector.Operation == "journal" {
			arguments["journal_id"] = strconv.FormatInt(vector.Id, 10)
		}
		if vector.Operation == "article" {
			arguments["article_id"] = strconv.FormatInt(vector.Id, 10)
		}
		if vector.Params != nil {
			ratings, hasRatings := vector.Params["ratings"].(map[string]any)
			if vector.Operation == "articles" && hasRatings {
				for key, value := range vector.Params {
					if key != "ratings" {
						arguments[key] = value
					}
				}
				if identifiers, ok := arguments["journal_id"].([]any); ok {
					values := make([]string, len(identifiers))
					for index, identifier := range identifiers {
						values[index] = strconv.FormatInt(int64(identifier.(float64)), 10)
					}
					arguments["journal_id"] = values
				}
			} else if len(vector.Params) > 2 || vector.Params["limit"] != float64(50) {
				continue
			} else {
				arguments["limit"] = 50
			}
			if hasRatings {
				for name, values := range ratings {
					arguments[name] = values
				}
				filtered[vector.Operation]++
			} else if len(vector.Params) != 1 {
				continue
			}
		}
		result := client.call(adminToken.Token, name, arguments)
		if result["isError"] == true {
			t.Fatalf("%s: %v", name, result)
		}
		var actual, expected any
		if err := json.Unmarshal([]byte(toolText(t, result)), &actual); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(vector.Output, &expected); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(actual, expected) {
			t.Fatalf("%s differs from original storage observation", name)
		}
		checked++
	}
	if checked < 8 {
		t.Fatalf("metadata coverage only %d", checked)
	}
	if filtered["articles"] != 5 || filtered["journals"] != 3 {
		t.Fatalf("filtered membership coverage changed: %v", filtered)
	}
	databases := client.call(adminToken.Token, "list_databases", map[string]any{})
	if toolText(t, databases) != "[\n  \"metadata.sqlite\"\n]" {
		t.Fatal("database tool leaked paths", databases)
	}
	_, err = service.RevokeToken(ctx, readerToken.Token, domain.AuditEvent{Action: "logout", Outcome: "completed", OccurredAt: float64(time.Now().Unix())})
	if err != nil {
		t.Fatal(err)
	}
	status, _ = client.post(readerToken.Token, map[string]any{"jsonrpc": "2.0", "id": 9, "method": "tools/list"})
	if status != 401 {
		t.Fatal("existing session bypassed revoked credential")
	}
	pool.Close()
	_, unavailable := client.post(adminToken.Token, map[string]any{"jsonrpc": "2.0", "id": 10, "method": "tools/list"})
	if unavailable["code"] != "service_unavailable" {
		t.Fatal("closed authentication pool did not fail closed", unavailable)
	}
}
