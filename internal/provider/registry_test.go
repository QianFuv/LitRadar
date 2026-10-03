package provider

import (
	"context"
	"encoding/json"
	"reflect"
	"strings"
	"sync"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

func TestRegistryOrderingOwnershipAndDuplicates(t *testing.T) {
	var registry Registry
	for _, name := range []string{"z_provider", "a_provider", "middle"} {
		hosts := []string{"doi.org"}
		registration, err := NewRegistration(Descriptor{Name: name, Capabilities: Capabilities{ArticleAbstract: true}, AllowedRedirectHosts: hosts}, Implementations{ArticleAbstract: noExecution{}})
		if err != nil {
			t.Fatal(err)
		}
		hosts[0] = "untrusted.example"
		if err := registry.Register(registration); err != nil {
			t.Fatal(err)
		}
		if err := registry.Register(registration); err == nil || err.Error() != "duplicate provider name: "+name {
			t.Fatal(err)
		}
		descriptor := registry.Find(name).Descriptor()
		descriptor.AllowedRedirectHosts[0] = "changed.example"
		if registry.Find(name).Descriptor().AllowedRedirectHosts[0] != "doi.org" {
			t.Fatal("descriptor mutated")
		}
	}
	names := []string{}
	for _, registration := range registry.ProvidersWith(domain.ArticleAbstract) {
		names = append(names, registration.Descriptor().Name)
	}
	if !reflect.DeepEqual(names, []string{"a_provider", "middle", "z_provider"}) {
		t.Fatal(names)
	}
	if registry.Find("unknown") != nil || len(registry.ProvidersWith(domain.IndexContent)) != 0 {
		t.Fatal("unexpected capability")
	}
	var workers sync.WaitGroup
	for index := 0; index < 8; index++ {
		workers.Go(func() {
			for count := 0; count < 100; count++ {
				registry.Find("middle").Descriptor()
				registry.ProvidersWith(domain.ArticleAbstract)
			}
		})
	}
	workers.Wait()
}

func TestEmptyBatchRoundTripsThroughStrictArrays(t *testing.T) {
	batch := domain.ProviderBatch{CatalogId: "journal.test", Journal: domain.JournalDraft{CatalogId: "journal.test"}, Progress: domain.ProviderProgress{State: domain.Complete}}
	encoded, err := json.Marshal(batch)
	if err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"observed_issns", "observed_title_aliases", "issues", "articles"} {
		if !strings.Contains(string(encoded), `"`+field+`":[]`) {
			t.Fatalf("%s was not an array: %s", field, encoded)
		}
	}
	var decoded domain.ProviderBatch
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), "next_anchor") {
		t.Fatal("absent completed anchor should be omitted")
	}
}

type countedIndex struct {
	calls   int
	failure error
	batch   domain.ProviderBatch
}

func (implementation *countedIndex) Fetch(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error) {
	implementation.calls++
	return implementation.batch, implementation.failure
}

func TestFixtureOpaquePreflightAndSafeFailure(t *testing.T) {
	empty := ""
	implementation := &countedIndex{failure: &Error{Kind: TemporarilyUnavailable, Message: "private upstream response"}}
	_, err := ValidateIndexProviderFixture(context.Background(), implementation, domain.JournalCatalogEntry{}, domain.IndexFetchContext{CommittedAnchor: &empty})
	if err == nil || implementation.calls != 0 {
		t.Fatal("invalid state reached provider", err)
	}
	_, err = ValidateIndexProviderFixture(context.Background(), implementation, domain.JournalCatalogEntry{}, domain.IndexFetchContext{})
	if err == nil || err.Error() != "index provider fixture failed with TemporarilyUnavailable" || implementation.calls != 1 {
		t.Fatal(err)
	}
}
