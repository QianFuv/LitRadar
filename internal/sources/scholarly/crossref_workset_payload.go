package scholarly

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strconv"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

func createdSecond(work any) (int64, error) {
	milliseconds := signedNumber(field(field(work, "created"), "timestamp"))
	if milliseconds == nil {
		return 0, invalidWorkset("Crossref work has no numeric creation timestamp")
	}
	second := *milliseconds / 1000
	if *milliseconds%1000 < 0 {
		second--
	}
	if !validTimestamp(second) {
		return 0, invalidWorkset("Crossref creation timestamp is out of range")
	}
	return second, nil
}
func consumedPayload(work any) map[string]any {
	payload := map[string]any{}
	object, _ := work.(map[string]any)
	for _, name := range []string{"DOI", "PMID", "title", "volume", "issue", "page", "abstract"} {
		if value, exists := object[name]; exists {
			payload[name] = cloneValue(value)
		}
	}
	if raw, ok := payload["DOI"].(string); ok {
		if doi := domain.NormalizeDoi(raw); doi != nil {
			payload["DOI"] = *doi
		}
	}
	for _, name := range []string{"published-online", "published-print", "published", "issued"} {
		if date, ok := object[name].(map[string]any); ok {
			if parts, exists := date["date-parts"]; exists {
				payload[name] = map[string]any{"date-parts": cloneValue(parts)}
			}
		}
	}
	if created, ok := object["created"].(map[string]any); ok {
		if timestamp, exists := created["timestamp"]; exists {
			payload["created"] = map[string]any{"timestamp": cloneValue(timestamp)}
		}
	}
	for _, projection := range []struct {
		name string
		keys []string
	}{{"author", []string{"given", "family"}}, {"updated-by", []string{"DOI", "type"}}} {
		if entries, ok := object[projection.name].([]any); ok {
			retained := make([]any, 0, len(entries))
			for _, entry := range entries {
				item, _ := entry.(map[string]any)
				selected := map[string]any{}
				for _, key := range projection.keys {
					if value, exists := item[key]; exists {
						selected[key] = cloneValue(value)
					}
				}
				retained = append(retained, selected)
			}
			payload[projection.name] = retained
		}
	}
	return payload
}
func workKey(work any, serialized string) string {
	if raw, ok := field(work, "DOI").(string); ok {
		if doi := domain.NormalizeDoi(raw); doi != nil {
			return "doi:" + *doi
		}
	}
	hash := sha256.Sum256([]byte(serialized))
	return "payload:" + hex.EncodeToString(hash[:])
}

type workOrder struct {
	Date, Fingerprint string
	Anchor            *string
	Year              int64
	Volume, Issue     string
}

func crossrefOrder(work any, key string) (workOrder, error) {
	order := workOrder{Year: -1}
	if date := CrossrefDate(work); date != nil {
		order.Date = *date
		switch len(order.Date) {
		case 4:
			order.Date += "-01-01"
		case 7:
			order.Date += "-01"
		}
	}
	if len(order.Date) >= 4 {
		if year, err := strconv.ParseInt(order.Date[:4], 10, 64); err == nil {
			order.Year = year
		}
	}
	anchor := CrossrefIssueAnchor(work)
	if anchor != nil {
		fingerprint, err := anchor.Issue.MarshalJSON()
		if err != nil {
			return workOrder{}, err
		}
		encoded, err := anchor.MarshalJSON()
		if err != nil {
			return workOrder{}, err
		}
		order.Fingerprint = string(fingerprint)
		serialized := string(encoded)
		order.Anchor = &serialized
	} else {
		order.Fingerprint = "unknown:" + key
	}
	label := func(name string) string {
		value, ok := field(work, name).(string)
		if !ok {
			return ""
		}
		trimmed := strings.TrimSpace(value)
		if strings.HasPrefix(trimmed, "+") {
			trimmed = trimmed[1:]
		}
		number, err := strconv.ParseUint(trimmed, 10, 64)
		if err != nil {
			return ""
		}
		return fmt.Sprintf("%020d", number)
	}
	order.Volume = label("volume")
	order.Issue = label("issue")
	return order, nil
}
