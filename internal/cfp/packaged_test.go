package cfp

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
	"unicode/utf16"

	"github.com/QianFuv/LitRadar/internal/platform/process"
)

func TestRealPackagedHelpers(t *testing.T) {
	if os.Getenv("LITRADAR_CFP_PACKAGED") != "1" {
		t.Skip("CFP live runner supplies the pinned hardened helper image")
	}
	assertPackagedHardening(t)
	directory := t.TempDir()
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		fmt.Fprint(writer, `<html><body><main id="result">before</main><script>document.getElementById('result').textContent='original rendered '+(2+2)</script></body></html>`)
	}))
	defer server.Close()
	config := SourceConfig{AllowedUrls: []UrlRule{{Host: "127.0.0.1", PathPrefix: "/"}}}
	output := filepath.Join(directory, "page.json")
	args := []string{"fetch", server.URL + "/", "--stealth", "--timeout", "30", "--wait-until", "domcontentloaded", "--wait", "0", "--eval", obscuraEval, "--quiet", "--output", output}
	data, err := runHelper(context.Background(), process.Config{Path: "/usr/local/bin/obscura", Args: args, Environment: append(os.Environ(), "OBSCURA_ALLOW_PRIVATE_NETWORK=1")}, time.Now().Add(45*time.Second), output)
	if err != nil {
		t.Fatal("real rendered helper protocol failed", err)
	}
	var page struct{ Html string }
	if err := json.Unmarshal(data, &page); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(page.Html, ">original rendered 4<") {
		t.Fatal("JavaScript or original HTML lost")
	}
	t.Setenv("OBSCURA_ALLOW_PRIVATE_NETWORK", "1")
	transport := NewLiveTransport(DefaultRefreshOptions())
	defer transport.Close()
	if _, err = transport.obscuraDocument(context.Background(), config, server.URL+"/", time.Now().Add(10*time.Second)); !errors.Is(err, ErrHelper) {
		t.Fatal("production adapter enabled private-network fixture override", err)
	}
	assertPackagedPdfExtraction(t, transport)
}

func syntheticPdf(text string) string {
	var encoded []byte
	for _, value := range utf16.Encode([]rune(text)) {
		encoded = append(encoded, byte(value>>8), byte(value))
	}
	stream := "BT /F1 12 Tf 72 720 Td <" + hex.EncodeToString(encoded) + "> Tj ET\n"
	objects := []string{
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
		"<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [6 0 R] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(stream), stream),
		"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> /FontDescriptor 7 0 R /DW 1000 >>",
		"<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>",
	}
	document := "%PDF-1.4\n"
	offsets := []string{"0000000000 65535 f \n"}
	for index, object := range objects {
		offsets = append(offsets, fmt.Sprintf("%010d 00000 n \n", len(document)))
		document += fmt.Sprintf("%d 0 obj\n%s\nendobj\n", index+1, object)
	}
	start := len(document)
	document += fmt.Sprintf("xref\n0 %d\n%strailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(offsets), strings.Join(offsets, ""), len(offsets), start)
	return document
}

// assertPackagedHardening requires the pinned nonroot identity, dropped privileges and read-only root.
func assertPackagedHardening(t *testing.T) {
	t.Helper()
	if os.Getuid() != 10001 {
		t.Fatal("nonroot identity changed")
	}
	status, err := os.ReadFile("/proc/self/status")
	if err != nil || !strings.Contains(string(status), "CapEff:\t0000000000000000") || !strings.Contains(string(status), "NoNewPrivs:\t1") {
		t.Fatal("hardening missing", err)
	}
	if err := os.WriteFile("/app/cfp-forbidden-write", nil, 0600); err == nil {
		t.Fatal("root filesystem is writable")
	}
}

// assertPackagedPdfExtraction replaces the fixture HTTP transport and verifies the original Unicode PDF text.
func assertPackagedPdfExtraction(t *testing.T, transport *LiveTransport) {
	t.Helper()
	pdf := syntheticPdf("LitRadar 征稿原文")
	httpTransport, publicConfig := publicFixture(t, func(writer http.ResponseWriter, request *http.Request) { writer.Write([]byte(pdf)) })
	transport.http.Close()
	transport.http = httpTransport
	document, err := transport.httpDocument(context.Background(), publicConfig, publicConfig.DiscoveryUrl, time.Now().Add(15*time.Second))
	if err != nil || document.Format != "pdf_text" || strings.Join(strings.Fields(document.Text), " ") != "LitRadar 征稿原文" {
		t.Fatal("real Poppler UTF-8 extraction failed", document, err)
	}
}
