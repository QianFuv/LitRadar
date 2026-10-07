package provider

import (
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// TestRegistrationErrorPrecedence preserves ordered host validation before capability checks.
func TestRegistrationErrorPrecedence(t *testing.T) {
	for _, entry := range []struct {
		descriptor Descriptor
		message    string
	}{
		{Descriptor{Name: "A", AllowedRedirectHosts: []string{"INVALID"}}, "invalid provider name: A"},
		{Descriptor{Name: "valid", AllowedRedirectHosts: []string{"INVALID"}}, "provider declares redirect hosts without an online capability: valid"},
		{Descriptor{Name: "valid", Capabilities: Capabilities{ArticleAbstract: true}, AllowedRedirectHosts: []string{"doi.org", "doi.org", "INVALID"}}, "duplicate redirect host for provider valid: doi.org"},
	} {
		_, err := NewRegistration(entry.descriptor, Implementations{})
		if err == nil || err.Error() != entry.message {
			t.Fatal(entry.descriptor, err)
		}
	}
}

// TestRegistryRevalidatesAndOwnsRegistration preserves an independent stored allowlist.
func TestRegistryRevalidatesAndOwnsRegistration(t *testing.T) {
	registration, err := NewRegistration(Descriptor{Name: "0_provider", Capabilities: Capabilities{ArticleAbstract: true}, AllowedRedirectHosts: []string{"123.456"}}, Implementations{ArticleAbstract: noExecution{}})
	if err != nil {
		t.Fatal(err)
	}
	var registry Registry
	if err := registry.Register(registration); err != nil {
		t.Fatal(err)
	}
	registration.descriptor.AllowedRedirectHosts[0] = "other.example"
	if registry.Find("0_provider").Descriptor().AllowedRedirectHosts[0] != "123.456" {
		t.Fatal("stored registration shares allowlist")
	}
}

// TestHostAndNameByteBoundaries pins ASCII length and separator admission.
func TestHostAndNameByteBoundaries(t *testing.T) {
	for _, name := range []string{"a", "_a", "A0", "a.b", strings.Repeat("a", 65), "éx"} {
		if validName(name) {
			t.Fatal("invalid provider name accepted", name)
		}
	}
	for _, host := range []string{"a", "a.", ".a", "a..b", "a.-b", "a.b-", "a.B", strings.Repeat("a", 64) + ".b"} {
		if validateRedirectHosts(Descriptor{Name: "valid", Capabilities: Capabilities{ArticleAbstract: true}, AllowedRedirectHosts: []string{host}}) == nil {
			t.Fatal("invalid redirect host accepted", host)
		}
	}
}

// TestDateGrammarRetainsIndependentRanges preserves shape validation without calendar checks.
func TestDateGrammarRetainsIndependentRanges(t *testing.T) {
	if err := optionalDate(new("2026-02-31")); err != nil {
		t.Fatal("independent day range changed", err)
	}
	for _, entry := range []struct{ value, message string }{
		{"0000-13-32", "date year must use a four-digit positive year"},
		{"2026-13-32", "date month is invalid"}, {"2026-02-32", "date day is invalid"},
		{"2026-2-01", "date must use YYYY, YYYY-MM, or YYYY-MM-DD"},
	} {
		if err := optionalDate(&entry.value); err == nil || err.Error() != entry.message {
			t.Fatal(entry, err)
		}
	}
}

// TestObservedIssnsValidateAfterMatch rejects later malformed observations despite an earlier match.
func TestObservedIssnsValidateAfterMatch(t *testing.T) {
	catalog := domain.JournalCatalogEntry{Title: "Journal", AllIssns: []string{"0378-5955"}}
	journal := domain.JournalDraft{ObservedTitle: new("Journal"), ObservedIssns: []string{"0378-5955", "INVALID"}}
	if err := validateJournal(catalog, journal); err == nil || err.Error() != "observed ISSN must use canonical NNNN-NNNX form" {
		t.Fatal(err)
	}
	journal.ObservedIssns = []string{"0378-5955", "0378-5955"}
	if err := validateJournal(catalog, journal); err != nil {
		t.Fatal("duplicate observations rejected", err)
	}
}

// TestBatchContentErrorPrecedesProgress preserves validation before progress admission.
func TestBatchContentErrorPrecedesProgress(t *testing.T) {
	catalog := domain.JournalCatalogEntry{CatalogId: "journal.test", Title: "Journal"}
	batch := domain.ProviderBatch{CatalogId: catalog.CatalogId, Journal: domain.JournalDraft{CatalogId: catalog.CatalogId}, Issues: []domain.IssueDraft{{CatalogId: "wrong"}}, Progress: domain.ProviderProgress{State: "invalid"}}
	if err := ValidateProviderBatch(catalog, batch); err == nil || err.Error() != "issue must echo the requested catalog_id" {
		t.Fatal(err)
	}
}
