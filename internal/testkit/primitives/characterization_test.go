package primitives

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/x509"
	"encoding/pem"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestCertificateIdentities verifies independent trust roots, profiles and matching private keys.
func TestCertificateIdentities(t *testing.T) {
	directory := t.TempDir()
	started := time.Now()
	if err := Run("certificate", directory); err != nil {
		t.Fatal(err)
	}
	finished := time.Now()
	roots := map[string]*x509.Certificate{}
	leaves := map[string]*x509.Certificate{}
	for _, identity := range []string{"valid", "wrong-san", "wrong-ca"} {
		root, leaf := assertCertificateIdentity(t, directory, identity, started, finished)
		roots[identity], leaves[identity] = root, leaf
	}
	entries, err := os.ReadDir(directory)
	if err != nil || len(entries) != 9 {
		t.Fatal("certificate fixture file set changed", entries, err)
	}
	assertDistinctFixtureTrust(t, roots, leaves)
}

// assertCertificateIdentity verifies one independently generated fixture identity.
func assertCertificateIdentity(t *testing.T, directory, identity string, started, finished time.Time) (*x509.Certificate, *x509.Certificate) {
	t.Helper()
	root := readFixtureCertificate(t, filepath.Join(directory, identity+"-ca.pem"))
	leaf := readFixtureCertificate(t, filepath.Join(directory, identity+".pem"))
	assertFixtureRootProfile(t, root)
	assertFixtureLeafProfile(t, leaf)
	assertFixtureValidity(t, root, leaf, started, finished)
	assertFixturePrivateKey(t, filepath.Join(directory, identity+"-key.pem"), root, leaf)
	expectedHost := "www.pushplus.plus"
	if identity == "wrong-san" {
		expectedHost = "wrong.fixture.invalid"
	}
	if len(leaf.DNSNames) != 1 || leaf.DNSNames[0] != expectedHost {
		t.Fatal("fixture SAN changed", identity, leaf.DNSNames)
	}
	trust := x509.NewCertPool()
	trust.AddCert(root)
	if _, err := leaf.Verify(x509.VerifyOptions{Roots: trust, DNSName: expectedHost}); err != nil {
		t.Fatal("fixture does not verify under its own root", identity, err)
	}
	return root, leaf
}

// readFixturePem requires one complete PEM block of the independently expected kind.
func readFixturePem(t *testing.T, filename, kind string) []byte {
	t.Helper()
	body, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	block, remainder := pem.Decode(body)
	if block == nil || block.Type != kind || len(remainder) != 0 {
		t.Fatal("fixture PEM framing changed", filename)
	}
	return block.Bytes
}

// readFixtureCertificate parses the published certificate independently of the writer.
func readFixtureCertificate(t *testing.T, filename string) *x509.Certificate {
	t.Helper()
	certificate, err := x509.ParseCertificate(readFixturePem(t, filename, "CERTIFICATE"))
	if err != nil {
		t.Fatal(err)
	}
	return certificate
}

// assertFixtureRootProfile retains the trust-root identity and signing capabilities.
func assertFixtureRootProfile(t *testing.T, root *x509.Certificate) {
	t.Helper()
	if root.SerialNumber.Int64() != 1 || root.Subject.CommonName != "LitRadar migration fixture" || !root.IsCA || !root.BasicConstraintsValid || root.KeyUsage != x509.KeyUsageCertSign|x509.KeyUsageDigitalSignature {
		t.Fatal("fixture root profile changed", root)
	}
}

// assertFixtureLeafProfile retains the server profile even for deliberately wrong SANs.
func assertFixtureLeafProfile(t *testing.T, leaf *x509.Certificate) {
	t.Helper()
	if leaf.SerialNumber.Int64() != 2 || leaf.Subject.CommonName != "www.pushplus.plus" || leaf.KeyUsage != x509.KeyUsageDigitalSignature || len(leaf.ExtKeyUsage) != 1 || leaf.ExtKeyUsage[0] != x509.ExtKeyUsageServerAuth {
		t.Fatal("fixture leaf profile changed", leaf)
	}
}

