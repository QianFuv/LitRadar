package textdecode

import (
	"encoding/hex"

	"io"
	"net/http"
	"net/http/httptest"

	"strings"
	"testing"

	"golang.org/x/net/html"
)

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
