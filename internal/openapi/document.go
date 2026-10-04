// Package openapi owns the public schema declarations and checks their runtime bindings.
package openapi

import (
	"bytes"
	_ "embed"
	"encoding/json"
	"fmt"
	"strings"
)

//go:embed definitions.json
var definitions []byte

//go:embed operations.json
var operations []byte

// Operation identifies a concrete registered handler and its stable public operation ID.
type Operation struct{ Method, Path, Id string }

type declaration struct {
	Method    string          `json:"method"`
	Path      string          `json:"path"`
	Operation json.RawMessage `json:"operation"`
}

// Generate emits the complete public document only when every declared operation
// has exactly one matching live binding and no undocumented operation is supplied.
// Declarations are Go-owned inputs; frozen migration expectations are never loaded.
func Generate(bindings []Operation) ([]byte, error) {
	var metadata map[string]json.RawMessage
	if err := json.Unmarshal(definitions, &metadata); err != nil {
		return nil, err
	}
	var declared []declaration
	if err := json.Unmarshal(operations, &declared); err != nil {
		return nil, err
	}
	remaining := make(map[string]Operation, len(bindings))
	for _, binding := range bindings {
		key := binding.Method + " " + binding.Path
		if _, exists := remaining[key]; exists {
			return nil, fmt.Errorf("duplicate API binding: %s", key)
		}
		remaining[key] = binding
	}
	paths := make(map[string]map[string]json.RawMessage)
	for _, item := range declared {
		key := item.Method + " " + item.Path
		binding, exists := remaining[key]
		if !exists {
			return nil, fmt.Errorf("missing API binding: %s", key)
		}
		var identity struct {
			OperationId string `json:"operationId"`
		}
		if err := json.Unmarshal(item.Operation, &identity); err != nil {
			return nil, err
		}
		if binding.Id != identity.OperationId {
			return nil, fmt.Errorf("API binding identity mismatch: %s", key)
		}
		delete(remaining, key)
		if paths[item.Path] == nil {
			paths[item.Path] = make(map[string]json.RawMessage)
		}
		paths[item.Path][strings.ToLower(item.Method)] = item.Operation
	}
	if len(remaining) != 0 {
		return nil, fmt.Errorf("undocumented API bindings: %d", len(remaining))
	}
	encodedPaths, err := json.Marshal(paths)
	if err != nil {
		return nil, err
	}
	metadata["paths"] = encodedPaths
	var output bytes.Buffer
	encoder := json.NewEncoder(&output)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(metadata); err != nil {
		return nil, err
	}
	return output.Bytes(), nil
}
