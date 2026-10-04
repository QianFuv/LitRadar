// Package recommend preserves candidate selection, message rendering and AI payload contracts.
package recommend

import (
	"log/slog"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
)

const (
	DefaultOpenaiBaseUrl = "https://api.siliconflow.cn/v1"
	DefaultOpenaiModel   = "deepseek-ai/DeepSeek-V3"
	PushplusChannel      = "wechat"
	MaxArticlesPerPush   = 20
	MaxPushContentLength = 18000
)

// GlobalConfig holds administrator-approved endpoint and notification defaults.
type GlobalConfig struct {
	AiBaseUrl         string   `json:"ai_base_url"`
	AiAllowedBaseUrls []string `json:"ai_allowed_base_urls"`
	AiApiKey          string   `json:"ai_api_key"`
	PushplusChannel   string   `json:"pushplus_channel"`
	PushplusTemplate  string   `json:"pushplus_template"`
	PushplusTopic     *string  `json:"pushplus_topic"`
	PushplusOption    *string  `json:"pushplus_option"`
	AiSystemPrompt    *string  `json:"ai_system_prompt"`
}

// String redacts integration secrets and administrator endpoint configuration.
func (config GlobalConfig) String() string { return "GlobalConfig([REDACTED])" }

// GoString redacts Go-syntax diagnostics.
func (config GlobalConfig) GoString() string { return config.String() }

// LogValue redacts structured diagnostics.
func (config GlobalConfig) LogValue() slog.Value { return slog.StringValue(config.String()) }

// Defaults holds the model candidate limit and sampling temperature.
type Defaults struct {
	MaxCandidates int     `json:"max_candidates"`
	AiModel       string  `json:"ai_model"`
	Temperature   float64 `json:"temperature"`
}

// AiRuntimeConfig is one resolved, allowlisted model endpoint.
type AiRuntimeConfig struct {
	BaseUrl      string `json:"base_url"`
	ApiKey       string `json:"api_key"`
	Model        string `json:"model"`
	SystemPrompt string `json:"system_prompt"`
}

// String redacts endpoint credentials and prompt content.
func (config AiRuntimeConfig) String() string { return "AiRuntimeConfig([REDACTED])" }

// GoString redacts Go-syntax diagnostics.
func (config AiRuntimeConfig) GoString() string { return config.String() }

// LogValue redacts structured diagnostics.
func (config AiRuntimeConfig) LogValue() slog.Value { return slog.StringValue(config.String()) }

// ResolveAiRuntimeConfigs resolves distinct primary and backup configurations, preserving explicit empty overrides.
func ResolveAiRuntimeConfigs(subscriber domain.Subscriber, global GlobalConfig, defaults Defaults, overrideModel *string) []AiRuntimeConfig {
	configs := []AiRuntimeConfig{}
	resolve := func(baseUrl, apiKey, model, prompt *string) {
		if overrideModel != nil {
			model = overrideModel
		}
		config := AiRuntimeConfig{strings.TrimSpace(orText(baseUrl, global.AiBaseUrl)), strings.TrimSpace(orText(apiKey, global.AiApiKey)), strings.TrimSpace(orText(model, defaults.AiModel)), strings.TrimSpace(orText(prompt, orText(global.AiSystemPrompt, "")))}
		if config.ApiKey == "" || config.Model == "" {
			return
		}
		isAllowed := false
		for _, allowed := range global.AiAllowedBaseUrls {
			if allowed == config.BaseUrl {
				isAllowed = true
				break
			}
		}
		if !isAllowed {
			return
		}
		for _, previous := range configs {
			if previous == config {
				return
			}
		}
		configs = append(configs, config)
	}
	resolve(subscriber.AiBaseUrl, subscriber.AiApiKey, subscriber.AiModel, subscriber.AiSystemPrompt)
	for _, override := range []*string{subscriber.AiBackupBaseUrl, subscriber.AiBackupApiKey, subscriber.AiBackupModel, subscriber.AiBackupSystemPrompt} {
		if override != nil && strings.TrimSpace(*override) != "" {
			resolve(subscriber.AiBackupBaseUrl, subscriber.AiBackupApiKey, subscriber.AiBackupModel, subscriber.AiBackupSystemPrompt)
			break
		}
	}
	return configs
}

func orText(value *string, fallback string) string {
	if value != nil {
		return *value
	}
	return fallback
}
