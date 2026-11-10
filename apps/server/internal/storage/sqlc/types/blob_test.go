package types

import "testing"

// TestBlobPreservesNullAndEmptyStorageValues checks storage classes and driver buffer ownership.
func TestBlobPreservesNullAndEmptyStorageValues(t *testing.T) {
	var value Blob
	if err := value.Scan(nil); err != nil || value != nil {
		t.Fatalf("NULL became a blob: %v %v", value, err)
	}
	if err := value.Scan([]byte{}); err != nil || value == nil || len(value) != 0 {
		t.Fatalf("empty blob became NULL: %v %v", value, err)
	}
	source := []byte{1, 2}
	if err := value.Scan(source); err != nil {
		t.Fatal(err)
	}
	source[0] = 3
	if value[0] != 1 {
		t.Fatal("stored blob retained the driver's mutable buffer")
	}
	for _, invalid := range []any{"text", int64(1), 1.5} {
		if err := value.Scan(invalid); err == nil {
			t.Fatalf("accepted a non-BLOB storage value: %T", invalid)
		}
	}
}
