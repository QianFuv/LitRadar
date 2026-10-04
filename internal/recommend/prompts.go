package recommend

import (
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
)

const selectionOutputContract = `Return exactly one JSON object with keys "summary" and "selected". "selected" must be an array of objects that each contain "article_id" and "score". Do not wrap JSON in markdown fences.`
const summaryOutputContract = `Return exactly one JSON object with the key "summary". Do not wrap JSON in markdown fences.`
const defaultSelectionPrompt = "You are a precise academic recommender. Use two-stage selection: directions-first filtering, then keyword-based ranking in the filtered set. Return relevant candidates ranked by score. Order selected items from highest to lowest. Judge by article content quality and topic relevance only. Ignore journal quality, prestige, and ranking completely. Do not invent article ids."
const summaryPromptSuffix = "Only summarize the supplied selected papers. Focus on major research themes, methods, and findings."

func selectionSystemPrompt(config AiRuntimeConfig) string {
	prompt := strings.TrimSpace(config.SystemPrompt)
	if prompt == "" {
		prompt = defaultSelectionPrompt
	}
	return prompt + "\n\n" + selectionOutputContract
}

func summarySystemPrompt(config AiRuntimeConfig) string {
	prompt := strings.TrimSpace(config.SystemPrompt)
	if prompt == "" {
		return "You are a precise academic summarizer. Only summarize the supplied selected papers. " + summaryOutputContract
	}
	return prompt + "\n\n" + summaryPromptSuffix + "\n\n" + summaryOutputContract
}

func subscriberPayload(subscriber domain.Subscriber) map[string]any {
	keywords, directions := make([]any, 0, len(subscriber.Keywords)), make([]any, 0, len(subscriber.Directions))
	for _, keyword := range subscriber.Keywords {
		keywords = append(keywords, keyword)
	}
	for _, direction := range subscriber.Directions {
		directions = append(directions, direction)
	}
	return map[string]any{"id": subscriber.SubscriberId, "name": subscriber.Name, "keywords": keywords, "directions": directions}
}

func candidatePayload(candidate storage.ArticleCandidate, isSelection bool) map[string]any {
	abstract := []rune(candidate.Abstract)
	var date any
	if candidate.Date != nil {
		date = *candidate.Date
	}
	payload := map[string]any{"article_id": candidate.ArticleId, "title": candidate.Title, "abstract": string(abstract[:min(len(abstract), 1200)]), "journal_title": candidate.JournalTitle, "date": date}
	if isSelection {
		var issue any
		if candidate.IssueId != nil {
			issue = *candidate.IssueId
		}
		payload["journal_id"], payload["issue_id"] = candidate.JournalId, issue
		payload["open_access"], payload["in_press"] = candidate.OpenAccess, candidate.InPress
	}
	return payload
}

func selectionRequest(subscriber domain.Subscriber, defaults Defaults, candidates []storage.ArticleCandidate) (any, any) {
	articles := make([]any, 0, len(candidates))
	for _, candidate := range candidates {
		articles = append(articles, candidatePayload(candidate, true))
	}
	schema := map[string]any{"type": "object", "properties": map[string]any{
		"summary":  map[string]any{"type": "string"},
		"selected": map[string]any{"type": "array", "items": map[string]any{"type": "object", "properties": map[string]any{"article_id": map[string]any{"type": "integer"}, "score": map[string]any{"type": "number"}}, "required": []any{"article_id", "score"}, "additionalProperties": false}},
	}, "required": []any{"summary", "selected"}, "additionalProperties": false}
	payload := map[string]any{
		"subscriber":          subscriberPayload(subscriber),
		"summary_requirement": "Summary must focus on the content of selected papers. Describe major research themes, methods, or findings in 2-4 sentences. Avoid generic recommendation language.",
		"selection_rules": map[string]any{
			"goal":             "Return ranked relevant candidates for this subscriber",
			"score_definition": "0 to 100, higher means better match and quality",
			"priority_order": []any{
				"First pass: directions-first filtering. When directions are provided, only keep candidates that clearly match at least one direction.",
				"Second pass: within the direction-matched subset, rank by keyword relevance.",
				"Third pass: break ties by methodological rigor, recency, and practical or theoretical contribution.",
			},
			"must_follow": []any{
				"Directions have higher priority than keywords. Do not elevate a keyword-only paper over a weaker direction-matched paper.",
				"If directions are non-empty and at least one candidate matches directions, do not select direction-mismatched papers.",
				"If directions are empty or no candidate matches directions, fallback to keyword relevance.",
			},
			"prefer": []any{"Article quality and methodological rigor", "Recent papers", "High conceptual overlap with subscriber goals", "Clear practical or theoretical contribution"},
			"avoid":  []any{"Low topical relevance", "Any preference based on journal prestige or ranking"},
		},
		"limits":             map[string]any{"max_candidates_input": defaults.MaxCandidates},
		"candidates":         articles,
		"output_instruction": "Return JSON only and strictly follow schema.",
	}
	return payload, schema
}

func summaryRequest(subscriber domain.Subscriber, candidates []storage.ArticleCandidate) (any, any) {
	articles := make([]any, 0, len(candidates))
	for _, candidate := range candidates {
		articles = append(articles, candidatePayload(candidate, false))
	}
	payload := map[string]any{"subscriber": subscriberPayload(subscriber), "selected_articles": articles, "instruction": "Summarize the content of these selected papers in 2-4 sentences. Focus on major research themes, methods, and findings. Avoid generic recommendation language."}
	schema := map[string]any{"type": "object", "properties": map[string]any{"summary": map[string]any{"type": "string"}}, "required": []any{"summary"}, "additionalProperties": false}
	return payload, schema
}
