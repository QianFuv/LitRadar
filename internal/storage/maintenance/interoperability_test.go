package maintenance

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"reflect"
	"strconv"
	"testing"
	"time"
)

func TestOriginalRustAndGoExchangeOptimizedDatabases(t *testing.T) {
	oracle := os.Getenv("LITRADAR_RUST_STORAGE_DATABASE_ORACLE")
	if oracle == "" {
		t.Skip("explicit interoperability runner supplies the original Rust oracle")
	}
	for _, version := range []int{6, 7, 8, 9} {
		t.Run(strconv.Itoa(version), func(t *testing.T) {
			configuration := fixture(t, version)
			ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
			defer cancel()
			first, err := Optimize(ctx, Options{configuration, true})
			if err != nil {
				t.Fatal(err)
			}
			request, err := json.Marshal(map[string]string{"operation": "optimize", "path": configuration.ProjectRoot})
			if err != nil {
				t.Fatal(err)
			}
			command := exec.CommandContext(ctx, oracle)
			command.Stdin = bytes.NewReader(append(request, '\n'))
			var diagnostics bytes.Buffer
			command.Stderr = &diagnostics
			output, err := command.Output()
			if err != nil {
				t.Fatalf("%v %s", err, diagnostics.String())
			}
			var result struct {
				Error  *string
				Output Report
			}
			if err := json.Unmarshal(output, &result); err != nil {
				t.Fatal(err)
			}
			if result.Error != nil {
				t.Fatal(*result.Error)
			}
			if result.Output.Outcome != "optimized" || len(result.Output.Databases) != 1 || !reflect.DeepEqual(result.Output.Databases[0].RowCounts, first.Databases[0].RowCounts) {
				t.Fatalf("%s", output)
			}
			last, err := Optimize(ctx, Options{configuration, true})
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(last.Databases[0].RowCounts, first.Databases[0].RowCounts) {
				t.Fatal("exchange changed authoritative membership")
			}
		})
	}
}
