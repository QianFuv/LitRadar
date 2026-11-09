package scholarly

import (
	"encoding/json"
	"fmt"
	"log/slog"
	"strings"
)

// LiveConfig carries explicit worker credentials and a common scheduling epoch.
type LiveConfig struct {
	TimeoutSeconds                uint64   `json:"timeout_seconds"`
	OpenAlexApiKeys               []string `json:"openalex_api_keys"`
	SemanticScholarApiKeys        []string `json:"semantic_scholar_api_keys"`
	CrossrefMailtos               []string `json:"crossref_mailtos"`
	SemanticScholarWorkerId       uint64   `json:"semantic_scholar_worker_id"`
	SemanticScholarProcessCount   uint64   `json:"semantic_scholar_process_count"`
	SemanticScholarBaseIntervalMs uint64   `json:"semantic_scholar_base_interval_ms"`
	ScheduleEpochUnixMillis       uint64   `json:"schedule_epoch_unix_millis"`
}

// MarshalJSON emits empty credential pools as arrays for worker bootstrap compatibility.
func (config LiveConfig) MarshalJSON() ([]byte, error) {
	type plainConfig LiveConfig
	return json.Marshal(plainConfig(cloneConfig(config)))
}

// ConfigFromValuePools parses ordered pools without requiring any provider to be enabled.
func ConfigFromValuePools(timeout uint64, openAlex, semanticScholar, mailtos string) LiveConfig {
	return LiveConfig{TimeoutSeconds: timeout, OpenAlexApiKeys: ValuePool(openAlex), SemanticScholarApiKeys: ValuePool(semanticScholar), CrossrefMailtos: ValuePool(mailtos), SemanticScholarProcessCount: 1, SemanticScholarBaseIntervalMs: 1100}
}

// WithWorkerContext anchors this worker to a nonempty cohort.
func (config LiveConfig) WithWorkerContext(worker, count uint64) LiveConfig {
	config.SemanticScholarWorkerId = worker
	config.SemanticScholarProcessCount = max(count, 1)
	return config
}

// WithScheduleEpoch uses the common parent-issued epoch for all provider starts.
func (config LiveConfig) WithScheduleEpoch(epoch uint64) LiveConfig {
	config.ScheduleEpochUnixMillis = epoch
	return config
}

// HasOpenAlexKey reports whether any OpenAlex credential is nonblank.
func (config LiveConfig) HasOpenAlexKey() bool { return hasNonblank(config.OpenAlexApiKeys) }

// HasSemanticScholarKey reports whether any Semantic Scholar credential is nonblank.
func (config LiveConfig) HasSemanticScholarKey() bool {
	return hasNonblank(config.SemanticScholarApiKeys)
}

// HasCrossrefMailto reports whether any polite-pool address is nonblank.
func (config LiveConfig) HasCrossrefMailto() bool { return hasNonblank(config.CrossrefMailtos) }
func hasNonblank(values []string) bool {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return true
		}
	}
	return false
}
func (config LiveConfig) String() string {
	return fmt.Sprintf("LiveScholarlyConfig(timeout_seconds=%d, openalex_api_key_count=%d, semantic_scholar_api_key_count=%d, crossref_mailto_count=%d, credentials=[REDACTED])", config.TimeoutSeconds, len(config.OpenAlexApiKeys), len(config.SemanticScholarApiKeys), len(config.CrossrefMailtos))
}
func (config LiveConfig) GoString() string     { return config.String() }
func (config LiveConfig) LogValue() slog.Value { return slog.StringValue(config.String()) }

func cloneConfig(config LiveConfig) LiveConfig {
	config.OpenAlexApiKeys = append([]string{}, config.OpenAlexApiKeys...)
	config.SemanticScholarApiKeys = append([]string{}, config.SemanticScholarApiKeys...)
	config.CrossrefMailtos = append([]string{}, config.CrossrefMailtos...)
	return config
}
