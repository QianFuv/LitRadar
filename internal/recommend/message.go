package recommend

import (
	"fmt"
	"strings"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/storage"
)

// BuildMessageTitle retains the first ten Unicode scalar values of the run identifier.
func BuildMessageTitle(dbName, runId string) string {
	prefix := []rune(runId)
	if len(prefix) > 10 {
		prefix = prefix[:10]
	}
	return fmt.Sprintf("LitRadar Weekly Update [%s] %s", dbName, string(prefix))
}

// BuildMarkdownContent preserves the original whole-section byte budget and fallback header truncation.
func BuildMarkdownContent(dbName, runId string, subscriber domain.Subscriber, summary string, selections []domain.RankedSelection, candidates map[int64]storage.ArticleCandidate) string {
	base := []string{"## Weekly Digest for " + subscriber.Name, "", "- Database: `" + dbName + "`", "- Run ID: `" + runId + "`"}
	if summary = strings.TrimSpace(summary); summary != "" {
		base = append(base, "", summary)
	}
	sections := []string{}
	for position, selection := range selections {
		if position >= MaxArticlesPerPush {
			break
		}
		candidate, exists := candidates[selection.ArticleId]
		if !exists {
			continue
		}
		sections = append(sections, markdownArticleSection(candidate, len(sections)+1))
	}
	kept := []string{}
	for _, section := range sections {
		trial := append(append([]string{}, kept...), section)
		if len(renderContent(base, trial)) <= MaxPushContentLength {
			kept = trial
		}
	}
	content := renderContent(base, kept)
	if len(content) <= MaxPushContentLength {
		return content
	}
	header := renderContent(base, nil)
	characters := []rune(header)
	if len(characters) > MaxPushContentLength {
		characters = characters[:MaxPushContentLength]
	}
	return string(characters)
}

func renderContent(base, sections []string) string {
	header := append(append([]string{}, base...), fmt.Sprintf("- Selected Articles: %d", len(sections)))
	parts := []string{strings.TrimSpace(strings.Join(header, "\n"))}
	for _, section := range sections {
		if strings.TrimSpace(section) != "" {
			parts = append(parts, section)
		}
	}
	return strings.TrimSpace(strings.Join(parts, "\n\n"))
}

func markdownArticleSection(candidate storage.ArticleCandidate, position int) string {
	doi := orText(candidate.Doi, "N/A")
	title := candidate.Title
	if strings.TrimSpace(title) == "" {
		title = "Title unavailable (DOI: " + doi + ")"
	}
	abstract := strings.TrimSpace(candidate.Abstract)
	if abstract == "" {
		abstract = "N/A"
	}
	return fmt.Sprintf("### %d. %s\n- Journal: %s\n- Date: %s\n- DOI: %s\n- Abstract: %s", position, title, candidate.JournalTitle, orText(candidate.Date, "Unknown"), doi, abstract)
}
