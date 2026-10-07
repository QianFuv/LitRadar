package cfp

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/process"
)

func TestMain(tests *testing.M) {
	if mode := os.Getenv("LITRADAR_CFP_HELPER_FIXTURE"); mode != "" {
		runHelperFixture(mode)
		os.Exit(0)
	}
	os.Exit(tests.Run())
}

func runHelperFixture(mode string) {
	if mode == "descendant" {
		runDescendantFixture()
	}
	output := helperFixtureOutput()
	if mode == "nonzero" {
		os.Exit(9)
	}
	if mode == "missing" {
		return
	}
	runBlockingFixture(mode, output)
	if mode == "leader-exit" {
		startFixtureDescendant()
	}
	if mode == "flood" {
		for index := 0; index < 128; index++ {
			fmt.Fprint(os.Stdout, strings.Repeat("o", 8192))
			fmt.Fprint(os.Stderr, strings.Repeat("e", 8192))
		}
	}
	data := []byte(os.Getenv("LITRADAR_CFP_OUTPUT"))
	if mode == "invalid-utf8" {
		data = []byte{0xff}
	}
	if err := os.WriteFile(output, data, 0600); err != nil {
		os.Exit(38)
	}
}

func helperExecutable(t *testing.T, mode, output string) string {
	t.Helper()
	t.Setenv("LITRADAR_CFP_HELPER_FIXTURE", mode)
	t.Setenv("LITRADAR_CFP_OUTPUT", output)
	t.Setenv("GORACE", "atexit_sleep_ms=0")
	path, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	return path
}

func TestHelperLifecycleAndWholeTreeOwnership(t *testing.T) {
	for _, scenario := range []struct {
		mode     string
		expected error
	}{
		{"success", nil}, {"flood", nil}, {"leader-exit", nil}, {"missing", ErrHelper}, {"nonzero", ErrHelper}, {"overflow", ErrTooLarge}, {"blocked", ErrDeadline}, {"cancelled", ErrCancelled},
	} {
		t.Run(scenario.mode, func(t *testing.T) {
			directory := t.TempDir()
			output := filepath.Join(directory, "result.txt")
			marker := filepath.Join(directory, "descendant.txt")
			t.Setenv("LITRADAR_CFP_MARKER", marker)
			mode := scenario.mode
			if mode == "cancelled" {
				mode = "blocked"
			}
			path := helperExecutable(t, mode, "complete original text")
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			deadline := time.Now().Add(10 * time.Second)
			if scenario.mode == "blocked" {
				deadline = time.Now().Add(150 * time.Millisecond)
			}
			if scenario.mode == "cancelled" {
				timer := time.AfterFunc(150*time.Millisecond, cancel)
				defer timer.Stop()
			}
			data, err := runHelper(ctx, process.Config{Path: path, Args: []string{"fixture", output}}, deadline, output)
			if !errors.Is(err, scenario.expected) {
				t.Fatalf("error=%v expected=%v", err, scenario.expected)
			}
			if err == nil && string(data) != "complete original text" {
				t.Fatal("helper output lost")
			}
			if scenario.mode == "leader-exit" {
				address, err := os.ReadFile(marker)
				if err != nil {
					t.Fatal(err)
				}
				connection, err := net.DialTimeout("tcp", string(address), time.Second)
				if err == nil {
					connection.Close()
					t.Fatal("successful leader left a live descendant")
				}
			}
		})
	}
}

func TestObscuraProtocolAndOneAttempt(t *testing.T) {
	config := SourceConfig{AllowedUrls: []UrlRule{{Host: "link.springer.com", PathPrefix: "/collections"}}}
	valid := `{"protocol":"litradar.cfp.page.v1","finalUrl":"https://link.springer.com/collections/a","html":"<main>complete text</main>"}`
	for _, payload := range []string{valid, `["litradar.cfp.page.v1","https://link.springer.com/collections/a","<main>complete text</main>"]`} {
		if _, err := decodeObscura(config, []byte(payload)); err != nil {
			t.Fatal(err)
		}
	}
	for _, payload := range []string{`[null,"https://link.springer.com/collections/a","x"]`, `["litradar.cfp.page.v1",null,"x"]`, `["litradar.cfp.page.v1","https://link.springer.com/collections/a",null]`, valid + "{}", strings.Replace(valid, `"html":`, `"unknown":`, 1), strings.Replace(valid, `"html":`, `"protocol":"duplicate","html":`, 1)} {
		if _, err := decodeObscura(config, []byte(payload)); !errors.Is(err, ErrHelper) {
			t.Fatalf("%s: %v", payload, err)
		}
	}
	t.Setenv("OBSCURA_ALLOW_PRIVATE_NETWORK", "1")
	path := helperExecutable(t, "success", valid)
	transport := NewLiveTransport(RefreshOptions{ObscuraPath: &path})
	defer transport.Close()
	if document, err := transport.obscuraDocument(context.Background(), config, "https://link.springer.com/collections/a", time.Now().Add(10*time.Second)); err != nil || document.Text != "<main>complete text</main>" {
		t.Fatal(document, err)
	}
	if _, err := transport.obscuraDocument(context.Background(), config, "https://link.springer.com/collections/a", time.Now().Add(10*time.Second)); !errors.Is(err, ErrHelper) {
		t.Fatal("rendered fallback reused", err)
	}
}

