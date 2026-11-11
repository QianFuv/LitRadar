package api

import (
	"encoding/json"
	"slices"

	"github.com/QianFuv/LitRadar/apps/server/internal/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
)

// validateRuntimeValues checks normalized shapes before validating raw provider capabilities.
func validateRuntimeValues(values map[string]*string) *apiError {
	keys := sortedRuntimeKeys(values)
	if failure := validateRuntimeNormalization(values, keys); failure != nil {
		return failure
	}
	validator := runtimeProviderValidator{capabilities: sources.BuiltInCapabilities()}
	for _, field := range keys {
		if values[field] == nil {
			continue
		}
		if failure := validator.field(field, *values[field]); failure != nil {
			return failure
		}
	}
	return nil
}

// sortedRuntimeKeys preserves deterministic field and catalog validation order.
func sortedRuntimeKeys[Value any](values map[string]Value) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	return keys
}

// validateRuntimeNormalization validates known settings without replacing the original raw values.
func validateRuntimeNormalization(values map[string]*string, keys []string) *apiError {
	for _, field := range keys {
		if values[field] == nil {
			continue
		}
		if _, known := settings.Default(field); !known {
			continue
		}
		if _, err := settings.Normalize(field, *values[field]); err != nil {
			return badRequest(err.Error())
		}
	}
	return nil
}

type runtimeProviderValidator struct{ capabilities []sources.CapabilityInfo }

// provider checks a raw provider name against the requested configured capability.
func (validator runtimeProviderValidator) provider(name, field string) *apiError {
	for _, info := range validator.capabilities {
		if info.Name != name {
			continue
		}
		if !supportsRuntimeCapability(field, info) {
			return badRequest("Provider " + name + " does not support the configured capability")
		}
		return nil
	}
	return badRequest("Unknown Provider: " + name)
}

// supportsRuntimeCapability distinguishes proxy policy from the three content capabilities.
func supportsRuntimeCapability(field string, info sources.CapabilityInfo) bool {
	switch field {
	case "provider_proxy_policy":
		return true
	case "index_provider_routes":
		return info.IndexContent
	case "article_abstract_provider_orders":
		return info.ArticleAbstract
	case "article_fulltext_provider_orders":
		return info.ArticleFullText
	}
	return false
}

// field selects the raw capability configuration grammar after the normalization phase.
func (validator runtimeProviderValidator) field(field, value string) *apiError {
	switch field {
	case "index_provider_routes":
		return validator.routes(field, value)
	case "provider_proxy_policy":
		return validator.proxyPolicy(field, value)
	case "article_abstract_provider_orders", "article_fulltext_provider_orders":
		return validator.orders(field, value)
	}
	return nil
}

// routes validates providers in sorted catalog order without canonicalizing their raw names.
func (validator runtimeProviderValidator) routes(field, value string) *apiError {
	var routes map[string]string
	if json.Unmarshal([]byte(value), &routes) != nil || routes == nil {
		return badRequest("Invalid index Provider routes")
	}
	for _, catalog := range sortedRuntimeKeys(routes) {
		if failure := validator.provider(routes[catalog], field); failure != nil {
			return failure
		}
	}
	return nil
}

// proxyPolicy validates all raw policy keys, including entries whose flag is false.
func (validator runtimeProviderValidator) proxyPolicy(field, value string) *apiError {
	var policy map[string]bool
	if json.Unmarshal([]byte(value), &policy) != nil || policy == nil {
		return badRequest("Invalid Provider proxy policy")
	}
	for _, name := range sortedRuntimeKeys(policy) {
		if failure := validator.provider(name, field); failure != nil {
			return failure
		}
	}
	return nil
}

// orders validates the default order before sorted catalog-specific orders.
func (validator runtimeProviderValidator) orders(field, value string) *apiError {
	var configuration settings.ProviderOrders
	if json.Unmarshal([]byte(value), &configuration) != nil {
		return badRequest("Invalid " + field)
	}
	orders := [][]string{configuration.Default}
	for _, catalog := range sortedRuntimeKeys(configuration.Catalogs) {
		orders = append(orders, configuration.Catalogs[catalog])
	}
	for _, order := range orders {
		if failure := validator.order(field, order); failure != nil {
			return failure
		}
	}
	return nil
}

// order rejects a repeated provider before rechecking that occurrence's capability.
func (validator runtimeProviderValidator) order(field string, order []string) *apiError {
	seen := map[string]bool{}
	for _, name := range order {
		if seen[name] {
			return badRequest("Duplicate Provider in order: " + name)
		}
		seen[name] = true
		if failure := validator.provider(name, field); failure != nil {
			return failure
		}
	}
	return nil
}
