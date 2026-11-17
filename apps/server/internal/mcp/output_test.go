package mcp

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	sdk "github.com/modelcontextprotocol/go-sdk/mcp"
)

// TestAmbiguousDatabaseToolErrorListsSelectableStems checks the actual tool error path.
func TestAmbiguousDatabaseToolErrorListsSelectableStems(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"beta.sqlite", "alpha.sqlite", "gamma.sqlite"} {
		if err := os.WriteFile(filepath.Join(configuration.IndexDir, name), nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
	pool := executor.New(1, time.Second)
	defer pool.Wait()
	services := Services{Storage: configuration, Pool: pool}
	for _, name := range []string{"list_areas", "search_articles", "get_article"} {
		input := &toolInput{fields: map[string]any{"article_id": "1"}}
		result, err := services.call(context.Background(), nil, name, input)
		if err != nil {
			t.Fatal(err)
		}
		if !result.IsError || result.Content[0].(*sdk.TextContent).Text != "Multiple databases found; set db to one of: alpha, beta, gamma" {
			t.Fatalf("tool %s: %#v", name, result)
		}
	}
}
