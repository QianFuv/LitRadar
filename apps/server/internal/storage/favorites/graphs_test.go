package favorites

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
)

func createTestGraph(t *testing.T, repository *Repository, owner identity.Id, title string) *Graph {
	t.Helper()
	value, err := repository.CreateGraph(context.Background(), owner, GraphCreate{Title: title, DbName: "source", RootArticleId: 42, NodeCount: 2, Graph: json.RawMessage(`{ "nodes": [1,2], "large": 9007199254740993 }`)})
	if err != nil {
		t.Fatal(err)
	}
	return value
}

// TestGraphLifecycle preserves source fields, raw snapshots and fractional timestamps.
func TestGraphLifecycle(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	created := createTestGraph(t, repository, owner, " first ")
	if created.Title != "first" || created.CreatedAt != created.UpdatedAt || math.Trunc(created.CreatedAt) == created.CreatedAt {
		t.Fatal(created)
	}
	loaded, err := repository.GetGraph(ctx, owner, created.Id)
	if err != nil || !reflect.DeepEqual(loaded, created) {
		t.Fatal(loaded, err)
	}
	runSql(t, repository, "UPDATE relation_graph_favorites SET updated_at=10.5")
	title := " renamed "
	renamed, err := repository.UpdateGraph(ctx, owner, created.Id, GraphUpdate{Title: &title})
	if err != nil || renamed.Title != "renamed" || string(renamed.Graph) != string(created.Graph) || renamed.NodeCount != 2 || renamed.UpdatedAt <= 10.5 || renamed.CreatedAt != created.CreatedAt {
		t.Fatal(renamed, err)
	}
	count := int64(1)
	snapshot := json.RawMessage(`{"nodes":[3],"nested":{"arbitrary":true}}`)
	updated, err := repository.UpdateGraph(ctx, owner, created.Id, GraphUpdate{NodeCount: &count, Graph: snapshot})
	if err != nil || updated.Title != "renamed" || updated.NodeCount != count || string(updated.Graph) != string(snapshot) || updated.DbName != "source" || updated.RootArticleId != 42 || updated.CreatedAt != created.CreatedAt || updated.UpdatedAt < renamed.UpdatedAt {
		t.Fatal(updated, err)
	}
	if changed, err := repository.DeleteGraph(ctx, owner, created.Id); err != nil || !changed {
		t.Fatal(changed, err)
	}
	if loaded, err := repository.GetGraph(ctx, owner, created.Id); err != nil || loaded != nil {
		t.Fatal(loaded, err)
	}
	if loaded, err := repository.UpdateGraph(ctx, owner, created.Id, GraphUpdate{Title: &title}); err != nil || loaded != nil {
		t.Fatal(loaded, err)
	}
	if changed, err := repository.DeleteGraph(ctx, owner, created.Id); err != nil || changed {
		t.Fatal(changed, err)
	}
}

// TestGraphPagination orders timestamp ties by ID and binds cursors to graph owners.
func TestGraphPagination(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	first := createTestGraph(t, repository, owner, "first")
	second := createTestGraph(t, repository, owner, "second")
	third := createTestGraph(t, repository, owner, "third")
	runSql(t, repository, "UPDATE relation_graph_favorites SET updated_at=12.5")
	runSql(t, repository, fmt.Sprintf("UPDATE relation_graph_favorites SET updated_at=13.25 WHERE id=%d", first.Id))
	var cursor *string
	for index, id := range []int64{first.Id, third.Id, second.Id} {
		page, err := repository.ListGraphPage(ctx, owner, 1, cursor)
		if err != nil || len(page.Items) != 1 || page.Items[0].Id != id || (page.NextCursor == nil) != (index == 2) {
			t.Fatal(page, err)
		}
		encoded, err := json.Marshal(page)
		if err != nil || strings.Contains(string(encoded), `"graph"`) {
			t.Fatal(string(encoded), err)
		}
		cursor = page.NextCursor
	}
	page, err := repository.ListGraphPage(ctx, owner, 1, nil)
	if err != nil {
		t.Fatal(err)
	}
	articleCursor := encodeCursor(owner, Favorite{Id: first.Id, FolderId: 1, CreatedAt: first.CreatedAt})
	for _, cursor := range []string{"", "bad", articleCursor, encodeCursor(owner+1, Favorite{Id: first.Id, FolderId: -1, CreatedAt: first.UpdatedAt})} {
		if _, err := repository.ListGraphPage(ctx, owner, 1, &cursor); !errors.Is(err, ErrCursor) {
			t.Fatal(cursor, err)
		}
	}
	if _, err := repository.ListGraphPage(ctx, owner+1, 1, page.NextCursor); !errors.Is(err, ErrCursor) {
		t.Fatal(err)
	}
	for _, limit := range []int64{0, 101} {
		if _, err := repository.ListGraphPage(ctx, owner, limit, nil); err == nil {
			t.Fatal("accepted limit", limit)
		}
	}
	page, err = repository.ListGraphPage(ctx, owner+1, 50, nil)
	if err != nil || page.Items == nil || len(page.Items) != 0 || page.NextCursor != nil {
		t.Fatal(page, err)
	}
}

