package api

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	scheduled "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/platform/cron"
	"github.com/QianFuv/LitRadar/internal/sources"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func runtimeSettingsBody() bodyType {
	pool := structBody("RuntimeSecretPoolUpdate", defaultBodyField("add", listBody(stringBody), []any{}), defaultBodyField("remove", listBody(stringBody), []any{}))
	return structBody("RuntimeSettingsUpdate", defaultBodyField("values", mapBody(optionalBody(stringBody)), map[string]any{}), defaultBodyField("secret_pool_updates", mapBody(pool), map[string]any{}))
}

// validateScheduledBody preserves label, cron, job and timing rejection order.
func validateScheduledBody(body map[string]any) *apiError {
	if failure := normalizeScheduledLabels(body); failure != nil {
		return failure
	}
	if value, ok := body["cron"].(string); ok {
		if _, err := cron.Parse(value); err != nil {
			return badRequest(err.Error())
		}
	}
	if job, ok := body["job"].(scheduled.Job); ok {
		if err := job.Validate(); err != nil {
			return badRequest(err.Error())
		}
	}
	return validateScheduledTiming(body)
}

// normalizeScheduledLabels trims labels in declaration order before semantic validation.
func normalizeScheduledLabels(body map[string]any) *apiError {
	for _, field := range []struct{ name, label string }{{"name", "Task name"}, {"cron", "Cron"}, {"timezone", "Timezone"}} {
		if value, ok := body[field.name].(string); ok {
			value = strings.TrimSpace(value)
			if value == "" {
				return badRequest(field.label + " must not be empty")
			}
			body[field.name] = value
		}
	}
	return nil
}

// validateScheduledTiming supplies defaults only for the supplied timing fields.
func validateScheduledTiming(body map[string]any) *apiError {
	zone, hasZone := body["timezone"].(string)
	timeout, hasTimeout := body["timeout_seconds"].(uint64)
	if hasZone || hasTimeout {
		if !hasZone {
			zone = "UTC"
		}
		if !hasTimeout {
			timeout = 3600
		}
		if err := scheduled.ValidateTiming(zone, timeout); err != nil {
			return badRequest(err.Error())
		}
	}
	return nil
}

func configurationFields(body map[string]any) (map[string]*string, map[string]settings.PoolUpdate) {
	values := map[string]*string{}
	for field, value := range body["values"].(map[string]any) {
		if value == nil {
			values[field] = nil
		} else {
			text := value.(string)
			values[field] = &text
		}
	}
	pools := map[string]settings.PoolUpdate{}
	for field, value := range body["secret_pool_updates"].(map[string]any) {
		fields := value.(map[string]any)
		update := settings.PoolUpdate{Add: []string{}, Remove: []string{}}
		for _, item := range fields["add"].([]any) {
			update.Add = append(update.Add, item.(string))
		}
		for _, item := range fields["remove"].([]any) {
			update.Remove = append(update.Remove, item.(string))
		}
		pools[field] = update
	}
	return values, pools
}

// validateRuntimeBody normalizes every known field before checking capabilities against raw values.
func validateRuntimeBody(body map[string]any) *apiError {
	values, _ := configurationFields(body)
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

// performConfiguration executes the selected administrator configuration operation.
func (handlers *adminHandlers) performConfiguration(ctx context.Context, name string, actor identity.Id, target int64, body map[string]any, event auth.AuditEvent) (any, error) {
	switch name {
	case "list_scheduled_tasks":
		return handlers.scheduler.List(ctx)
	case "scheduler_status":
		return handlers.scheduler.Status(ctx, currentTimestamp(), 90, 20)
	case "delete_scheduled_task":
		return handlers.scheduler.Delete(ctx, target, &actor, &event)
	case "create_scheduled_task":
		return handlers.scheduler.Create(ctx, scheduled.Create{Name: body["name"].(string), Job: body["job"].(scheduled.Job), Cron: body["cron"].(string), Timezone: body["timezone"].(string), TimeoutSeconds: body["timeout_seconds"].(uint64), Coalesce: body["coalesce"].(bool), Enabled: body["enabled"].(bool)}, &actor, &event)
	case "update_scheduled_task":
		update := scheduledUpdate(target, body)
		return handlers.scheduler.Update(ctx, update, &actor, &event)
	case "list_runtime_settings":
		return handlers.settings.List(ctx)
	case "update_runtime_settings":
		values, pools := configurationFields(body)
		return handlers.settings.Update(ctx, &actor, values, pools, &event)
	case "get_provider_catalog":
		catalogs, err := handlers.storage.ListProviderCatalogs()
		if err != nil {
			return nil, err
		}
		return struct {
			Providers []sources.CapabilityInfo `json:"providers"`
			Catalogs  []domain.ProviderCatalog `json:"catalogs"`
		}{sources.BuiltInCapabilities(), catalogs}, nil
	}
	return nil, fmt.Errorf("unknown administrator configuration operation")
}

// scheduledUpdate retains the distinction between omitted and supplied update fields.
func scheduledUpdate(target int64, body map[string]any) scheduled.Update {
	update := scheduled.Update{TaskId: target}
	if value, ok := body["name"].(string); ok {
		update.Name = &value
	}
	if value, ok := body["job"].(scheduled.Job); ok {
		update.Job = &value
	}
	if value, ok := body["cron"].(string); ok {
		update.Cron = &value
	}
	if value, ok := body["timezone"].(string); ok {
		update.Timezone = &value
	}
	if value, ok := body["timeout_seconds"].(uint64); ok {
		update.TimeoutSeconds = &value
	}
	if value, ok := body["coalesce"].(bool); ok {
		update.Coalesce = &value
	}
	if value, ok := body["enabled"].(bool); ok {
		update.Enabled = &value
	}
	return update
}