// assertFixtureValidity bounds both clock reads and requires the leaf to inherit its root interval.
func assertFixtureValidity(t *testing.T, root, leaf *x509.Certificate, started, finished time.Time) {
	t.Helper()
	if !root.NotBefore.Equal(leaf.NotBefore) || !root.NotAfter.Equal(leaf.NotAfter) {
		t.Fatal("fixture validity inheritance changed")
	}
	if root.NotBefore.Before(started.Add(-time.Hour-time.Second)) || root.NotBefore.After(finished.Add(-time.Hour)) || root.NotAfter.Before(started.Add(24*time.Hour-time.Second)) || root.NotAfter.After(finished.Add(24*time.Hour)) {
		t.Fatal("fixture clock interval changed", root.NotBefore, root.NotAfter)
	}
}

// assertFixturePrivateKey requires PKCS8 P256 ownership shared by the root and its leaf.
func assertFixturePrivateKey(t *testing.T, filename string, root, leaf *x509.Certificate) {
	t.Helper()
	parsed, err := x509.ParsePKCS8PrivateKey(readFixturePem(t, filename, "PRIVATE KEY"))
	if err != nil {
		t.Fatal(err)
	}
	key, ok := parsed.(*ecdsa.PrivateKey)
	if !ok || key.Curve != elliptic.P256() || !key.PublicKey.Equal(root.PublicKey) || !key.PublicKey.Equal(leaf.PublicKey) {
		t.Fatal("fixture key ownership changed")
	}
}

// assertDistinctFixtureTrust checks the deliberately wrong CA and SAN controls.
func assertDistinctFixtureTrust(t *testing.T, roots, leaves map[string]*x509.Certificate) {
	t.Helper()
	validKey := roots["valid"].PublicKey.(*ecdsa.PublicKey)
	wrongCaKey := roots["wrong-ca"].PublicKey.(*ecdsa.PublicKey)
	wrongSanKey := roots["wrong-san"].PublicKey.(*ecdsa.PublicKey)
	if validKey.Equal(wrongCaKey) || validKey.Equal(wrongSanKey) || wrongSanKey.Equal(wrongCaKey) {
		t.Fatal("fixture identities shared a trust root")
	}
	trust := x509.NewCertPool()
	trust.AddCert(roots["wrong-ca"])
	if _, err := leaves["valid"].Verify(x509.VerifyOptions{Roots: trust, DNSName: "www.pushplus.plus"}); err == nil {
		t.Fatal("wrong fixture CA trusted the valid identity")
	}
	if err := leaves["wrong-san"].VerifyHostname("www.pushplus.plus"); err == nil {
		t.Fatal("wrong SAN accepted the valid host")
	}
}

// TestCertificateWriteFailure preserves prior outputs and stops before later identities.
func TestCertificateWriteFailure(t *testing.T) {
	for _, scenario := range []struct {
		blocked         string
		present, absent []string
	}{
		{"valid-ca.pem", nil, []string{"valid.pem", "valid-key.pem", "wrong-san-ca.pem", "wrong-ca.pem"}},
		{"wrong-san-ca.pem", []string{"valid-ca.pem", "valid.pem", "valid-key.pem"}, []string{"wrong-san.pem", "wrong-san-key.pem", "wrong-ca-ca.pem"}},
		{"valid.pem", []string{"valid-ca.pem"}, []string{"wrong-san-ca.pem", "wrong-ca-ca.pem"}},
	} {
		directory := t.TempDir()
		blocked := filepath.Join(directory, scenario.blocked)
		if err := os.Mkdir(blocked, 0700); err != nil {
			t.Fatal(err)
		}
		if err := certificates(directory); err == nil {
			t.Fatal("blocked certificate output accepted", scenario.blocked)
		}
		assertFixtureOutputs(t, directory, scenario.present, scenario.absent)
		metadata, err := os.Stat(blocked)
		if err != nil || !metadata.IsDir() {
			t.Fatal("blocked fixture directory changed", err)
		}
	}
}

