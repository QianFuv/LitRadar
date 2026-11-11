package favorites

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/graphsdb"
)

var graphQueries graphsdb.Queries

// MaxGraphBytes is the maximum encoded size of a saved graph snapshot.
const MaxGraphBytes = 2 * 1024 * 1024

// GraphSummary describes an owned graph without loading its snapshot.
type GraphSummary struct {
	ID            int64       `json:"id" nullable:"false" format:"int64"`
	Title         string      `json:"title" nullable:"false"`
	DBName        string      `json:"db_name" nullable:"false"`
	RootArticleID identity.ID `json:"root_article_id" nullable:"false" pattern:"^[1-9][0-9]*$"`
	NodeCount     int64       `json:"node_count" nullable:"false" format:"int64"`
	CreatedAt     float64     `json:"created_at" nullable:"false" format:"double"`
	UpdatedAt     float64     `json:"updated_at" nullable:"false" format:"double"`
}

// Graph includes the original JSON object snapshot with its summary.
type Graph struct {
	GraphSummary
	Graph json.RawMessage `json:"graph" nullable:"false"`
}

// GraphCreate supplies the immutable source and initial graph snapshot.
type GraphCreate struct {
	Title         string
	DBName        string
	RootArticleID int64
	NodeCount     int64
	Graph         json.RawMessage
}

// GraphUpdate supplies a title, a complete replacement snapshot, or both.
type GraphUpdate struct {
	Title     *string
	NodeCount *int64
	Graph     json.RawMessage
}

// GraphPage contains summaries and an owner-bound continuation cursor.
type GraphPage struct {
	Items      []GraphSummary `json:"items" nullable:"false"`
	NextCursor *string        `json:"next_cursor" nullable:"true"`
}

func validateGraphTitle(title string) error {
	title = strings.TrimSpace(title)
	if title == "" || utf8.RuneCountInString(title) > 200 {
		return InvalidInput{"title must be 1-200 characters"}
	}
	return nil
}

func validateGraphSnapshot(graph json.RawMessage, count int64) error {
	if count < 1 || count > 500 {
		return InvalidInput{"node_count must be between 1 and 500"}
	}
	if len(graph) > MaxGraphBytes {
		return InvalidInput{"graph must be at most 2 MiB"}
	}
	trimmed := bytes.TrimSpace(graph)
	if len(trimmed) == 0 || trimmed[0] != '{' || !json.Valid(graph) {
		return InvalidInput{"graph must be a JSON object"}
	}
	return nil
}

// Validate checks graph creation before storage or executor admission.
func (value GraphCreate) Validate() error {
	if err := validateGraphTitle(value.Title); err != nil {
		return err
	}
	if err := characters("db_name", value.DBName, 255); err != nil {
		return err
	}
	if err := positive("root_article_id", value.RootArticleID); err != nil {
		return err
	}
	return validateGraphSnapshot(value.Graph, value.NodeCount)
}

// Validate requires a mutable field and keeps snapshot data and count together.
func (value GraphUpdate) Validate() error {
	if value.Title == nil && value.Graph == nil {
		return InvalidInput{"At least one of title or graph is required"}
	}
	if value.Title != nil {
		if err := validateGraphTitle(*value.Title); err != nil {
			return err
		}
	}
	if (value.Graph == nil) != (value.NodeCount == nil) {
		return InvalidInput{"node_count and graph must be provided together"}
	}
	if value.Graph != nil {
		return validateGraphSnapshot(value.Graph, *value.NodeCount)
	}
	return nil
}

const graphSummaryColumns = "id,title,db_name,root_article_id,node_count,created_at,updated_at"

func scanGraphSummary(row scanner) (GraphSummary, error) {
	var value GraphSummary
	err := row.Scan(&value.ID, &value.Title, &value.DBName, &value.RootArticleID, &value.NodeCount, &value.CreatedAt, &value.UpdatedAt)
	return value, err
}

func scanGraph(row scanner) (*Graph, error) {
	var value Graph
	var snapshot string
	err := row.Scan(&value.ID, &value.Title, &value.DBName, &value.RootArticleID, &value.NodeCount, &value.CreatedAt, &value.UpdatedAt, &snapshot)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	value.Graph = json.RawMessage(snapshot)
	return &value, nil
}

