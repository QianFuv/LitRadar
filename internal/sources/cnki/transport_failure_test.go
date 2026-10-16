package cnki

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

// failedCnkiTransport injects a network failure without contacting an upstream.
type failedCnkiTransport struct{ failure error }

// RoundTrip returns the injected error without exposing the request in diagnostics.
func (wire failedCnkiTransport) RoundTrip(*http.Request) (*http.Response, error) {
	return nil, wire.failure
}

// TestTransportFailureDiagnostics preserves safe public errors while identifying actionable failures.
func TestTransportFailureDiagnostics(t *testing.T) {
	for _, scenario := range []struct {
		name    string
		failure error
	}{
		{"tls_hostname", &tls.CertificateVerificationError{Err: x509.HostnameError{Certificate: &x509.Certificate{}, Host: "private-host"}}},
		{"tls_validity", &tls.CertificateVerificationError{Err: x509.CertificateInvalidError{Cert: &x509.Certificate{}, Reason: x509.Expired}}},
		{"tls_unknown_authority", &tls.CertificateVerificationError{Err: x509.UnknownAuthorityError{Cert: &x509.Certificate{}}}},
		{"tls_certificate", &tls.CertificateVerificationError{Err: errors.New("private-certificate")}},
		{"cancelled", context.Canceled},
		{"timeout", context.DeadlineExceeded},
		{"dns", &net.DNSError{Name: "private-host"}},
		{"eof", io.EOF},
		{"eof", io.ErrUnexpectedEOF},
		{"transport", errors.New("private-error")},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			var output strings.Builder
			original := slog.Default()
			slog.SetDefault(slog.New(slog.NewJSONHandler(&output, nil)))
			t.Cleanup(func() { slog.SetDefault(original) })
			live, err := NewLiveTransport(LiveConfig{TimeoutSeconds: 1}, transport.Proxy{}, time.Time{})
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(live.Close)
			live.state.client.Transport = failedCnkiTransport{&url.Error{Op: "Get", URL: "https://private-user:private-password@private-host/?token=private-token", Err: fmt.Errorf("private-error: %w", scenario.failure)}}
			budget := requestBudget{transportFailures: 7, attemptsStarted: 8}
			_, _, _, shouldRetry, err := live.readTextAttempt(context.Background(), time.Now().Add(time.Second), "GET", KnsBase+"/article?captchaId=private-captcha", nil, http.Header{"Authorization": {"private-token"}}, "article_detail", &budget)
			if err == nil || err.Error() != "domestic CNKI request failed" || shouldRetry || budget.transportFailures != 8 {
				t.Fatal("public failure or retry budget changed")
			}
			attempts := live.DrainAttempts()
			if len(attempts) != 1 || attempts[0].Error == nil || *attempts[0].Error != "request failed" || !attempts[0].DidRetry {
				t.Fatal("attempt contract changed")
			}
			if strings.Contains(output.String(), "private-") {
				t.Fatal("diagnostic exposed private data")
			}
			var record map[string]any
			if err := json.Unmarshal([]byte(output.String()), &record); err != nil {
				t.Fatalf("missing structured transport diagnostic: %v", err)
			}
			if record["event"] != "source.cnki.transport.failed" || record["error_kind"] != scenario.name || record["endpoint"] != "article_detail" || record["method"] != "GET" || record["is_retry"] != true {
				t.Fatalf("unexpected diagnostic: %v", record)
			}
		})
	}
}