// TestGraphOwnershipAndUserCascade hides foreign rows and removes snapshots with users.
func TestGraphOwnershipAndUserCascade(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	ctx := context.Background()
	runSql(t, repository, "INSERT INTO users(username,password_hash,salt,is_admin,created_at,updated_at) VALUES('other','hash','salt',0,1,1)")
	foreign := owner + 1
	graph := createTestGraph(t, repository, foreign, "foreign")
	title := "intrusion"
	for _, id := range []int64{graph.Id, graph.Id + 1000} {
		if value, err := repository.GetGraph(ctx, owner, id); err != nil || value != nil {
			t.Fatal(value, err)
		}
		if value, err := repository.UpdateGraph(ctx, owner, id, GraphUpdate{Title: &title}); err != nil || value != nil {
			t.Fatal(value, err)
		}
		if changed, err := repository.DeleteGraph(ctx, owner, id); err != nil || changed {
			t.Fatal(changed, err)
		}
	}
	if value, err := repository.GetGraph(ctx, foreign, graph.Id); err != nil || !reflect.DeepEqual(value, graph) {
		t.Fatal(value, err)
	}
	if err := repository.auth.DeleteUser(ctx, owner, foreign, nil); err != nil {
		t.Fatal(err)
	}
	if value, err := repository.GetGraph(ctx, foreign, graph.Id); err != nil || value != nil {
		t.Fatal(value, err)
	}
}

// TestGraphValidation rejects invalid snapshots and incomplete replacement requests.
func TestGraphValidation(t *testing.T) {
	repository, _, owner := fixtureRepository(t)
	valid := GraphCreate{Title: "title", DbName: "source", RootArticleId: 1, NodeCount: 1, Graph: json.RawMessage(`{}`)}
	for _, change := range []func(*GraphCreate){
		func(value *GraphCreate) { value.Title = " " },
		func(value *GraphCreate) { value.Title = strings.Repeat("文", 201) },
		func(value *GraphCreate) { value.DbName = strings.Repeat("文", 256) },
		func(value *GraphCreate) { value.RootArticleId = 0 },
		func(value *GraphCreate) { value.NodeCount = 0 },
		func(value *GraphCreate) { value.NodeCount = 501 },
		func(value *GraphCreate) { value.Graph = json.RawMessage(`[]`) },
		func(value *GraphCreate) { value.Graph = json.RawMessage(`null`) },
		func(value *GraphCreate) { value.Graph = json.RawMessage(`1`) },
		func(value *GraphCreate) { value.Graph = json.RawMessage(`{broken`) },
		func(value *GraphCreate) {
			value.Graph = json.RawMessage(`{"data":"` + strings.Repeat("x", MaxGraphBytes) + `"}`)
		},
	} {
		value := valid
		change(&value)
		if _, err := repository.CreateGraph(context.Background(), owner, value); err == nil {
			t.Fatal("accepted invalid graph")
		}
	}
	count := int64(1)
	for _, update := range []GraphUpdate{{}, {NodeCount: &count}, {Graph: json.RawMessage(`{}`)}} {
		if _, err := repository.UpdateGraph(context.Background(), owner, 1, update); err == nil {
			t.Fatal("accepted incomplete update", update)
		}
	}
	valid.Title = strings.Repeat("文", 200)
	valid.Graph = json.RawMessage(`{"data":"` + strings.Repeat("x", MaxGraphBytes-len(`{"data":""}`)) + `"}`)
	if _, err := repository.CreateGraph(context.Background(), owner, valid); err != nil {
		t.Fatal("rejected boundary graph", err)
	}
}
