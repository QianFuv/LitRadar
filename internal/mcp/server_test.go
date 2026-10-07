package mcp_test

import (
	"bytes"
	"context"
	"encoding/json"
	contentfixture "github.com/QianFuv/LitRadar/internal/testkit/content"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"

	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/api"
	"github.com/QianFuv/LitRadar/internal/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/mcp"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
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

func TestAuthenticatedToolsEnforceCurrentIdentity(t *testing.T) {
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
	actors := authenticatedToolActors(t, ctx, service)
	folders := favorites.New(repository)
	adminFolder, readerFolder := authenticatedToolFolders(t, ctx, folders, actors)
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
	assertAuthenticatedToolInventory(t, &client, actors.adminToken.Token)
	assertAuthenticatedToolInputErrors(t, &client, actors.adminToken.Token)
	assertAuthenticatedFavoriteOwnership(t, &client, actors, adminFolder, readerFolder)
	assertDatabaseToolBasenames(t, &client, configuration, actors.adminToken.Token)
	assertRevokedToolCredential(t, ctx, &client, service, actors.readerToken.Token)
	assertClosedToolAuthentication(t, &client, pool, actors.adminToken.Token)
}

// authenticatedActors shares credentials across phases of one authenticated MCP session.
type authenticatedActors struct {
	admin, reader           domain.User
	adminToken, readerToken domain.IssuedToken
}

// authenticatedToolActors creates the administrator and invited reader in the original order.
func authenticatedToolActors(t *testing.T, ctx context.Context, service *auth.Service) authenticatedActors {
	t.Helper()
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
	return authenticatedActors{admin, reader, adminToken, readerToken}
}

// authenticatedToolFolders creates separate folders for the two authenticated users.
func authenticatedToolFolders(t *testing.T, ctx context.Context, folders *favorites.Repository, actors authenticatedActors) (favorites.Folder, favorites.Folder) {
	t.Helper()
	adminFolder, err := folders.CreateFolder(ctx, actors.admin.Id, "admin only", false)
	if err != nil {
		t.Fatal(err)
	}
	readerFolder, err := folders.CreateFolder(ctx, actors.reader.Id, "reader only", false)
	if err != nil {
		t.Fatal(err)
	}
	return adminFolder, readerFolder
}

// assertAuthenticatedToolInventory initializes the shared session and checks the declared tool inventory.
func assertAuthenticatedToolInventory(t *testing.T, client *wireClient, token string) {
	t.Helper()
	status, _ := client.post(token, map[string]any{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": map[string]any{"protocolVersion": "2025-06-18", "capabilities": map[string]any{}, "clientInfo": map[string]any{"name": "fixture", "version": "1"}}})
	if status != 200 || client.session == "" {
		t.Fatal("real authentication did not initialize")
	}
	_, listed := client.post(token, map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": map[string]any{}})
	if tools, ok := listed["result"].(map[string]any)["tools"].([]any); !ok || len(tools) != 13 {
		t.Fatal("unexpected tool inventory", listed)
	}

}

// assertAuthenticatedToolInputErrors checks wire error priority without losing the shared session.
func assertAuthenticatedToolInputErrors(t *testing.T, client *wireClient, token string) {
	t.Helper()
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
		result := client.call(token, scenario.name, scenario.args)
		if result["isError"] != true || toolText(t, result) != scenario.message {
			t.Fatalf("%s: %v", scenario.name, result)
		}
	}
}

// assertAuthenticatedFavoriteOwnership checks current identity and favorite mutation ownership in sequence.
func assertAuthenticatedFavoriteOwnership(t *testing.T, client *wireClient, actors authenticatedActors, adminFolder, readerFolder favorites.Folder) {
	t.Helper()
	assertCurrentToolIdentity(t, client, actors.readerToken.Token)
	args := map[string]any{"folder_id": readerFolder.Id, "article_id": "9007199254740993", "db_name": "fixture"}
	added := client.call(actors.readerToken.Token, "add_favorite", args)
	if added["isError"] == true || !strings.Contains(toolText(t, added), `"article_id": "9007199254740993"`) {
		t.Fatal(added)
	}
	denied := client.call(actors.adminToken.Token, "remove_favorite", args)
	if denied["isError"] != true || toolText(t, denied) != "Favorite not found" {
		t.Fatal("favorite mutation crossed user boundary", denied)
	}
	removed := client.call(actors.readerToken.Token, "remove_favorite", args)
	if removed["isError"] == true || toolText(t, removed) != "{\n  \"ok\": true\n}" {
		t.Fatal(removed)
	}
	if missing := client.call(actors.readerToken.Token, "remove_favorite", args); toolText(t, missing) != "Favorite not found" || missing["isError"] != true {
		t.Fatal(missing)
	}
	args["folder_id"] = adminFolder.Id
	if denied := client.call(actors.readerToken.Token, "add_favorite", args); denied["isError"] != true {
		t.Fatal("foreign folder accepted")
	}
}

// assertDatabaseToolBasenames checks that database discovery returns only basenames.
func assertDatabaseToolBasenames(t *testing.T, client *wireClient, configuration config.Config, token string) {
	t.Helper()
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	if err := contentfixture.Create(filepath.Join(configuration.IndexDir, "metadata.sqlite"), 9); err != nil {
		t.Fatal(err)
	}

	databases := client.call(token, "list_databases", map[string]any{})
	if toolText(t, databases) != "[\n  \"metadata.sqlite\"\n]" {
		t.Fatal("database tool leaked paths", databases)
	}
}

// assertRevokedToolCredential checks that the established session still rejects revoked credentials.
func assertRevokedToolCredential(t *testing.T, ctx context.Context, client *wireClient, service *auth.Service, token string) {
	t.Helper()
	_, err := service.RevokeToken(ctx, token, domain.AuditEvent{Action: "logout", Outcome: "completed", OccurredAt: float64(time.Now().Unix())})
	if err != nil {
		t.Fatal(err)
	}
	status, _ := client.post(token, map[string]any{"jsonrpc": "2.0", "id": 9, "method": "tools/list"})
	if status != 401 {
		t.Fatal("existing session bypassed revoked credential")
	}
}

// assertClosedToolAuthentication checks that closed authentication admission fails closed.
func assertClosedToolAuthentication(t *testing.T, client *wireClient, pool *executor.Pool, token string) {
	t.Helper()
	pool.Close()
	_, unavailable := client.post(token, map[string]any{"jsonrpc": "2.0", "id": 10, "method": "tools/list"})
	if unavailable["code"] != "service_unavailable" {
		t.Fatal("closed authentication pool did not fail closed", unavailable)
	}
}

// assertCurrentToolIdentity checks the request principal after administrator initialization.
func assertCurrentToolIdentity(t *testing.T, client *wireClient, token string) {
	t.Helper()
	readerFolders := client.call(token, "list_folders", map[string]any{"unknown": "accepted"})
	if text := toolText(t, readerFolders); !strings.Contains(text, "reader only") || strings.Contains(text, "admin only") {
		t.Fatal("session cached initialize identity")
	}
}
