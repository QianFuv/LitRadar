package recommend

import (
	"math"
	"sort"
	"strconv"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/domain/storage"
)

// CandidateMatchScore counts each matching keyword or direction, including repeated preferences.
func CandidateMatchScore(candidate storage.ArticleCandidate, subscriber domain.Subscriber) int64 {
	text := sources.Lowercase(candidate.Title + " " + candidate.Abstract)
	var score int64
	for _, phrases := range [][]string{subscriber.Keywords, subscriber.Directions} {
		for _, phrase := range phrases {
			phrase = sources.Lowercase(strings.TrimSpace(phrase))
			if phrase != "" && strings.Contains(text, phrase) {
				score++
			}
		}
	}
	return score
}

// HasSelectionPreferences reports whether any nonblank preference was configured.
func HasSelectionPreferences(subscriber domain.Subscriber) bool {
	for _, phrases := range [][]string{subscriber.Keywords, subscriber.Directions} {
		for _, phrase := range phrases {
			if strings.TrimSpace(phrase) != "" {
				return true
			}
		}
	}
	return false
}

// DeduplicateCandidates retains the first occurrence of each article identifier.
func DeduplicateCandidates(candidates []storage.ArticleCandidate) []storage.ArticleCandidate {
	result := []storage.ArticleCandidate{}
	seen := map[int64]bool{}
	for _, candidate := range candidates {
		if !seen[candidate.ArticleId] {
			seen[candidate.ArticleId] = true
			result = append(result, candidate)
		}
	}
	return result
}

// CandidatesById preserves the last duplicate, matching the original ordered-map collection.
func CandidatesById(candidates []storage.ArticleCandidate) map[int64]storage.ArticleCandidate {
	result := make(map[int64]storage.ArticleCandidate, len(candidates))
	for _, candidate := range candidates {
		result[candidate.ArticleId] = candidate
	}
	return result
}

// DeliveryKey identifies one subscriber/article dedupe reservation.
func DeliveryKey(subscriber domain.Subscriber, articleId int64) string {
	return subscriber.SubscriberId + ":" + strconv.FormatInt(articleId, 10)
}

// ApplySelectionRules filters stale and delivered identifiers, supplements matching candidates, and stably ranks at most twenty entries.
func ApplySelectionRules(result domain.SelectionResult, subscriber domain.Subscriber, candidates map[int64]storage.ArticleCandidate, dedupe map[string]string) []domain.RankedSelection {
	eligible := []domain.RankedSelection{}
	selected := map[int64]bool{}
	var preferences []string
	scoreCandidate := func(candidate storage.ArticleCandidate) int64 {
		if preferences == nil {
			preferences = make([]string, 0, len(subscriber.Keywords)+len(subscriber.Directions))
			for _, phrases := range [][]string{subscriber.Keywords, subscriber.Directions} {
				for _, phrase := range phrases {
					if normalized := sources.Lowercase(strings.TrimSpace(phrase)); normalized != "" {
						preferences = append(preferences, normalized)
					}
				}
			}
		}
		if len(preferences) == 0 {
			return 0
		}
		text := sources.Lowercase(candidate.Title + " " + candidate.Abstract)
		var score int64
		for _, phrase := range preferences {
			if strings.Contains(text, phrase) {
				score++
			}
		}
		return score
	}
	matchScores := map[int64]int64{}
	matchScore := func(articleId int64) int64 {
		if score, exists := matchScores[articleId]; exists {
			return score
		}
		score := scoreCandidate(candidates[articleId])
		matchScores[articleId] = score
		return score
	}
	isDelivered := func(articleId int64) bool { _, exists := dedupe[DeliveryKey(subscriber, articleId)]; return exists }
	for _, selection := range result.Selections {
		candidate, exists := candidates[selection.ArticleId]
		if !exists || isDelivered(candidate.ArticleId) {
			continue
		}
		eligible = append(eligible, selection)
		selected[selection.ArticleId] = true
	}
	if len(eligible) < MaxArticlesPerPush && HasSelectionPreferences(subscriber) {
		supplemental := []domain.RankedSelection{}
		for articleId, candidate := range candidates {
			if selected[candidate.ArticleId] || isDelivered(candidate.ArticleId) {
				continue
			}
			if score := scoreCandidate(candidate); score > 0 {
				matchScores[articleId] = score
				supplemental = append(supplemental, domain.RankedSelection{ArticleId: candidate.ArticleId})
			}
		}
		sort.Slice(supplemental, func(left, right int) bool {
			leftMatch, rightMatch := matchScore(supplemental[left].ArticleId), matchScore(supplemental[right].ArticleId)
			if leftMatch != rightMatch {
				return leftMatch > rightMatch
			}
			return supplemental[left].ArticleId > supplemental[right].ArticleId
		})
		eligible = append(eligible, supplemental...)
	}
	sort.SliceStable(eligible, func(left, right int) bool {
		leftMatch, rightMatch := matchScore(eligible[left].ArticleId), matchScore(eligible[right].ArticleId)
		if leftMatch != rightMatch {
			return leftMatch > rightMatch
		}
		return orderedScore(eligible[left].Score) > orderedScore(eligible[right].Score)
	})
	if len(eligible) > MaxArticlesPerPush {
		eligible = eligible[:MaxArticlesPerPush]
	}
	return eligible
}

func orderedScore(score float64) int64 {
	score = math.Round(score * 1_000_000)
	if math.IsNaN(score) {
		return 0
	}
	if score >= float64(math.MaxInt64) {
		return math.MaxInt64
	}
	if score <= float64(math.MinInt64) {
		return math.MinInt64
	}
	return int64(score)
}
