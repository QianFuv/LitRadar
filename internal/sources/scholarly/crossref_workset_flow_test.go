package scholarly

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/transport"
)

type worksetFlowAction struct {
	Kind      string          `json:"kind"`
	Page      json.RawMessage `json:"page"`
	Upper     string          `json:"upper"`
	Lower     *string         `json:"lower"`
	Candidate *Anchor         `json:"candidate"`
	After     *EmissionKey    `json:"after"`
	Index     int             `json:"index"`
}

func worksetSnapshot(t *testing.T, workset *CrossrefWorkset) any {
	t.Helper()
	state, err := workset.state.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	var previous storage.OptionalText
	if err := workset.row("SELECT previous FROM metadata WHERE id=1").Scan(&previous); err != nil {
		t.Fatal(err)
	}
	counts := []int{}
	for _, table := range []string{"partitions", "works", "groups"} {
		counts = append(counts, countWorksetRows(t, workset, table))
	}
	return map[string]any{"state": string(state), "counts": counts, "previous": previous.Value}
}

func TestOriginalCrossrefWorksetFlows(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/workset-flow-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct{ Id, Input, Output string } `json:"observations"`
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, observation := range corpus.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			var input struct {
				Initial CrossrefCheckpoint  `json:"initial"`
				Actions []worksetFlowAction `json:"actions"`
			}
			if err := json.Unmarshal([]byte(observation.Input), &input); err != nil {
				t.Fatal(err)
			}
			workset := createTestWorkset(t, input.Initial)
			defer func() { workset.Close() }()
			root := workset.root
			states := []CrossrefCheckpoint{input.Initial}
			events := []any{map[string]any{"snapshot": worksetSnapshot(t, workset), "replay": false}}
			for _, action := range input.Actions {
				event := map[string]any{}
				var operationErr error
				switch action.Kind {
				case "accept":
					var raw struct {
						Items  []json.RawMessage `json:"items"`
						Total  uint64            `json:"total_results"`
						Cursor *string           `json:"next_cursor"`
					}
					if err := json.Unmarshal(action.Page, &raw); err != nil {
						t.Fatal(err)
					}
					page := CrossrefPage{TotalResults: raw.Total, NextCursor: raw.Cursor}
					for _, body := range raw.Items {
						work, err := transport.ParseJson(body)
						if err != nil {
							t.Fatal(err)
						}
						page.Items = append(page.Items, work)
					}
					_, operationErr = workset.Accept(page)
					event["error"] = nil
				case "seal":
					_, operationErr = workset.SealSelection(action.Upper, action.Lower, action.Candidate)
					event["error"] = nil
				case "emit":
					var page EmissionPage
					page, operationErr = workset.Emit(action.Upper, action.Lower, action.After)
					if operationErr == nil {
						works := []string{}
						for _, work := range page.Works {
							body, err := domain.Json(work)
							if err != nil {
								t.Fatal(err)
							}
							works = append(works, string(body))
						}
						event["page"] = map[string]any{"works": works, "after": page.After, "has_more": page.HasMore}
					}
				case "groups":
					group, err := workset.FirstGroup()
					if err != nil {
						t.Fatal(err)
					}
					event["first"] = nil
					if group != nil {
						anchor, err := group.Anchor.MarshalJSON()
						if err != nil {
							t.Fatal(err)
						}
						event["first"] = map[string]any{"anchor": string(anchor), "date": group.Date}
					}
					unknown, err := workset.HasUnknownGroups()
					if err != nil {
						t.Fatal(err)
					}
					event["unknown"] = unknown
				case "reopen":
					if err := workset.Close(); err != nil {
						t.Fatal(err)
					}
					restored, replay, err := OpenCrossrefWorkset(root, "catalog", states[action.Index])
					if err != nil {
						t.Fatal(err)
					}
					workset = restored
					event["replay"] = replay
				default:
					t.Fatal(action.Kind)
				}
				if operationErr != nil {
					event["error"] = operationErr.Error()
				}
				states = append(states, workset.Checkpoint())
				event["snapshot"] = worksetSnapshot(t, workset)
				events = append(events, event)
			}
			encoded, err := json.Marshal(map[string]any{"events": events})
			if err != nil {
				t.Fatal(err)
			}
			var actual, expected any
			if json.Unmarshal(encoded, &actual) != nil || json.Unmarshal([]byte(observation.Output), &expected) != nil {
				t.Fatal("invalid observation JSON")
			}
			if !reflect.DeepEqual(actual, expected) {
				t.Fatalf("durable workflow mismatch\nwant %s\ngot %s", observation.Output, encoded)
			}
		})
	}
}

func runWorksetOracle(t *testing.T, root string, input any) map[string]any {
	t.Helper()
	encoded, err := json.Marshal(input)
	if err != nil {
		t.Fatal(err)
	}
	request, err := json.Marshal(map[string]any{"kind": "workflow", "root": root, "input": string(encoded)})
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	binary := "../../../output/migration/execution/sources-workset-oracle.exe"
	if _, err := os.Stat(binary); os.IsNotExist(err) {
		t.Skip("live Rust handoff requires the separately preserved migration oracle; the migration runner requires this test to pass")
	} else if err != nil {
		t.Fatal(err)
	}
	command := exec.CommandContext(ctx, binary)
	command.Stdin = bytes.NewReader(append(request, '\n'))
	output, err := command.CombinedOutput()
	if err != nil {
		t.Fatalf("oracle: %v %s", err, output)
	}
	var response struct {
		Output string `json:"output"`
	}
	if err := json.Unmarshal(output, &response); err != nil {
		t.Fatal(err, string(output))
	}
	var result map[string]any
	if err := json.Unmarshal([]byte(response.Output), &result); err != nil {
		t.Fatal(err)
	}
	return result
}

func TestCrossrefWorksetOriginalFileHandoff(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("original Rust oracle is a Windows executable; frozen workflows cover both platforms")
	}
	initial := worksetTestState()
	root := filepath.Join(t.TempDir(), "handoff")
	result := runWorksetOracle(t, root, map[string]any{"initial": initial, "actions": []any{map[string]any{"kind": "accept", "page": worksetTestPage(0, 1, 1, 1)}}})
	events := result["events"].([]any)
	encoded := events[1].(map[string]any)["snapshot"].(map[string]any)["state"].(string)
	var original CrossrefCheckpoint
	if err := json.Unmarshal([]byte(encoded), &original); err != nil {
		t.Fatal(err)
	}
	workset, replay, err := OpenCrossrefWorkset(root, "catalog", initial)
	if err != nil || !replay {
		t.Fatalf("Rust to Go: %v %v", replay, err)
	}
	if !reflect.DeepEqual(workset.Checkpoint(), original) {
		t.Fatal("Rust checkpoint changed on Go open")
	}
	sealed, err := workset.SealSelection("9999-12-31", nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := workset.Close(); err != nil {
		t.Fatal(err)
	}
	result = runWorksetOracle(t, root, map[string]any{"initial": original, "open": true, "actions": []any{map[string]any{"kind": "emit", "upper": "9999-12-31"}}})
	events = result["events"].([]any)
	first := events[0].(map[string]any)
	state, _ := sealed.MarshalJSON()
	if first["replay"] != true || first["snapshot"].(map[string]any)["state"] != string(state) {
		t.Fatal("Go to Rust seal replay changed checkpoint")
	}
	page := events[1].(map[string]any)["page"].(map[string]any)
	if len(page["works"].([]any)) != 1 {
		t.Fatal("Rust failed to emit Go-sealed data")
	}
}
