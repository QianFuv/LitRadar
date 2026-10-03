package sources

import (
	"fmt"
	"log/slog"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
	"github.com/QianFuv/LitRadar/internal/transport"
)

func TestProxySelectionValidationAndPrecedence(t *testing.T) {
	for _, test := range []struct {
		url, policy string
		want        error
	}{
		{"", `{}`, nil}, {"", `null`, ErrProxyPolicy}, {"", `[]`, ErrProxyPolicy}, {"", `{"cnki":null}`, ErrProxyPolicy},
		{"", `{"cnki":"bad","cnki":true}`, ErrProxyPolicy}, {"", `{"cnki":null,"cnki":false}`, ErrProxyPolicy},
		{"", `{"unknown":false}`, ErrUnknownProxyProvider}, {"bad", `{"unknown":false}`, ErrUnknownProxyProvider},
		{"bad", `{}`, transport.ErrProxyUrl}, {"", `{"cnki":true}`, ErrMissingProxyUrl}, {" \t", `{"cnki":true}`, ErrMissingProxyUrl},
		{"http://localhost:8080", `{"cnki":true,"cnki":false}`, nil}, {"http://localhost:8080", `{"cnki":false,"cnki":true}`, nil},
	} {
		t.Run(test.policy+test.url, func(t *testing.T) {
			_, err := NewProxySelection(test.url, test.policy)
			if err != test.want {
				t.Fatalf("%v want %v", err, test.want)
			}
		})
	}
	selection, err := NewProxySelection("http://private-user:private-password@localhost:8080", `{"cnki":true,"scholarly":false,"zjlib":true}`)
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"cnki", "zjlib", "scholarly", "cnki_oversea"} {
		if selection.ForProvider(name).IsExplicit() != (name == "cnki" || name == "zjlib") {
			t.Fatal(name)
		}
	}
	if value, exists := selection.ProxyUrlForProvider("cnki"); !exists || !strings.Contains(value, "private-password") {
		t.Fatal("explicit bootstrap lost")
	}
	for _, value := range []any{selection, &selection} {
		var output strings.Builder
		logger := slog.New(slog.NewJSONHandler(&output, nil))
		logger.Info("selection", slog.Any("value", value))
		output.WriteString(fmt.Sprintf("%v %+v %#v", value, value, value))
		if strings.Contains(output.String(), "private-") {
			t.Fatal("credentials leaked")
		}
	}
}

func TestRuntimeProxySettingsUseFirstOccurrence(t *testing.T) {
	selection, err := ProxySelectionFromRuntime([]settings.Value{{Field: "provider_proxy_url", Value: "http://localhost:8080"}, {Field: "provider_proxy_url", Value: "bad"}, {Field: "provider_proxy_policy", Value: `{"cnki":true}`}, {Field: "provider_proxy_policy", Value: `{"unknown":true}`}})
	if err != nil || !selection.ForProvider("cnki").IsExplicit() {
		t.Fatal(selection, err)
	}
	selection, err = ProxySelectionFromRuntime(nil)
	if err != nil || selection.ForProvider("cnki").IsExplicit() {
		t.Fatal(selection, err)
	}
}
