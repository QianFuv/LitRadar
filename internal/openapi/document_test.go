package openapi

import (
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

func TestDocumentRetainsFrozenContractAndRejectsMissingBindings(t *testing.T) {
	data, err := os.ReadFile("../../tests/data/migration/rust/openapi.json")
	if err != nil {
		t.Fatal(err)
	}
	var expected map[string]any
	if err := json.Unmarshal(data, &expected); err != nil {
		t.Fatal(err)
	}
	var inventory struct {
		Operations []struct{ Method, Path, OperationId string }
	}
	data, err = os.ReadFile("../../tests/data/migration/inventory.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &inventory); err != nil {
		t.Fatal(err)
	}
	bindings := make([]Operation, 0, len(inventory.Operations))
	for _, operation := range inventory.Operations {
		bindings = append(bindings, Operation{operation.Method, operation.Path, operation.OperationId})
	}
	if len(bindings) != 86 {
		t.Fatal("operation inventory changed")
	}
	data, err = Generate(bindings)
	if err != nil {
		t.Fatal(err)
	}
	var actual map[string]any
	if err := json.Unmarshal(data, &actual); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(actual, expected) {
		t.Fatal("Go schema declarations changed public contract")
	}
	if len(actual["paths"].(map[string]any)) != 71 || len(actual["components"].(map[string]any)["schemas"].(map[string]any)) != 121 {
		t.Fatal("schema inventory changed")
	}
	if _, err := Generate(bindings[1:]); err == nil {
		t.Fatal("missing runtime route accepted")
	}
	if _, err := Generate(append(bindings, bindings[0])); err == nil {
		t.Fatal("duplicate runtime route accepted")
	}
	extra := append(append([]Operation(nil), bindings...), Operation{"POST", "/api/undocumented", "extra"})
	if _, err := Generate(extra); err == nil {
		t.Fatal("undocumented runtime route accepted")
	}
	bindings[0].Id = "wrong"
	if _, err := Generate(bindings); err == nil {
		t.Fatal("misbound runtime route accepted")
	}
}
