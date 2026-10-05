package scholarly

import (
	"testing"
)

func TestCrossrefMetadataOwnsMutableInputs(t *testing.T) {
	initial := map[string]any{"title": []any{"original"}, "author": []any{map[string]any{"given": "A"}}}
	retained := consumedPayload(initial)
	retained["title"].([]any)[0] = "changed"
	retained["author"].([]any)[0].(map[string]any)["given"] = "changed"
	if initial["title"].([]any)[0] != "original" || initial["author"].([]any)[0].(map[string]any)["given"] != "A" {
		t.Fatal("retained payload aliases caller")
	}
	state, err := NewCrossrefCheckpoint("1234-5679", 10, nil)
	if err != nil {
		t.Fatal(err)
	}
	state.CreatedFrom = clonePointer(new(int64(1)))
	state.Phase = CrossrefPhase{Kind: "collect", Partition: 1, From: 1, Until: 10, Cursor: clonePointer(new("cursor"))}
	copied := state.Clone()
	*copied.CreatedFrom = 2
	*copied.Phase.Cursor = "other"
	if *state.CreatedFrom != 1 || *state.Phase.Cursor != "cursor" {
		t.Fatal("checkpoint aliases caller")
	}
}