func TestPdfHelperFaithfulTextAndArguments(t *testing.T) {
	for _, scenario := range []struct {
		mode, text string
		expected   error
	}{{"success", "first line\nsecond line\n", nil}, {"success", " \n", ErrUnrecognized}, {"invalid-utf8", "", ErrEncoding}} {
		t.Run(scenario.mode+scenario.text, func(t *testing.T) {
			path := helperExecutable(t, scenario.mode, scenario.text)
			httpTransport, config := publicFixture(t, func(writer http.ResponseWriter, request *http.Request) { writer.Write([]byte("%PDF-fixture")) })
			transport := &LiveTransport{http: httpTransport, options: RefreshOptions{PdftotextPath: &path}}
			document, err := transport.httpDocument(context.Background(), config, config.DiscoveryUrl, time.Now().Add(10*time.Second))
			if !errors.Is(err, scenario.expected) {
				t.Fatal(err)
			}
			if err == nil && (document.Format != "pdf_text" || document.Text != scenario.text) {
				t.Fatal("PDF text modified", document)
			}
		})
	}
}

func TestCaptureResumeSkipsMalformedEntriesAndKeepsIdentity(t *testing.T) {
	path := filepath.Join(t.TempDir(), "capture.json")
	data := `{"result":{"sourceKey":"journal:a"},"documents":[1,null,{"requestedUrl":"https://old.example/cfp","url":"https://link.springer.com/collections/a","text":"original","format":"html"}],"browserAttempts":[null,4,"https://other.example/cfp"]}`
	if err := os.WriteFile(path, []byte(data), 0600); err != nil {
		t.Fatal(err)
	}
	cache := newCaptureCache()
	if err := cache.resume(path, "journal:a"); err != nil {
		t.Fatal(err)
	}
	if len(cache.documents) != 2 || cache.bytes != 8 || !cache.browserAttempts["https://old.example/cfp"] || !cache.browserAttempts["https://other.example/cfp"] {
		t.Fatalf("resume lost valid entries: %+v", cache)
	}
	if err := cache.resume(path, "journal:b"); err == nil {
		t.Fatal("cross-source resume accepted")
	}
	assertResumeEnvelopeIdentity(t, path)
	var envelope map[string]any
	if json.Unmarshal([]byte(data), &envelope) != nil {
		t.Fatal("fixture JSON")
	}
}

// runDescendantFixture exposes a readiness marker before waiting for parent tree cleanup.
func runDescendantFixture() {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		os.Exit(30)
	}
	os.WriteFile(os.Getenv("LITRADAR_CFP_MARKER"), []byte(listener.Addr().String()), 0600)
	for {
		time.Sleep(time.Second)
	}
}

// helperFixtureOutput verifies browser and PDF invocation arguments before selecting the output file.
func helperFixtureOutput() string {
	output := os.Args[len(os.Args)-1]
	if os.Args[1] == "fetch" {
		position := slices.Index(os.Args, "--output")
		if position < 0 {
			os.Exit(31)
		}
		output = os.Args[position+1]
		assertBrowserFixtureArguments()
	} else if os.Args[1] == "-enc" {
		assertPdfFixtureArguments()
	}
	return output
}

// runBlockingFixture preserves deadline and overflow fixture loops and file ownership.
func runBlockingFixture(mode, output string) {
	if mode == "blocked" {
		for {
			time.Sleep(time.Second)
		}
	}
	if mode == "overflow" {
		file, _ := os.Create(output)
		file.Truncate(MaxPageBytes + 1)
		file.Close()
		for {
			time.Sleep(time.Second)
		}
	}
}

// startFixtureDescendant preserves inherited streams and waits for the descendant readiness marker.
func startFixtureDescendant() {
	command := exec.Command(os.Args[0])
	command.Env = []string{}
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "LITRADAR_CFP_HELPER_FIXTURE=") {
			command.Env = append(command.Env, entry)
		}
	}
	command.Env = append(command.Env, "LITRADAR_CFP_HELPER_FIXTURE=descendant")
	command.Stdin, command.Stdout, command.Stderr = os.Stdin, os.Stdout, os.Stderr
	if command.Start() != nil {
		os.Exit(36)
	}
	deadline := time.Now().Add(5 * time.Second)
	for {
		if _, err := os.Stat(os.Getenv("LITRADAR_CFP_MARKER")); err == nil {
			break
		}
		if time.Now().After(deadline) {
			os.Exit(37)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// assertResumeEnvelopeIdentity rejects valid JSON envelopes without matching source identity.
func assertResumeEnvelopeIdentity(t *testing.T, path string) {
	t.Helper()
	for _, payload := range []string{`[]`, `1`, `null`, `"value"`} {
		os.WriteFile(path, []byte(payload), 0600)
		if err := newCaptureCache().resume(path, "journal:a"); err == nil || err.Error() != "Saved capture journal identity does not match" {
			t.Fatal(payload, err)
		}
	}
}

// assertBrowserFixtureArguments preserves private-network, stealth, evaluation and Springer wait checks.
func assertBrowserFixtureArguments() {
	if os.Getenv("OBSCURA_ALLOW_PRIVATE_NETWORK") != "" || !slices.Contains(os.Args, "--stealth") || !slices.Contains(os.Args, obscuraEval) {
		os.Exit(32)
	}
	if strings.Contains(os.Args[2], "link.springer.com") && !slices.Contains(os.Args, "domcontentloaded") {
		os.Exit(33)
	}
}

// assertPdfFixtureArguments preserves exact converter flags and input signature checks.
func assertPdfFixtureArguments() {
	if len(os.Args) != 8 || os.Args[2] != "UTF-8" || os.Args[3] != "-eol" || os.Args[4] != "unix" || os.Args[5] != "-nopgbrk" {
		os.Exit(34)
	}
	data, err := os.ReadFile(os.Args[6])
	if err != nil || !strings.HasPrefix(string(data), "%PDF-") {
		os.Exit(35)
	}
}
