package search

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"unicode"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

func TestFrozenRustSearchAndAuthors(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "..", "tests", "migration", "storage", "search-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		ScalarBlocks []struct {
			Start  rune
			Count  int
			Sha256 string
		} `json:"scalar_blocks"`
		Cases []struct {
			Kind, Input, Mode, Text, Query string
			UsesSimple                     bool `json:"uses_simple"`
			Valid                          bool
			Output                         []string
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, block := range fixture.ScalarBlocks {
		t.Run(fmt.Sprintf("Unicode/U+%04X", block.Start), func(t *testing.T) {
			hash := sha256.New()
			count := 0
			for character := block.Start; character < block.Start+4096; character++ {
				if !utf8.ValidRune(character) {
					continue
				}
				actual := PrepareText(string(character), true)
				var size [4]byte
				binary.LittleEndian.PutUint32(size[:], uint32(len(actual)))
				hash.Write(size[:])
				hash.Write([]byte(actual))
				count++
			}
			if actual := hex.EncodeToString(hash.Sum(nil)); actual != block.Sha256 || count != block.Count {
				t.Fatalf("Unicode %s projection mismatch: count=%d sha=%s", unicode.Version, count, actual)
			}
		})
	}
	for index, scenario := range fixture.Cases {
		t.Run(fmt.Sprintf("%s/%d", scenario.Kind, index), func(t *testing.T) {
			if scenario.Kind == "authors" {
				actual, err := DecodeAuthorNames(scenario.Input)
				if (err == nil) != scenario.Valid || scenario.Valid && !reflect.DeepEqual(actual, scenario.Output) {
					t.Fatalf("Rust author mismatch: %q: %#v %v; expected %#v valid=%t", scenario.Input, actual, err, scenario.Output, scenario.Valid)
				}
			} else {
				if actual := PrepareText(scenario.Input, scenario.UsesSimple); actual != scenario.Text {
					t.Fatalf("text %q: %q != %q", scenario.Input, actual, scenario.Text)
				}
				if actual := PrepareQuery(scenario.Input, scenario.UsesSimple, domain.SearchMode(scenario.Mode)); actual != scenario.Query {
					t.Fatalf("query %q: %q != %q", scenario.Input, actual, scenario.Query)
				}
			}
		})
	}
}
