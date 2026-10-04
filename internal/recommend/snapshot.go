package recommend

import (
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
)

// Snapshot stores the exact issue and in-press count maps used by legacy delivery state.
type Snapshot struct {
	IssueArticleCounts   map[string]int64 `json:"issue_article_counts"`
	InpressArticleCounts map[string]int64 `json:"inpress_article_counts"`
}

// ComputeChangedIssueKeys retains current additions and changed counts in numeric issue order.
func ComputeChangedIssueKeys(previous, current map[string]int64) []string {
	changed := changedKeys(previous, current)
	sort.SliceStable(changed, func(left, right int) bool {
		leftJournal, leftIssue, _ := parseIssueKey(changed[left])
		rightJournal, rightIssue, _ := parseIssueKey(changed[right])
		if leftJournal != rightJournal {
			return leftJournal < rightJournal
		}
		return leftIssue < rightIssue
	})
	return changed
}

// ComputeChangedInpressKeys retains current additions and changed counts in numeric journal order.
func ComputeChangedInpressKeys(previous, current map[string]int64) []string {
	changed := changedKeys(previous, current)
	sort.SliceStable(changed, func(left, right int) bool { return integerOrZero(changed[left]) < integerOrZero(changed[right]) })
	return changed
}

func changedKeys(previous, current map[string]int64) []string {
	changed := []string{}
	for key, count := range current {
		if old, exists := previous[key]; !exists || old != count {
			changed = append(changed, key)
		}
	}
	sort.Strings(changed)
	return changed
}

func parseIssueKey(key string) (int64, int64, bool) {
	journal, issue, exists := strings.Cut(key, ":")
	if !exists {
		return 0, 0, false
	}
	journalId, firstError := strconv.ParseInt(journal, 10, 64)
	issueId, secondError := strconv.ParseInt(issue, 10, 64)
	if firstError != nil || secondError != nil {
		return 0, 0, false
	}
	return journalId, issueId, true
}

func integerOrZero(value string) int64 {
	result, err := strconv.ParseInt(value, 10, 64)
	if err != nil {
		return 0
	}
	return result
}

// IsDatabaseSelected compares the platform's basename and case-sensitive SQLite suffix.
func IsDatabaseSelected(selected []string, dbName string) bool {
	target := normalizeDbName(dbName)
	if target == "" {
		return false
	}
	if len(selected) == 0 {
		return true
	}
	for _, candidate := range selected {
		if normalizeDbName(candidate) == target {
			return true
		}
	}
	return false
}

func normalizeDbName(value string) string {
	value = strings.TrimSpace(value)
	if runtime.GOOS == "windows" && strings.HasPrefix(value, `\\?\`) {
		value = strings.TrimRight(value, `\`)
		volume := filepath.VolumeName(value)
		if value == volume {
			return ""
		}
		_, name, exists := strings.Cut(value[len(volume):], `\`)
		if !exists {
			name = value[len(volume):]
		}
		if index := strings.LastIndex(name, `\`); index >= 0 {
			name = name[index+1:]
		}
		if name == "" || name == "." || name == ".." {
			return ""
		}
		if strings.HasSuffix(name, ".sqlite") {
			return name
		}
		return name + ".sqlite"
	}
	for {
		value = strings.TrimRightFunc(value, func(character rune) bool { return character < 128 && os.IsPathSeparator(uint8(character)) })
		if value == "" {
			return ""
		}
		name := filepath.Base(value)
		if name == "." && value != "." {
			value = strings.TrimSuffix(value, ".")
			continue
		}
		if name == "." || name == ".." || name == filepath.VolumeName(value) {
			return ""
		}
		if strings.HasSuffix(name, ".sqlite") {
			return name
		}
		return name + ".sqlite"
	}
}
