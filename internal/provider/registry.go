// Package provider validates and composes independently declared source capabilities.
package provider

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"sync"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// ErrorKind is the safe classification used by ordered provider fallback.
type ErrorKind string

const (
	NotFound               ErrorKind = "NotFound"
	AuthenticationRequired ErrorKind = "AuthenticationRequired"
	TemporarilyUnavailable ErrorKind = "TemporarilyUnavailable"
	InvalidResponse        ErrorKind = "InvalidResponse"
	Internal               ErrorKind = "Internal"
)

// Error carries a safe message without raw provider responses or credentials.
type Error struct {
	Kind    ErrorKind
	Message string
}

func (err *Error) Error() string { return err.Message }

// IndexContent resolves the next canonical batch for one maintained journal.
type IndexContent interface {
	Fetch(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error)
}

// ArticleAbstract resolves an ephemeral abstract destination from local metadata.
type ArticleAbstract interface {
	SupportsAbstract(domain.ArticleLocator) bool
	ResolveAbstract(context.Context, domain.ArticleLocator, domain.ArticleAccessContext) (domain.ArticleRedirect, error)
}

// ArticleFullText resolves an ephemeral document or redirect from local metadata.
type ArticleFullText interface {
	SupportsFullText(domain.ArticleLocator) bool
	ResolveFullText(context.Context, domain.ArticleLocator, domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error)
}

// Capabilities is the exact declaration of implemented provider operations.
type Capabilities struct{ IndexContent, ArticleAbstract, ArticleFullText bool }

// Contains tests a declared capability without inferring other operations.
func (capabilities Capabilities) Contains(kind domain.CapabilityKind) bool {
	switch kind {
	case domain.IndexContent:
		return capabilities.IndexContent
	case domain.ArticleAbstract:
		return capabilities.ArticleAbstract
	case domain.ArticleFullText:
		return capabilities.ArticleFullText
	}
	return false
}

// Descriptor fixes a stable provider name, capabilities and exact permitted redirect hosts.
type Descriptor struct {
	Name                 string
	Capabilities         Capabilities
	AllowedRedirectHosts []string
}

// Implementations supplies the concrete optional operations that must match the declaration.
type Implementations struct {
	IndexContent    IndexContent
	ArticleAbstract ArticleAbstract
	ArticleFullText ArticleFullText
}

// Registration owns an immutable validated descriptor and its implementations.
type Registration struct {
	descriptor      Descriptor
	implementations Implementations
}

// RegistryError records a configuration failure without raw upstream data.
type RegistryError struct{ Kind, Provider, Detail, Host string }

func (err *RegistryError) Error() string {
	switch err.Kind {
	case "InvalidConfiguration":
		return fmt.Sprintf("invalid configuration for provider %s: %s", err.Provider, err.Detail)
	case "InvalidName":
		return "invalid provider name: " + err.Provider
	case "NoCapabilities":
		return "provider declares no capabilities: " + err.Provider
	case "CapabilityMismatch":
		return "provider capability declaration mismatch: " + err.Provider
	case "InvalidRedirectHost":
		return fmt.Sprintf("invalid redirect host for provider %s: %s", err.Provider, err.Host)
	case "DuplicateRedirectHost":
		return fmt.Sprintf("duplicate redirect host for provider %s: %s", err.Provider, err.Host)
	case "RedirectHostsWithoutOnlineCapability":
		return "provider declares redirect hosts without an online capability: " + err.Provider
	case "DuplicateName":
		return "duplicate provider name: " + err.Provider
	default:
		return "invalid provider registration"
	}
}

// NewRegistration checks names and redirect hosts before capability consistency.
func NewRegistration(descriptor Descriptor, implementations Implementations) (*Registration, error) {
	if !validName(descriptor.Name) {
		return nil, &RegistryError{Kind: "InvalidName", Provider: descriptor.Name}
	}
	if err := validateRedirectHosts(descriptor); err != nil {
		return nil, err
	}
	if descriptor.Capabilities == (Capabilities{}) {
		return nil, &RegistryError{Kind: "NoCapabilities", Provider: descriptor.Name}
	}
	actual := Capabilities{implementations.IndexContent != nil, implementations.ArticleAbstract != nil, implementations.ArticleFullText != nil}
	if descriptor.Capabilities != actual {
		return nil, &RegistryError{Kind: "CapabilityMismatch", Provider: descriptor.Name}
	}
	descriptor.AllowedRedirectHosts = slices.Clone(descriptor.AllowedRedirectHosts)
	return &Registration{descriptor, implementations}, nil
}

