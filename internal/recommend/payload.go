package recommend

import (
	"encoding/json"
	"errors"
	"math"
	"sort"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/transport"
)

// PayloadKind selects the normalizer for model selections or summaries.
type PayloadKind string

const (
	SelectionPayload PayloadKind = "selection"
	SummaryPayload   PayloadKind = "summary"
)

var errStructuredPayload = errors.New("Structured response is not a JSON object")

// ExtractResponsePayload normalizes the first OpenAI-compatible choice while honoring refusal and parsed-field precedence.
func ExtractResponsePayload(response any, kind PayloadKind) (map[string]any, error) {
	root, _ := response.(map[string]any)
	choices, _ := root["choices"].([]any)
	if len(choices) == 0 {
		return nil, errors.New("AI response missing choices")
	}
	choice, isObject := choices[0].(map[string]any)
	if !isObject {
		return nil, errors.New("AI response has invalid choice item")
	}
	message, isObject := choice["message"].(map[string]any)
	if !isObject {
		return nil, errors.New("AI response missing message")
	}
	if refusal, ok := message["refusal"].(string); ok && strings.TrimSpace(refusal) != "" {
		return nil, errors.New("AI model refused structured output")
	}
	if parsed, exists := message["parsed"]; exists {
		return normalizePayload(parsed, kind)
	}
	switch content := message["content"].(type) {
	case map[string]any:
		return normalizePayload(content, kind)
	case []any:
		var joined strings.Builder
		for _, item := range content {
			if object, ok := item.(map[string]any); ok {
				if text, ok := object["text"].(string); ok {
					joined.WriteString(text)
				}
			}
		}
		return normalizeContentText(joined.String(), kind)
	case string:
		return normalizeContentText(content, kind)
	}
	return nil, errors.New("AI message content is invalid")
}

func normalizeContentText(content string, kind PayloadKind) (map[string]any, error) {
	normalized := strings.TrimSpace(content)
	if strings.HasPrefix(normalized, "```") {
		lines := strings.Split(normalized, "\n")
		for index := range lines {
			lines[index] = strings.TrimSuffix(lines[index], "\r")
		}
		lines = lines[1:]
		if len(lines) > 0 && strings.HasPrefix(lines[len(lines)-1], "```") {
			lines = lines[:len(lines)-1]
		}
		normalized = strings.TrimSpace(strings.Join(lines, "\n"))
	}
	value, err := transport.ParseJson([]byte(normalized))
	if err != nil {
		value = normalized
	}
	return normalizePayload(value, kind)
}

func normalizePayload(value any, kind PayloadKind) (map[string]any, error) {
	if kind == SummaryPayload {
		return normalizeSummary(value)
	}
	if items, ok := value.([]any); ok {
		return map[string]any{"summary": "", "selected": coerceSelectedItems(items)}, nil
	}
	object, ok := value.(map[string]any)
	if !ok {
		return nil, errStructuredPayload
	}
	for _, key := range []string{"selected", "items", "results", "recommendations", "articles"} {
		if selected, exists := object[key]; exists {
			return map[string]any{"summary": extractSummary(object), "selected": coerceSelectedValue(selected)}, nil
		}
	}
	if articleId, ok := jsonInt64(object["article_id"]); ok {
		item := map[string]any{"article_id": articleId}
		if score, ok := jsonFloat64(object["score"]); ok {
			item["score"] = finiteScore(score)
		}
		return map[string]any{"summary": extractSummary(object), "selected": []any{item}}, nil
	}
	return nil, errStructuredPayload
}

func normalizeSummary(value any) (map[string]any, error) {
	var summary string
	switch item := value.(type) {
	case string:
		summary = strings.TrimSpace(item)
	case []any:
		texts := []string{}
		for _, item := range item {
			if text, ok := item.(string); ok && strings.TrimSpace(text) != "" {
				texts = append(texts, strings.TrimSpace(text))
			}
		}
		summary = strings.Join(texts, "\n")
	case map[string]any:
		summary = extractSummary(item)
		if summary == "" && len(item) == 1 {
			for _, value := range item {
				if text, ok := value.(string); ok {
					summary = strings.TrimSpace(text)
				}
			}
		}
	}
	if summary == "" {
		return nil, errStructuredPayload
	}
	return map[string]any{"summary": summary}, nil
}

func extractSummary(object map[string]any) string {
	for _, key := range []string{"summary", "message", "text", "analysis", "reason"} {
		if text, ok := object[key].(string); ok && strings.TrimSpace(text) != "" {
			return strings.TrimSpace(text)
		}
	}
	return ""
}

func coerceSelectedValue(value any) []any {
	if object, ok := value.(map[string]any); ok {
		keys := make([]string, 0, len(object))
		for key := range object {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		items := []any{}
		for _, key := range keys {
			articleId, err := strconv.ParseInt(key, 10, 64)
			if err != nil {
				continue
			}
			item := map[string]any{"article_id": articleId}
			if score, ok := jsonFloat64(object[key]); ok {
				item["score"] = finiteScore(score)
			}
			items = append(items, item)
		}
		return items
	}
	items, _ := value.([]any)
	return coerceSelectedItems(items)
}

func coerceSelectedItems(items []any) []any {
	result := []any{}
	for _, item := range items {
		if object, ok := item.(map[string]any); ok {
			result = append(result, object)
			continue
		}
		if articleId, ok := jsonInt64(item); ok {
			result = append(result, map[string]any{"article_id": articleId, "score": int64(0)})
			continue
		}
		if values, ok := item.([]any); ok && len(values) >= 2 {
			articleId, hasId := jsonInt64(values[0])
			score, hasScore := jsonFloat64(values[1])
			if hasId && hasScore {
				result = append(result, map[string]any{"article_id": articleId, "score": finiteScore(score)})
			}
		}
	}
	return result
}

func jsonInt64(value any) (int64, bool) {
	switch item := value.(type) {
	case string:
		number, err := strconv.ParseInt(item, 10, 64)
		return number, err == nil
	case json.Number:
		number, err := sources.ParseNumber(item)
		if err == nil {
			return number.AsInt64()
		}
	case sources.Number:
		return item.AsInt64()
	case int64:
		return item, true
	}
	return 0, false
}

func jsonFloat64(value any) (float64, bool) {
	switch item := value.(type) {
	case string:
		if item == "" || strings.TrimSpace(item) != item || strings.ContainsAny(item, "_xXpP") {
			return 0, false
		}
		unsigned := item
		if unsigned[0] == '+' || unsigned[0] == '-' {
			unsigned = unsigned[1:]
		}
		if strings.EqualFold(unsigned, "nan") {
			return math.NaN(), true
		}
		number, err := strconv.ParseFloat(item, 64)
		if err != nil {
			if numericError, ok := err.(*strconv.NumError); !ok || numericError.Err != strconv.ErrRange {
				return 0, false
			}
		}
		return number, true
	case json.Number:
		number, err := sources.ParseNumber(item)
		if err == nil {
			return number.AsFloat64(), true
		}
	case sources.Number:
		return item.AsFloat64(), true
	case float64:
		return item, true
	case int64:
		return float64(item), true
	}
	return 0, false
}

func finiteScore(value float64) any {
	if math.IsNaN(value) || math.IsInf(value, 0) {
		return nil
	}
	return value
}
