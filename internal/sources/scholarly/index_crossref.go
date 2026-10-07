package scholarly

import (
	"context"
	"errors"
	"math"
	"reflect"
	"slices"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

func (index *IndexProvider) startWorkset(catalog domain.JournalCatalogEntry, window indexWindow, issn string, frozen int64, createdFrom *int64) (domain.ProviderBatch, error) {
	state, err := NewCrossrefCheckpoint(issn, frozen, indexWindowFilter(window, indexDate(frozen)))
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	state.Candidate = copyAnchor(window.CandidateAnchor)
	if createdFrom != nil {
		state.CreatedFrom = clonePointer(createdFrom)
		state.restartCollection()
	}
	scope, err := indexWorksetScope(catalog, window)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	workset, err := CreateCrossrefWorkset(index.worksetDir, scope, state)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	defer workset.Close()
	return continueIndexBatch(catalog, window, indexSource{Kind: "crossref_workset", State: new(workset.Checkpoint())}, nil)
}

func replayedCrossrefBatch(catalog domain.JournalCatalogEntry, window indexWindow, state CrossrefCheckpoint) (domain.ProviderBatch, error) {
	window.CandidateAnchor = copyAnchor(state.Candidate)
	prepareIndexReplay(&window)
	if state.Phase.Kind == "emit" {
		if state.Phase.Lower == nil {
			unboundIndexWindow(&window)
		}
		window.HasReachedCandidate = state.Candidate != nil
		window.HasSeenBase = window.Phase == "bounded"
	}
	return continueIndexBatch(catalog, window, indexSource{Kind: "crossref_workset", State: &state}, nil)
}

func (index *IndexProvider) replayCrossref(catalog domain.JournalCatalogEntry, window indexWindow, state CrossrefCheckpoint, workset *CrossrefWorkset) (domain.ProviderBatch, error) {
	unboundIndexWindow(&window)
	if err := workset.Discard(); err != nil {
		return domain.ProviderBatch{}, err
	}
	return index.startWorkset(catalog, window, state.Issn, state.FrozenAt, state.CreatedFrom)
}

func (index *IndexProvider) nextJournalSource(ctx context.Context, catalog domain.JournalCatalogEntry, window indexWindow, state CrossrefCheckpoint) (domain.ProviderBatch, error) {
	issns := domain.CatalogIssns(catalog)
	position := slices.Index(issns, state.Issn)
	if position >= 0 && position+1 < len(issns) {
		return index.startWorkset(catalog, window, issns[position+1], state.FrozenAt, nil)
	}
	return index.fetchOpenAlex(ctx, catalog, window, nil)
}

func (index *IndexProvider) fetchWorkset(ctx context.Context, catalog domain.JournalCatalogEntry, window indexWindow, state CrossrefCheckpoint) (domain.ProviderBatch, error) {
	if !slices.Contains(domain.CatalogIssns(catalog), state.Issn) {
		return domain.ProviderBatch{}, invalidWorkset("Crossref checkpoint ISSN does not belong to the catalog")
	}
	if !reflect.DeepEqual(state.Candidate, window.CandidateAnchor) {
		return domain.ProviderBatch{}, invalidWorkset("Crossref checkpoint candidate does not match the frozen window")
	}
	if !reflect.DeepEqual(state.UpdatedFrom, indexWindowFilter(window, indexDate(state.FrozenAt))) {
		return domain.ProviderBatch{}, invalidWorkset("Crossref checkpoint update bounds do not match the frozen window")
	}
	scope, err := indexWorksetScope(catalog, window)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	workset, replay, err := OpenCrossrefWorkset(index.worksetDir, scope, state)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	defer workset.Close()
	if replay {
		return replayedCrossrefBatch(catalog, window, workset.Checkpoint())
	}
	switch state.Phase.Kind {
	case "discover", "collect":
		return index.collectIndexWorkset(ctx, catalog, window, state, workset)
	case "ready":
		return index.selectWorkset(ctx, catalog, window, state, workset)
	case "emit":
		return index.emitIndexWorkset(ctx, catalog, window, state, workset)
	default:
		return domain.ProviderBatch{}, invalidWorkset("Crossref checkpoint phase is inconsistent")
	}
}

func (index *IndexProvider) selectWorkset(ctx context.Context, catalog domain.JournalCatalogEntry, window indexWindow, state CrossrefCheckpoint, workset *CrossrefWorkset) (domain.ProviderBatch, error) {
	if isEmptyDiscoveredWorkset(state) {
		if err := workset.Discard(); err != nil {
			return domain.ProviderBatch{}, err
		}
		return index.nextJournalSource(ctx, catalog, window, state)
	}
	candidate, window, shouldReplay, err := selectWorksetCandidate(workset, window, state)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	if shouldReplay {
		return index.replayCrossref(catalog, window, state, workset)
	}
	base, upper, err := selectWorksetGroupBounds(workset, window.BaseAnchor, candidate)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	window, lower, shouldReplay, err := selectWorksetLowerBound(workset, window, state, base, candidate, upper)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	if shouldReplay {
		return index.replayCrossref(catalog, window, state, workset)
	}
	window.HasReachedCandidate = window.CandidateAnchor != nil
	next, err := workset.SealSelection(upper, lower, window.CandidateAnchor)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	return continueIndexBatch(catalog, window, indexSource{Kind: "crossref_workset", State: &next}, nil)
}

// collectIndexWorkset fetches and accepts a collection page before emitting a continuation.
func (index *IndexProvider) collectIndexWorkset(ctx context.Context, catalog domain.JournalCatalogEntry, window indexWindow, state CrossrefCheckpoint, workset *CrossrefWorkset) (domain.ProviderBatch, error) {
	query, err := state.Query()
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	page, err := index.client.FetchCrossrefPage(ctx, state.Issn, query)
	if err != nil {
		if isMissingCrossrefDiscovery(err, state.Phase.Kind) {
			if err := workset.Discard(); err != nil {
				return domain.ProviderBatch{}, err
			}
			return index.nextJournalSource(ctx, catalog, window, state)
		}
		return domain.ProviderBatch{}, mapIndexSourceError(err)
	}
	next, err := workset.Accept(page)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	return continueIndexBatch(catalog, window, indexSource{Kind: "crossref_workset", State: &next}, nil)
}

// emitIndexWorkset enriches one immutable emission page before continuation or disposal.
func (index *IndexProvider) emitIndexWorkset(ctx context.Context, catalog domain.JournalCatalogEntry, window indexWindow, state CrossrefCheckpoint, workset *CrossrefWorkset) (domain.ProviderBatch, error) {
	if err := validateWorksetEmissionCandidate(workset, window.CandidateAnchor, state.Phase.Upper); err != nil {
		return domain.ProviderBatch{}, err
	}
	page, err := workset.Emit(state.Phase.Upper, state.Phase.Lower, state.Phase.After)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	articles, err := index.enrichCrossref(ctx, catalog, page.Works)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	if len(articles) != len(page.Works) {
		return domain.ProviderBatch{}, invalidWorkset("Crossref selected work cannot be represented without losing its identity")
	}
	if page.HasMore {
		if state.Sequence == math.MaxUint64 {
			return domain.ProviderBatch{}, invalidWorkset("Crossref sequence overflow")
		}
		state.Sequence++
		state.Phase.After = page.After
		return continueIndexBatch(catalog, window, indexSource{Kind: "crossref_workset", State: &state}, articles)
	}
	batch, err := completeIndexBatch(catalog, window, articles)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	if err := workset.Discard(); err != nil {
		return domain.ProviderBatch{}, err
	}
	return batch, nil
}

// isMissingCrossrefDiscovery switches journal sources only for discovery HTTP 404.
func isMissingCrossrefDiscovery(err error, phase string) bool {
	var failure *Error
	return errors.As(err, &failure) && failure.Kind == "HttpStatus" && failure.StatusCode == 404 && phase == "discover"
}

// validateWorksetEmissionCandidate checks frozen issue identity before loading an emission page.
func validateWorksetEmissionCandidate(workset *CrossrefWorkset, candidate *Anchor, upper string) error {
	if candidate != nil {
		group, err := workset.Group(*candidate)
		if err != nil {
			return err
		}
		if group == nil || group.Date != upper {
			return invalidWorkset("Crossref emission candidate does not match its workset")
		}
	}
	return nil
}

// selectWorksetCandidate resolves the frozen candidate or selects the newest verified group.
func selectWorksetCandidate(workset *CrossrefWorkset, window indexWindow, state CrossrefCheckpoint) (*IssueGroup, indexWindow, bool, error) {
	var candidate *IssueGroup
	var err error
	if window.CandidateAnchor != nil {
		candidate, err = workset.Group(*window.CandidateAnchor)
	} else {
		candidate, err = workset.FirstGroup()
	}
	if err != nil {
		return nil, window, false, err
	}
	if window.CandidateAnchor != nil && candidate == nil {
		if window.Phase == "bounded" && state.UpdatedFrom != nil {
			return nil, window, true, nil
		}
		return nil, window, false, invalidWorkset("Crossref frozen candidate is missing after unbounded collection")
	}
	if window.CandidateAnchor == nil && candidate != nil {
		window.CandidateAnchor = copyAnchor(&candidate.Anchor)
	}
	return candidate, window, false, nil
}

// selectWorksetGroupBounds resolves the base group before deriving the inclusive upper date.
func selectWorksetGroupBounds(workset *CrossrefWorkset, baseAnchor *Anchor, candidate *IssueGroup) (*IssueGroup, string, error) {
	var base *IssueGroup
	if baseAnchor != nil {
		var err error
		base, err = workset.Group(*baseAnchor)
		if err != nil {
			return nil, "", err
		}
	}
	upper := "9999-12-31"
	if candidate != nil {
		upper = candidate.Date
	}
	return base, upper, nil
}

// selectWorksetLowerBound replays unsafe filtered collection or widens an already-unfiltered selection.
func selectWorksetLowerBound(workset *CrossrefWorkset, window indexWindow, state CrossrefCheckpoint, base, candidate *IssueGroup, upper string) (indexWindow, *string, bool, error) {
	var lower *string
	if window.Phase == "bounded" {
		unknown, err := workset.HasUnknownGroups()
		if err != nil {
			return window, nil, false, err
		}
		isUnsafe := isUnsafeWorksetSelection(unknown, base, candidate, upper, window)
		if isUnsafe {
			if state.UpdatedFrom != nil {
				return window, nil, true, nil
			}
			unboundIndexWindow(&window)
		} else {
			lower = &base.Date
			window.HasSeenBase = true
		}
	}
	return window, lower, false, nil
}

// isUnsafeWorksetSelection preserves unknown, missing-group and reversed-window checks.
func isUnsafeWorksetSelection(unknown bool, base, candidate *IssueGroup, upper string, window indexWindow) bool {
	return unknown || base == nil || candidate == nil || base.Date > upper || window.CandidateAnchor != nil && window.BaseAnchor != nil && issueIsOlder(window.CandidateAnchor.Issue, window.BaseAnchor.Issue)
}

// isEmptyDiscoveredWorkset distinguishes an empty discovery from a verified bounded zero count.
func isEmptyDiscoveredWorkset(state CrossrefCheckpoint) bool {
	return state.RootTotal != nil && *state.RootTotal == 0 && state.CreatedFrom == nil
}
