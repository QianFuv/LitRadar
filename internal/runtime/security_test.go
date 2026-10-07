package runtime

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestCspHashesOriginalInlineBytesAndAttributeGrammar(t *testing.T) {
	first := "globalThis.first = '<tag>';"
	second := "globalThis.second = '中文';"
	html := "<SCRIPT data-src='ignored'>" + first + "</SCRIPT><script src='/external.js'>ignored</script><script>" + second + "</script><!-- <script>comment script</script> --><script title='src=x'>same</script><script ſrc='x'>same</script><script src>ignored</script>"
	hashes, err := inlineScriptHashes([]byte(html))
	if err != nil {
		t.Fatal(err)
	}
	expected := []string{cspDigest([]byte(first)), cspDigest([]byte(second)), cspDigest([]byte("comment script")), cspDigest([]byte("same")), cspDigest([]byte("same"))}
	if !reflect.DeepEqual(hashes, expected) {
		t.Fatal(hashes, expected)
	}
	for _, html := range []string{"<script title='unterminated>", "<script>not closed", "<script>text</script / >"} {
		if _, err := inlineScriptHashes([]byte(html)); err == nil {
			t.Fatal(html)
		}
	}
	if hashes, err := inlineScriptHashes([]byte("<scripture>ignore</scripture><script")); err != nil || len(hashes) != 0 {
		t.Fatal(hashes, err)
	}
}

func writeManifest(t *testing.T, root string) cspManifest {
	t.Helper()
	manifest, err := buildCspManifest(root)
	if err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(manifest)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "csp-hashes.json"), data, 0600); err != nil {
		t.Fatal(err)
	}
	return manifest
}

func TestCspStartupRejectsMissingStaleUnsafeAndMalformedExport(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "index.html")
	assertInitialCspPolicy(t, root, filename)
	assertStaleAndInvalidCspHtml(t, root, filename)
	if err := os.WriteFile(filename, []byte("<html>no script</html>"), 0600); err != nil {
		t.Fatal(err)
	}
	manifest := writeManifest(t, root)
	if len(manifest.ScriptHashes) != 0 {
		t.Fatal(manifest)
	}
	assertMalformedCspManifests(t, root)
	writeManifest(t, root)
	if err := os.Symlink(filename, filepath.Join(root, "linked.js")); err != nil {
		t.Skip("host does not permit creating symbolic links")
	}
	if _, err := loadSecurityPolicy(root); err == nil || !strings.Contains(err.Error(), "symbolic links") {
		t.Fatal(err)
	}
}

func TestCspManifestRetainsSortedFilesDuplicateHashesAndSequenceEncoding(t *testing.T) {
	root := t.TempDir()
	for name, html := range map[string]string{"z.HTML": "<script>a</script><script>a</script>", "a.html": "<script>b</script>"} {
		if err := os.WriteFile(filepath.Join(root, name), []byte(html), 0600); err != nil {
			t.Fatal(err)
		}
	}
	manifest := writeManifest(t, root)
	if manifest.Files[0].Path != "a.html" || len(manifest.Files[1].InlineScriptHashes) != 2 || len(manifest.ScriptHashes) != 2 {
		t.Fatal(manifest)
	}
	files := []any{}
	for _, file := range manifest.Files {
		files = append(files, []any{file.Path, file.HtmlSha256, file.InlineScriptHashes})
	}
	data, err := json.Marshal([]any{manifest.Version, manifest.Algorithm, files, manifest.ScriptHashes})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "csp-hashes.json"), data, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadSecurityPolicy(root); err != nil {
		t.Fatal(err)
	}
}

func TestCspMatchesBuiltFrontendManifest(t *testing.T) {
	root := os.Getenv("LITRADAR_TEST_WEB_ROOT")
	if root == "" {
		t.Skip("requires independently built frontend export")
	}
	policy, err := loadSecurityPolicy(root)
	if err != nil || !strings.Contains(policy, "sha256-") {
		t.Fatal(policy, err)
	}
}

// assertInitialCspPolicy checks missing manifest admission and exact initial script-hash policy.
func assertInitialCspPolicy(t *testing.T, root, filename string) {
	t.Helper()
	if err := os.WriteFile(filename, []byte("<script>ready=true;</script>"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadSecurityPolicy(root); err == nil || !strings.Contains(err.Error(), "CSP manifest metadata") {
		t.Fatal(err)
	}
	manifest := writeManifest(t, root)
	policy, err := loadSecurityPolicy(root)
	if err != nil || !strings.Contains(policy, "'"+manifest.ScriptHashes[0]+"'") {
		t.Fatal(policy, err)
	}
}

// assertStaleAndInvalidCspHtml checks stale deployment and UTF-8 failure before valid empty scripts.
func assertStaleAndInvalidCspHtml(t *testing.T, root, filename string) {
	t.Helper()
	if err := os.WriteFile(filename, []byte("<script>ready=false;</script>"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadSecurityPolicy(root); err == nil || err.Error() != "CSP manifest does not match the deployed static HTML" {
		t.Fatal(err)
	}
	if err := os.WriteFile(filename, []byte{255}, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadSecurityPolicy(root); err == nil || !strings.Contains(err.Error(), "valid UTF-8") {
		t.Fatal(err)
	}
}

// assertMalformedCspManifests checks every malformed manifest before the final symlink scenario.
func assertMalformedCspManifests(t *testing.T, root string) {
	t.Helper()
	for _, contents := range []string{`{"version":1,"version":1,"algorithm":"sha256","files":[],"script_hashes":[]}`, `{"version":1,"algorithm":"sha256","files":[],"script_hashes":[],"extra":1}`, `{"version":1,"algorithm":"sha256","files":[],"script_hashes":null}`, `{"version":1,"algorithm":"sha256","files":[{"path":"\ud800","html_sha256":"x","inline_script_hashes":[]}],"script_hashes":[]}`} {
		if err := os.WriteFile(filepath.Join(root, "csp-hashes.json"), []byte(contents), 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := loadSecurityPolicy(root); err == nil || !strings.Contains(err.Error(), "Invalid CSP manifest") {
			t.Fatal(contents, err)
		}
	}
}
