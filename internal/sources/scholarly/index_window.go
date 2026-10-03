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

func planIndexPageWindow(window indexWindow, anchors []*Anchor, hasUnknown, hasNext bool) (indexPagePlan, error) {
	window = window.clone()
	selected := []int{}
	replay := func() indexPagePlan {
		unboundIndexWindow(&window)
		return indexPagePlan{[]int{}, window, "replay_unbounded"}
	}
	if window.Phase == "bounded" && hasUnknown {
		return replay(), nil
	}
	for index, anchor := range anchors {
		if window.Phase == "bounded" && anchor == nil {
			return replay(), nil
		}
		if window.CandidateAnchor == nil {
			if anchor != nil {
				if window.Phase == "bounded" && window.BaseAnchor != nil && issueIsOlder(anchor.Issue, window.BaseAnchor.Issue) {
					return replay(), nil
				}
				window.CandidateAnchor = copyAnchor(anchor)
				window.HasReachedCandidate = true
			}
		} else if !window.HasReachedCandidate {
			if anchor != nil && reflect.DeepEqual(anchor.Issue, window.CandidateAnchor.Issue) {
				window.HasReachedCandidate = true
			} else {
				continue
			}
		}
		if window.HasReachedCandidate && anchor != nil && window.CandidateAnchor != nil && !reflect.DeepEqual(anchor.Issue, window.CandidateAnchor.Issue) && issueIsOlder(window.CandidateAnchor.Issue, anchor.Issue) {
			continue
		}
		if window.Phase == "bounded" {
			if window.HasSeenBase && window.BaseAnchor != nil && !reflect.DeepEqual(anchor.Issue, window.BaseAnchor.Issue) {
				return indexPagePlan{selected, window, "complete"}, nil
			}
			if window.BaseAnchor != nil && reflect.DeepEqual(anchor.Issue, window.BaseAnchor.Issue) {
				window.HasSeenBase = true
			}
		}
		if window.CandidateAnchor == nil || window.HasReachedCandidate {
			selected = append(selected, index)
		}
	}
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