// CreateGraph stores a trimmed title and the verbatim encoded snapshot.
func (repository *Repository) CreateGraph(ctx context.Context, owner identity.ID, value GraphCreate) (*Graph, error) {
	if err := value.Validate(); err != nil {
		return nil, err
	}
	var result *Graph
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		now := nowSeconds()
		var err error
		row, lookup := graphQueries.CreateGraph(ctx, connection, graphsdb.CreateGraphParams{UserID: int64(owner), Title: strings.TrimSpace(value.Title), DBName: value.DBName, RootArticleID: value.RootArticleID, NodeCount: value.NodeCount, Graph: string(value.Graph), CreatedAt: now, UpdatedAt: now})
		result, err = decodeGraph(graphsdb.GraphRow(row), lookup)
		return err
	})
	return result, err
}

// GetGraph returns nil for absent graphs and graphs belonging to another owner.
func (repository *Repository) GetGraph(ctx context.Context, owner identity.ID, id int64) (*Graph, error) {
	var result *Graph
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		result, err = decodeGraph(graphQueries.Graph(ctx, connection, graphsdb.GraphParams{UserID: int64(owner), ID: id}))
		return err
	})
	return result, err
}

// ListGraphPage orders summaries by descending update time and row ID.
func (repository *Repository) ListGraphPage(ctx context.Context, owner identity.ID, limit int64, cursor *string) (GraphPage, error) {
	result := GraphPage{Items: []GraphSummary{}}
	if limit < 1 || limit > 100 {
		return result, InvalidInput{"limit must be between 1 and 100"}
	}
	statement := "SELECT " + graphSummaryColumns + " FROM relation_graph_favorites WHERE user_id=?"
	values := []any{owner}
	if cursor != nil {
		stamp, id, err := decodeCursor(*cursor, owner, -1)
		if err != nil {
			return result, err
		}
		statement += " AND (updated_at,id)<(?,?)"
		values = append(values, stamp, id)
	}
	statement += " ORDER BY updated_at DESC,id DESC LIMIT ?"
	values = append(values, limit+1)
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) (resultError error) {
		rows, err := connection.QueryContext(ctx, statement, values...)
		if err != nil {
			return err
		}
		defer func() {
			if closeError := rows.Close(); closeError != nil {
				resultError = errors.Join(resultError, fmt.Errorf("close rows: %w", closeError))
			}
		}()
		for rows.Next() {
			value, err := scanGraphSummary(rows)
			if err != nil {
				return err
			}
			result.Items = append(result.Items, value)
		}
		return rows.Err()
	})
	if int64(len(result.Items)) > limit {
		result.Items = result.Items[:limit]
		last := result.Items[len(result.Items)-1]
		cursor := encodeCursor(owner, Favorite{ID: last.ID, FolderID: -1, CreatedAt: last.UpdatedAt})
		result.NextCursor = &cursor
	}
	return result, err
}

// UpdateGraph atomically replaces supplied fields and returns the updated owned graph.
func (repository *Repository) UpdateGraph(ctx context.Context, owner identity.ID, id int64, value GraphUpdate) (*Graph, error) {
	if err := value.Validate(); err != nil {
		return nil, err
	}
	assignments := []string{"updated_at=?"}
	values := []any{nowSeconds()}
	if value.Title != nil {
		assignments = append(assignments, "title=?")
		values = append(values, strings.TrimSpace(*value.Title))
	}
	if value.Graph != nil {
		assignments = append(assignments, "graph=?", "node_count=?")
		values = append(values, string(value.Graph), *value.NodeCount)
	}
	values = append(values, owner, id)
	var result *Graph
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		result, err = scanGraph(connection.QueryRowContext(ctx, "UPDATE relation_graph_favorites SET "+strings.Join(assignments, ",")+" WHERE user_id=? AND id=? RETURNING "+graphSummaryColumns+",graph", values...))
		return err
	})
	return result, err
}

// DeleteGraph removes only the owner's graph and reports whether a row existed.
func (repository *Repository) DeleteGraph(ctx context.Context, owner identity.ID, id int64) (bool, error) {
	var changed bool
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		result, err := graphQueries.DeleteGraph(ctx, connection, graphsdb.DeleteGraphParams{UserID: int64(owner), ID: id})
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		changed = count > 0
		return err
	})
	return changed, err
}

func decodeGraph(row graphsdb.GraphRow, err error) (*Graph, error) {
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &Graph{GraphSummary: GraphSummary{row.ID, row.Title, row.DBName, identity.ID(row.RootArticleID), row.NodeCount, row.CreatedAt, row.UpdatedAt}, Graph: json.RawMessage(row.Graph)}, nil
}
