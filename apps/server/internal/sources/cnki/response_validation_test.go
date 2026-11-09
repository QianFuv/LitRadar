package cnki

import (
	"errors"
	"testing"
)

// TestResponseClassificationPrecedence preserves verification, deletion, structure and URL admission order.
func TestResponseClassificationPrecedence(t *testing.T) {
	for _, scenario := range []struct {
		name, text, endpoint, expectedKind, expectedMessage string
	}{
		{"empty", " \t\n", "article_detail", "Parse", "domestic CNKI returned an empty response"},
		{"verification-before-deletion", "CAPTCHA RECORD DOES NOT EXIST", "article_detail", "Request", "domestic CNKI verification required"},
		{"deletion-before-structure", "RECORD HAS BEEN DELETED", "article_detail", "PermanentArticleMissing", "domestic CNKI article is permanently unavailable"},
		{"content-deletion", `<input id="paramfilename" value="id">CAPTCHA RECORD DOES NOT EXIST`, "article_detail", "PermanentArticleMissing", "domestic CNKI article is permanently unavailable"},
		{"content-verification-marker", `<input id="paramfilename" value="id">CAPTCHA`, "article_detail", "", ""},
		{"structure", "ordinary text", "article_detail", "Parse", "domestic CNKI article_detail response is structurally incomplete"},
		{"other-endpoint", `<input id="pykm" value="id">RECORD HAS BEEN DELETED`, "journal_detail", "", ""},
		{"unicode", `<input id="paramfilename" value="id">İ ΟΣ 研究`, "article_detail", "", ""},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			err := validateResponse(scenario.endpoint, scenario.text)
			if scenario.expectedKind == "" {
				if err != nil {
					t.Fatal(err)
				}
				return
			}
			var failure *Error
			if !errors.As(err, &failure) || failure.Kind != scenario.expectedKind || failure.Error() != scenario.expectedMessage {
				t.Fatal("response classification changed", err)
			}
		})
	}
	if _, err := ParseArticleDetail("RECORD HAS BEEN DELETED", "not an allowed URL"); err == nil || err.Error() != "domestic CNKI article is permanently unavailable" {
		t.Fatal("URL validation replaced the earlier response failure", err)
	}
}