// assertFixtureOutputs checks only deterministic publication boundaries, preserving map write order.
func assertFixtureOutputs(t *testing.T, directory string, present, absent []string) {
	t.Helper()
	for _, name := range present {
		if metadata, err := os.Stat(filepath.Join(directory, name)); err != nil || !metadata.Mode().IsRegular() {
			t.Fatal("prior fixture output missing", name, err)
		}
	}
	for _, name := range absent {
		if _, err := os.Stat(filepath.Join(directory, name)); !os.IsNotExist(err) {
			t.Fatal("later fixture output published", name, err)
		}
	}
}

// TestCertificateDirectoryFailure leaves a preexisting file untouched.
func TestCertificateDirectoryFailure(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "operator-data")
	if err := os.WriteFile(filename, []byte("preserve"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := certificates(filepath.Join(filename, "nested")); err == nil {
		t.Fatal("file ancestor accepted")
	}
	body, err := os.ReadFile(filename)
	if err != nil || string(body) != "preserve" {
		t.Fatal("operator data changed", err)
	}
}

// TestHelperCommandCaptureAndEnvironment exercises owned input, both output streams and explicit private policy.
func TestHelperCommandCaptureAndEnvironment(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("LITRADAR_PRIMITIVE_HELPER_MODE", "echo")
	t.Setenv("LITRADAR_PRIMITIVE_HELPER_OTHER", "preserved")
	t.Setenv("OBSCURA_ALLOW_PRIVATE_NETWORK", "ambient")
	for _, allowPrivate := range []bool{false, true} {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		output, diagnostics, err := helperCommand(ctx, executable, []string{"-test.run=^TestPrimitiveHelperFixture$"}, "synthetic-input", allowPrivate)
		cancel()
		expectedPolicy := ""
		if allowPrivate {
			expectedPolicy = "1"
		}
		if err != nil || output != "policy="+expectedPolicy+"|other=preserved|input=synthetic-input" || diagnostics != "synthetic-diagnostic" {
			t.Fatal("helper capture or private policy changed", output, diagnostics, err)
		}
	}
}

// TestHelperCommandFailureAndLimit retains diagnostics on exit failure and redacts truncated captures.
func TestHelperCommandFailureAndLimit(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	for _, mode := range []string{"fail", "flood"} {
		t.Setenv("LITRADAR_PRIMITIVE_HELPER_MODE", mode)
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		output, diagnostics, err := helperCommand(ctx, executable, []string{"-test.run=^TestPrimitiveHelperFixture$"}, "", false)
		cancel()
		assertHelperFailureCapture(t, mode, output, diagnostics, err)
	}
}

// assertHelperFailureCapture checks independently expected exit and truncation results.
func assertHelperFailureCapture(t *testing.T, mode, output, diagnostics string, err error) {
	t.Helper()
	if err == nil {
		t.Fatal("helper failure accepted", mode)
	}
	if mode == "fail" && (output != "synthetic-failure" || diagnostics != "synthetic-diagnostic") {
		t.Fatal("failure diagnostics lost", output, diagnostics)
	}
	if mode == "flood" && (output != "" || diagnostics != "" || err.Error() != "helper exceeded fixture capture limit") {
		t.Fatal("truncated capture exposed", len(output), len(diagnostics), err)
	}
}

type fixtureCommandResult struct {
	output, diagnostics string
	err                 error
}

// TestHelperCommandCancellation releases a blocked input writer and the child-owned listener.
func TestHelperCommandCancellation(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	ready := filepath.Join(t.TempDir(), "ready")
	t.Setenv("LITRADAR_PRIMITIVE_HELPER_MODE", "hold")
	t.Setenv("LITRADAR_PRIMITIVE_HELPER_READY", ready)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	result := make(chan fixtureCommandResult, 1)
	done := make(chan struct{})
	t.Cleanup(func() {
		cancel()
		select {
		case <-done:
		case <-time.After(12 * time.Second):
			t.Error("helper command did not finalize")
		}
	})
	go func() {
		defer close(done)
		output, diagnostics, err := helperCommand(ctx, executable, []string{"-test.run=^TestPrimitiveHelperFixture$"}, strings.Repeat("x", 2*1024*1024), false)
		result <- fixtureCommandResult{output, diagnostics, err}
	}()
	address := waitForFixtureListener(t, ready)
	cancel()
	select {
	case captured := <-result:
		if captured.err == nil {
			t.Fatal("cancelled fixture accepted")
		}
	case <-time.After(12 * time.Second):
		t.Fatal("cancelled fixture writer did not join")
	}
	assertFixtureListenerClosed(t, address)
}

// waitForFixtureListener observes a bounded child readiness barrier without cancelling before startup.
func waitForFixtureListener(t *testing.T, ready string) string {
	t.Helper()
	deadline := time.Now().Add(6 * time.Second)
	for time.Now().Before(deadline) {
		if body, err := os.ReadFile(ready); err == nil && fixtureListenerReady(string(body)) {
			return string(body)
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("synthetic child did not become ready")
	return ""
}

// fixtureListenerReady verifies the published loopback address accepts a connection.
func fixtureListenerReady(address string) bool {
	host, port, err := net.SplitHostPort(address)
	if err != nil || host != "127.0.0.1" || port == "" || port == "0" {
		return false
	}
	connection, err := net.DialTimeout("tcp", address, 250*time.Millisecond)
	if err != nil {
		return false
	}
	connection.Close()
	return true
}

// assertFixtureListenerClosed proves the owned process released its socket before helperCommand returned.
func assertFixtureListenerClosed(t *testing.T, address string) {
	t.Helper()
	connection, err := net.DialTimeout("tcp", address, time.Second)
	if err == nil {
		connection.Close()
		t.Fatal("cancelled helper retained its listener")
	}
	listener, err := net.Listen("tcp", address)
	if err != nil {
		t.Fatal("cancelled helper did not release its socket", err)
	}
	listener.Close()
}

// TestPrimitiveHelperFixture runs only explicitly selected synthetic child modes.
func TestPrimitiveHelperFixture(t *testing.T) {
	switch os.Getenv("LITRADAR_PRIMITIVE_HELPER_MODE") {
	case "":
		return
	case "echo":
		writeFixtureEcho()
	case "fail":
		io.WriteString(os.Stdout, "synthetic-failure")
		io.WriteString(os.Stderr, "synthetic-diagnostic")
		os.Exit(7)
	case "flood":
		io.WriteString(os.Stdout, strings.Repeat("x", 1024*1024+1))
		io.WriteString(os.Stderr, "synthetic-diagnostic")
		os.Exit(0)
	case "hold":
		holdFixtureListener()
	default:
		t.Fatal("unknown synthetic fixture mode")
	}
}

// writeFixtureEcho consumes input through EOF and exposes only synthetic environment values.
func writeFixtureEcho() {
	body, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(8)
	}
	fmt.Fprintf(os.Stdout, "policy=%s|other=%s|input=%s", os.Getenv("OBSCURA_ALLOW_PRIVATE_NETWORK"), os.Getenv("LITRADAR_PRIMITIVE_HELPER_OTHER"), body)
	io.WriteString(os.Stderr, "synthetic-diagnostic")
	os.Exit(0)
}

// holdFixtureListener intentionally blocks stdin until the parent terminates its owned process.
func holdFixtureListener() {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		os.Exit(9)
	}
	defer listener.Close()
	if err := os.WriteFile(os.Getenv("LITRADAR_PRIMITIVE_HELPER_READY"), []byte(listener.Addr().String()), 0600); err != nil {
		os.Exit(10)
	}
	for {
		time.Sleep(time.Second)
	}
}