// Descriptor returns a copy so callers cannot mutate validated redirect policy.
func (registration *Registration) Descriptor() Descriptor {
	copy := registration.descriptor
	copy.AllowedRedirectHosts = slices.Clone(copy.AllowedRedirectHosts)
	return copy
}

// IndexContent returns the declared indexing implementation, if any.
func (registration *Registration) IndexContent() IndexContent {
	return registration.implementations.IndexContent
}

// ArticleAbstract returns the declared abstract implementation, if any.
func (registration *Registration) ArticleAbstract() ArticleAbstract {
	return registration.implementations.ArticleAbstract
}

// ArticleFullText returns the declared full-text implementation, if any.
func (registration *Registration) ArticleFullText() ArticleFullText {
	return registration.implementations.ArticleFullText
}

// Registry holds registrations in deterministic name order; its zero value is empty.
type Registry struct {
	mutex         sync.RWMutex
	registrations map[string]*Registration
}

// Register rejects duplicates and preserves immutable registration ownership.
func (registry *Registry) Register(registration *Registration) error {
	validated, err := NewRegistration(registration.descriptor, registration.implementations)
	if err != nil {
		return err
	}
	registry.mutex.Lock()
	defer registry.mutex.Unlock()
	name := validated.descriptor.Name
	if _, exists := registry.registrations[name]; exists {
		return &RegistryError{Kind: "DuplicateName", Provider: name}
	}
	if registry.registrations == nil {
		registry.registrations = make(map[string]*Registration)
	}
	registry.registrations[name] = validated
	return nil
}

// Find resolves one exact provider name without applying a fallback.
func (registry *Registry) Find(name string) *Registration {
	registry.mutex.RLock()
	defer registry.mutex.RUnlock()
	return registry.registrations[name]
}

// ProvidersWith lists matching registrations sorted by their stable names.
func (registry *Registry) ProvidersWith(kind domain.CapabilityKind) []*Registration {
	registry.mutex.RLock()
	defer registry.mutex.RUnlock()
	result := make([]*Registration, 0)
	for _, registration := range registry.registrations {
		if registration.descriptor.Capabilities.Contains(kind) {
			result = append(result, registration)
		}
	}
	slices.SortFunc(result, func(first, second *Registration) int {
		return strings.Compare(first.descriptor.Name, second.descriptor.Name)
	})
	return result
}

func validName(name string) bool {
	if len(name) < 2 || len(name) > 64 {
		return false
	}
	for index, character := range []byte(name) {
		if character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || index > 0 && (character == '_' || character == '-') {
			continue
		}
		return false
	}
	return true
}

func validateRedirectHosts(descriptor Descriptor) error {
	if !descriptor.Capabilities.ArticleAbstract && !descriptor.Capabilities.ArticleFullText && len(descriptor.AllowedRedirectHosts) > 0 {
		return &RegistryError{Kind: "RedirectHostsWithoutOnlineCapability", Provider: descriptor.Name}
	}
	seen := make(map[string]bool)
	for _, host := range descriptor.AllowedRedirectHosts {
		isValid := len(host) >= 1 && len(host) <= 253 && strings.Contains(host, ".")
		for _, label := range strings.Split(host, ".") {
			if len(label) < 1 || len(label) > 63 || label[0] == '-' || label[len(label)-1] == '-' {
				isValid = false
				break
			}
			for _, character := range []byte(label) {
				if !(character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || character == '-') {
					isValid = false
					break
				}
			}
		}
		if !isValid {
			return &RegistryError{Kind: "InvalidRedirectHost", Provider: descriptor.Name, Host: host}
		}
		if seen[host] {
			return &RegistryError{Kind: "DuplicateRedirectHost", Provider: descriptor.Name, Host: host}
		}
		seen[host] = true
	}
	return nil
}
