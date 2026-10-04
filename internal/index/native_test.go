package index

import (
	"crypto/sha256"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestIndexRuntimeTokenizerIdentity(t *testing.T) {
	actual, err := sqlite.SimpleLibrary()
	if err != nil {
		t.Fatal(err)
	}
	name := "libsimple.so"
	if runtime.GOOS == "windows" {
		name = "simple.dll"
	}
	expected, err := filepath.Abs(filepath.Join("../../libs/simple", runtime.GOOS, name))
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Clean(actual) != filepath.Clean(expected) {
		t.Fatalf("runtime tokenizer must be covered by V06 input hashes: %s", actual)
	}
	body, err := os.ReadFile(actual)
	if err != nil {
		t.Fatal(err)
	}
	t.Logf("tokenizer_identity path=%s sha256=%x", actual, sha256.Sum256(body))
}
