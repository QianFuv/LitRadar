package scholarly

import (
	"reflect"
	"strconv"
)

type indexPagePlan struct {
	SelectedIndices []int
	Window          indexWindow
	Progress        string
}

func fingerprintYear(issue IssueFingerprint) *int64 {
	if issue.Kind == "date" {
		if len(issue.Date) < 4 {
			return nil
		}
		if year, err := strconv.ParseInt(issue.Date[:4], 10, 64); err == nil {
			return &year
		}
		return nil
	}
	return clonePointer(issue.PublicationYear)
}

func numericLabelIsOlder(candidate, base *string) *bool {
	if candidate == nil || base == nil {
		return nil
	}
	parse := func(value string) (uint64, error) {
		if len(value) > 0 && value[0] == '+' {
			value = value[1:]
		}
		return strconv.ParseUint(value, 10, 64)
	}
	first, err := parse(*candidate)
	if err != nil {
		return nil
	}
	second, err := parse(*base)
	if err != nil || first == second {
		return nil
	}
	return new(first < second)
}

func issueIsOlder(candidate, base IssueFingerprint) bool {
	first, second := fingerprintYear(candidate), fingerprintYear(base)
	if first != nil && second != nil && *first != *second {
		return *first < *second
	}
	if candidate.Kind == "date" && base.Kind == "date" {
		return candidate.Date < base.Date
	}
	if candidate.Kind == "volume_issue" && base.Kind == "volume_issue" {
		if older := numericLabelIsOlder(candidate.Volume, base.Volume); older != nil {
			return *older
		}
		if older := numericLabelIsOlder(candidate.Issue, base.Issue); older != nil {
			return *older
		}
	}
	return false
}

// planIndexPageWindow owns the copied traversal state and selected source positions.
func planIndexPageWindow(window indexWindow, anchors []*Anchor, hasUnknown, hasNext bool) (indexPagePlan, error) {
	window = window.clone()
	selected := []int{}
	if window.Phase == "bounded" && hasUnknown {
		return replayIndexWindow(window), nil
	}
	for index, anchor := range anchors {
		if window.Phase == "bounded" && anchor == nil {
			return replayIndexWindow(window), nil
		}
		shouldSelect, shouldComplete, shouldReplay := processIndexWindowAnchor(&window, anchor)
		if shouldReplay {
			return replayIndexWindow(window), nil
		}
		if shouldComplete {
			return indexPagePlan{selected, window, "complete"}, nil
		}
		if shouldSelect {
			selected = append(selected, index)
		}
	}
	return finishIndexWindow(window, selected, hasNext)
}

// replayIndexWindow discards partial selections when bounded issue identity is unsafe.
func replayIndexWindow(window indexWindow) indexPagePlan {
	unboundIndexWindow(&window)
	return indexPagePlan{[]int{}, window, "replay_unbounded"}
}

// advanceIndexCandidate freezes the first candidate and skips replay rows until it is reached.
func advanceIndexCandidate(window *indexWindow, anchor *Anchor) (shouldReplay, shouldSkip bool) {
	if window.CandidateAnchor == nil {
		if anchor != nil {
			if window.Phase == "bounded" && window.BaseAnchor != nil && issueIsOlder(anchor.Issue, window.BaseAnchor.Issue) {
				return true, false
			}
			window.CandidateAnchor = copyAnchor(anchor)
			window.HasReachedCandidate = true
		}
	} else if !window.HasReachedCandidate {
		if anchor != nil && reflect.DeepEqual(anchor.Issue, window.CandidateAnchor.Issue) {
			window.HasReachedCandidate = true
		} else {
			return false, true
		}
	}
	return false, false
}

// isNewerThanIndexCandidate skips out-of-order newer issues after the frozen candidate is reached.
func isNewerThanIndexCandidate(window indexWindow, anchor *Anchor) bool {
	return window.HasReachedCandidate && anchor != nil && window.CandidateAnchor != nil && !reflect.DeepEqual(anchor.Issue, window.CandidateAnchor.Issue) && issueIsOlder(window.CandidateAnchor.Issue, anchor.Issue)
}

// crossIndexBase completes only after leaving an observed bounded base issue.
func crossIndexBase(window *indexWindow, anchor *Anchor) bool {
	if window.HasSeenBase && window.BaseAnchor != nil && !reflect.DeepEqual(anchor.Issue, window.BaseAnchor.Issue) {
		return true
	}
	if window.BaseAnchor != nil && reflect.DeepEqual(anchor.Issue, window.BaseAnchor.Issue) {
		window.HasSeenBase = true
	}
	return false
}

// processIndexWindowAnchor advances candidate and base markers before selecting one source position.
func processIndexWindowAnchor(window *indexWindow, anchor *Anchor) (shouldSelect, shouldComplete, shouldReplay bool) {
	shouldReplay, shouldSkip := advanceIndexCandidate(window, anchor)
	if shouldReplay || shouldSkip {
		return false, false, shouldReplay
	}
	if isNewerThanIndexCandidate(*window, anchor) {
		return false, false, false
	}
	if window.Phase == "bounded" && crossIndexBase(window, anchor) {
		return false, true, false
	}
	return window.CandidateAnchor == nil || window.HasReachedCandidate, false, false
}

// finishIndexWindow preserves terminal fallback and missing frozen-candidate errors.
func finishIndexWindow(window indexWindow, selected []int, hasNext bool) (indexPagePlan, error) {
	if hasNext {
		return indexPagePlan{selected, window, "continue"}, nil
	}
	progress := "complete"
	if window.Phase == "bounded" && !window.HasSeenBase {
		unboundIndexWindow(&window)
		progress = "replay_unbounded"
	} else if window.Phase == "unbounded" && window.CandidateAnchor != nil && !window.HasReachedCandidate {
		return indexPagePlan{}, invalidWorkset("scholarly frozen candidate issue disappeared during replay")
	}
	return indexPagePlan{selected, window, progress}, nil
}
