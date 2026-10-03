// Package sources composes the built-in scholarly and library providers.
package sources

import (
	"encoding/json"
	"errors"
	"log/slog"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const (
	ScholarlyProviderName = "scholarly"
	CnkiProviderName      = "cnki"
	ZjlibProviderName     = "zjlib"
)

// CapabilityInfo exposes aggregate built-in capabilities without network or secret state.
type CapabilityInfo struct {
	Name            string `json:"name"`
	IndexContent    bool   `json:"index_content"`
	ArticleAbstract bool   `json:"article_abstract"`
	ArticleFullText bool   `json:"article_full_text"`
}

// BuiltInCapabilities returns the original deterministic logical provider catalog.
func BuiltInCapabilities() []CapabilityInfo {
	return []CapabilityInfo{{CnkiProviderName, true, true, false}, {ScholarlyProviderName, true, true, false}, {ZjlibProviderName, false, false, true}}
}

// Capabilities projects an aggregate metadata entry onto registration flags.
func (info CapabilityInfo) Capabilities() provider.Capabilities {
	return provider.Capabilities{IndexContent: info.IndexContent, ArticleAbstract: info.ArticleAbstract, ArticleFullText: info.ArticleFullText}
}

var (
	ErrProxyPolicy          = errors.New("Invalid Provider proxy policy")
	ErrUnknownProxyProvider = errors.New("Unknown Provider proxy policy name")
	ErrMissingProxyUrl      = errors.New("Provider proxy URL is required when a Provider proxy is enabled")
)

// ProxySelection applies one validated URL only to explicitly enabled logical providers.
type ProxySelection struct {
	global  transport.Proxy
	enabled map[string]bool
}

// NewProxySelection validates policy shape and names before the URL, even for a disabled policy.
func NewProxySelection(proxyUrl, policy string) (ProxySelection, error) {
	value, err := transport.ParseJson([]byte(policy))
	if err != nil {
		return ProxySelection{}, ErrProxyPolicy
	}
	object, isObject := value.(map[string]any)
	if !isObject {
		return ProxySelection{}, ErrProxyPolicy
	}
	for _, value := range object {
		if _, isBoolean := value.(bool); !isBoolean {
			return ProxySelection{}, ErrProxyPolicy
		}
	}
	decoder := json.NewDecoder(strings.NewReader(policy))
	decoder.UseNumber()
	decoder.Token()
	for decoder.More() {
		decoder.Token()
		var value any
		if err := decoder.Decode(&value); err != nil {
			return ProxySelection{}, ErrProxyPolicy
		}
		if _, isBoolean := value.(bool); !isBoolean {
			return ProxySelection{}, ErrProxyPolicy
		}
	}
	known := make(map[string]bool)
	for _, info := range BuiltInCapabilities() {
		known[info.Name] = true
	}
	for name := range object {
		if !known[name] {
			return ProxySelection{}, ErrUnknownProxyProvider
		}
	}
	enabled := make(map[string]bool)
	for name, value := range object {
		if value.(bool) {
			enabled[name] = true
		}
	}
	var global transport.Proxy
	if proxyUrl = strings.TrimSpace(proxyUrl); proxyUrl != "" {
		global, err = transport.ExplicitProxy(proxyUrl)
		if err != nil {
			return ProxySelection{}, err
		}
	}
	if len(enabled) > 0 && !global.IsExplicit() {
		return ProxySelection{}, ErrMissingProxyUrl
	}
	return ProxySelection{global, enabled}, nil
}

// ProxySelectionFromRuntime preserves the first occurrence of each managed setting key.
func ProxySelectionFromRuntime(values []settings.Value) (ProxySelection, error) {
	proxyUrl, policy := "", "{}"
	hasUrl, hasPolicy := false, false
	for _, value := range values {
		if value.Field == "provider_proxy_url" && !hasUrl {
			proxyUrl = value.Value
			hasUrl = true
		}
		if value.Field == "provider_proxy_policy" && !hasPolicy {
			policy = value.Value
			hasPolicy = true
		}
	}
	return NewProxySelection(proxyUrl, policy)
}

// ForProvider selects direct networking when a name is absent or disabled.
func (selection ProxySelection) ForProvider(name string) transport.Proxy {
	if selection.enabled[name] {
		return selection.global
	}
	return transport.Proxy{}
}

// ProxyUrlForProvider explicitly exposes the selected bootstrap URL to its intended child.
func (selection ProxySelection) ProxyUrlForProvider(name string) (string, bool) {
	return selection.ForProvider(name).Url()
}

func (selection ProxySelection) String() string {
	names := make([]string, 0, len(selection.enabled))
	for name := range selection.enabled {
		names = append(names, name)
	}
	slices.Sort(names)
	encoded, _ := json.Marshal(names)
	return "ProviderProxySelection(enabled_providers=" + string(encoded) + ", proxy_url=[REDACTED])"
}
func (selection ProxySelection) GoString() string     { return selection.String() }
func (selection ProxySelection) LogValue() slog.Value { return slog.StringValue(selection.String()) }
