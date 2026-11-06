package settings

import (
	"context"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

// TestOidcSettingsValidationAndSecretStorage checks HTTPS policy and encrypted client secrets.
func TestOidcSettingsValidationAndSecretStorage(t *testing.T) {
	for _, value := range []string{"http://issuer.example", "https://user:pass@issuer.example", "https://issuer.example?query=secret", "https://issuer.example/#fragment", "//issuer.example", "ftp://issuer.example"} {
		if ValidateOidcUrl(value, false) == nil {
			t.Fatal("invalid production endpoint accepted")
		}
	}
	if err := ValidateOidcUrl("http://127.0.0.1:8000", true); err != nil {
		t.Fatal(err)
	}
	repository, _ := testSettings(t)
	if _, err := repository.Update(context.Background(), nil, map[string]*string{"oidc_issuer_url": textValue("http://127.0.0.1")}, nil, nil); err == nil {
		t.Fatal("production accepted HTTP issuer")
	}
	items, err := repository.Update(context.Background(), nil, map[string]*string{"oidc_client_secret": textValue("oidc-test-secret")}, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	item := settingInfo(t, items, "oidc_client_secret")
	if item.Value != "" || !item.HasValue || !item.IsSecret || item.MaskedValue == "" || !secrets.IsRuntimeSecret("oidc_client_secret") {
		t.Fatal("secret escaped managed handling")
	}
	values, err := repository.Load(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, value := range values {
		if value.Field == "oidc_client_secret" {
			found = value.Value == "oidc-test-secret"
		}
	}
	if !found {
		t.Fatal("secret did not decrypt")
	}
	repository.isDevelopment = true
	if _, err := repository.Update(context.Background(), nil, map[string]*string{"oidc_issuer_url": textValue("http://127.0.0.1")}, nil, nil); err != nil {
		t.Fatal(err)
	}
}
