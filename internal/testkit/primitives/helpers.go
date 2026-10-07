package primitives

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strings"
	"time"
	"unicode/utf16"

	"github.com/QianFuv/LitRadar/internal/platform/process"
)

func helperCommand(ctx context.Context, executable string, args []string, input string, allowPrivate bool) (string, string, error) {
	environment := []string{}
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "OBSCURA_ALLOW_PRIVATE_NETWORK=") {
			environment = append(environment, entry)
		}
	}
	if allowPrivate {
		environment = append(environment, "OBSCURA_ALLOW_PRIVATE_NETWORK=1")
	}
	child, err := process.Start(ctx, process.Config{Path: executable, Args: args, Environment: environment, OutputLimit: 1024 * 1024})
	if err != nil {
		return "", "", err
	}
	defer child.Close()
	written := make(chan error, 1)
	go func() { _, err := io.WriteString(child.Stdin, input); child.Stdin.Close(); written <- err }()
	waitError := child.Wait(ctx)
	closeError := child.Close()
	writeError := <-written
	output, truncated := child.Output()
	diagnostics, diagnosticsTruncated := child.Diagnostics()
	if truncated || diagnosticsTruncated {
		return "", "", errors.New("helper exceeded fixture capture limit")
	}
	return string(output), string(diagnostics), errors.Join(waitError, closeError, writeError)
}

// helpers owns the local server and shared deadline through ordered native protocol checks.
func helpers() error {
	if err := verifyFixtureHardening(); err != nil {
		return err
	}
	listener, err := net.Listen("tcp", "127.0.0.1:8081")
	if err != nil {
		return err
	}
	server := &http.Server{Handler: http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html;charset=utf-8")
		io.WriteString(writer, `<!doctype html><html><body><div id="result">before</div><script>document.getElementById('result').textContent='rendered '+(2+2)</script></body></html>`)
	}), ReadHeaderTimeout: time.Second}
	go server.Serve(listener)
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	if err := verifyFixtureBrowserPolicy(ctx); err != nil {
		return err
	}
	if err := verifyFixtureRenderedPage(ctx); err != nil {
		return err
	}
	if err := verifyFixturePdf(ctx); err != nil {
		return err
	}
	fmt.Println(`{"uid":10001,"readOnly":true,"noCapabilities":true,"noNewPrivileges":true,"obscura":"0.2.2+litradar.1","privateNetworkDenied":true,"javascript":true,"originalHtml":true,"pdfCjk":true}`)
	return nil
}

func pdfFixture(text string) (string, error) {
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
	return document, nil
}

// verifyFixtureHardening requires the original nonroot, capability, privilege and read-only boundaries.
func verifyFixtureHardening() error {
	if os.Getuid() != 10001 {
		return errors.New("helper fixture is not nonroot uid10001")
	}
	status, err := os.ReadFile("/proc/self/status")
	if err != nil {
		return err
	}
	if !strings.Contains(string(status), "CapEff:\t0000000000000000") || !strings.Contains(string(status), "NoNewPrivs:\t1") {
		return errors.New("container hardening missing")
	}
	if err := os.WriteFile("/app/forbidden-write", nil, 0600); err == nil {
		return errors.New("root filesystem is writable")
	}
	return nil
}

// verifyFixtureBrowserPolicy checks the fixed browser version before explicit private-address denial.
func verifyFixtureBrowserPolicy(ctx context.Context) error {
	version, diagnostics, err := helperCommand(ctx, "/usr/local/bin/obscura", []string{"--version"}, "", false)
	if err != nil || strings.TrimSpace(version) != "obscura 0.2.2+litradar.1" {
		return fmt.Errorf("fixed browser version: %q %q %v", version, diagnostics, err)
	}
	_, diagnostics, err = helperCommand(ctx, "/usr/local/bin/obscura", []string{"fetch", "http://127.0.0.1:8081/", "--stealth", "--timeout", "5", "--quiet"}, "", false)
	if err == nil || !strings.Contains(diagnostics, "Access to private/internal IP address 127.0.0.1 is not allowed") {
		return fmt.Errorf("private address policy failed: %q %v", diagnostics, err)
	}
	return nil
}

// verifyFixtureRenderedPage preserves exact invocation and original rendered-page protocol assertions.
func verifyFixtureRenderedPage(ctx context.Context) error {
	_, diagnostics, err := helperCommand(ctx, "/usr/local/bin/obscura", []string{"fetch", "http://127.0.0.1:8081/", "--stealth", "--timeout", "30", "--wait-until", "domcontentloaded", "--wait", "0", "--eval", "JSON.stringify({protocol:'litradar.cfp.page.v1',finalUrl:location.href,html:document.documentElement.outerHTML})", "--quiet", "--output", "/tmp/page.json"}, "", true)
	if err != nil {
		return fmt.Errorf("browser protocol: %s %w", diagnostics, err)
	}
	data, err := os.ReadFile("/tmp/page.json")
	if err != nil {
		return err
	}
	var page struct{ Protocol, FinalUrl, Html string }
	if err := json.Unmarshal(data, &page); err != nil {
		return err
	}
	if page.Protocol != "litradar.cfp.page.v1" || page.FinalUrl != "http://127.0.0.1:8081/" || !strings.Contains(page.Html, ">rendered 4<") {
		return fmt.Errorf("rendered HTML protocol mismatch: %s", data)
	}
	return nil
}

// verifyFixturePdf preserves CJK input, UTF8 extraction arguments and normalized original-text assertion.
func verifyFixturePdf(ctx context.Context) error {
	pdf, err := pdfFixture("LitRadar 征稿原文")
	if err != nil {
		return err
	}
	text, diagnostics, err := helperCommand(ctx, "/usr/bin/pdftotext", []string{"-enc", "UTF-8", "-eol", "unix", "-nopgbrk", "-", "-"}, pdf, false)
	if err != nil || strings.Join(strings.Fields(text), " ") != "LitRadar 征稿原文" {
		return fmt.Errorf("PDF original text: %q %q %v", text, diagnostics, err)
	}
	return nil
}
