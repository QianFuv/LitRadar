package textdecode

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"unicode/utf8"

	"golang.org/x/net/html"
)

func TestExhaustiveGb18030RustOracle(t *testing.T) {
	digest := sha256.New()
	digest.Write([]byte("LRGB1801"))
	verify := func(unit []byte) {
		decoded, err := Decode(unit, "gb18030")
		value := int32(-1)
		if err == nil {
			var size int
			value, size = utf8.DecodeRuneInString(decoded)
			if size != len(decoded) {
				t.Fatalf("unexpected multi-scalar unit %x", unit)
			}
		}
		var encoded [4]byte
		binary.LittleEndian.PutUint32(encoded[:], uint32(value))
		digest.Write(encoded[:])
	}
	for first := 0x81; first <= 0xfe; first++ {
		for second := 0x40; second <= 0xfe; second++ {
			if second != 0x7f {
				verify([]byte{byte(first), byte(second)})
			}
		}
	}
	for first := 0x81; first <= 0xfe; first++ {
		for second := 0x30; second <= 0x39; second++ {
			for third := 0x81; third <= 0xfe; third++ {
				for fourth := 0x30; fourth <= 0x39; fourth++ {
					verify([]byte{byte(first), byte(second), byte(third), byte(fourth)})
				}
			}
		}
	}
	if actual := hex.EncodeToString(digest.Sum(nil)); actual != "d930c412b9d64eae3b126448db4f0aa65d18b8e882f639423bb2219ff7a7e66f" {
		t.Fatalf("Rust exhaustive unit oracle mismatch: %s", actual)
	}
	for _, scenario := range []struct {
		raw      []byte
		expected string
	}{
		{[]byte{0xa6, 0xdd}, "︔"},
		{[]byte{0xfe, 0x7e}, "龹"},
		{[]byte{0x81, 0x35, 0xf4, 0x37}, "\ue7c7"},
	} {
		if actual, err := Decode(scenario.raw, "gbk"); err != nil || actual != scenario.expected {
			t.Fatalf("GBK correction %x: %q %v", scenario.raw, actual, err)
		}
		if _, err := Decode(append(append([]byte{}, scenario.raw...), 0x81), "gbk"); err == nil {
			t.Fatalf("accepted malformed suffix after correction %x", scenario.raw)
		}
	}
}

func TestFrozenRustStrictDecoding(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "..", "tests", "data", "migration", "encoding-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Label, InputHex string
			DecodedHex      *string
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range fixture.Cases {
		body, err := hex.DecodeString(scenario.InputHex)
		if err != nil {
			t.Fatal(err)
		}
		actual, err := Decode(body, scenario.Label)
		if scenario.DecodedHex == nil {
			if err == nil {
				t.Errorf("accepted invalid %s %s", scenario.Label, scenario.InputHex)
			}
			continue
		}
		if err != nil || hex.EncodeToString([]byte(actual)) != *scenario.DecodedHex {
			t.Errorf("%s %s: %x %v", scenario.Label, scenario.InputHex, actual, err)
		}
	}
}

func TestHttpCharsetPrecedenceAndUtf8Dom(t *testing.T) {
	body, _ := hex.DecodeString("3c6d65746120636861727365743d2767626b273e3c68313ed6d0cec43c2f68313e")
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html")
		writer.Write(body)
	}))
	defer server.Close()
	response, err := http.Get(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	raw, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := Html(raw, response.Header.Get("Content-Type"))
	if err != nil {
		t.Fatal(err)
	}
	document, err := html.Parse(strings.NewReader(decoded))
	if err != nil {
		t.Fatal(err)
	}
	found := false
	var visit func(*html.Node)
	visit = func(node *html.Node) {
		if node.Type == html.TextNode && node.Data == "中文" {
			found = true
		}
		for child := node.FirstChild; child != nil; child = child.NextSibling {
			visit(child)
		}
	}
	visit(document)
	if !found {
		t.Fatal("decoded Chinese text missing from DOM")
	}
	if actual, err := Html(append([]byte{0xef, 0xbb, 0xbf}, []byte("中文�")...), "text/html;charset=unknown"); err != nil || actual != "中文�" {
		t.Fatalf("BOM precedence %q %v", actual, err)
	}
	if _, err := Html(body, "text/html;charset=utf-8"); err == nil {
		t.Fatal("HTTP charset did not override meta")
	}
	if _, err := Html([]byte("plain"), "text/plain;charset=unknown"); err == nil {
		t.Fatal("unknown charset accepted")
	}
}
