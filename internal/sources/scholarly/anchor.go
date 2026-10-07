package scholarly

import (
	"fmt"
	"strconv"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// IssueFingerprint retains the original tagged issue boundary used by source checkpoints.
type IssueFingerprint struct {
	Kind            string
	PublicationYear *int64
	Volume, Issue   *string
	Date, Title     string
}
type volumeIssueWire struct {
	Kind            string  `json:"kind"`
	PublicationYear int64   `json:"publication_year"`
	Volume          *string `json:"volume,omitempty"`
	Issue           *string `json:"issue,omitempty"`
}
type dateIssueWire struct {
	Kind string `json:"kind"`
	Date string `json:"date"`
}
type titleIssueWire struct {
	Kind            string `json:"kind"`
	PublicationYear *int64 `json:"publication_year,omitempty"`
	Title           string `json:"title"`
}

// MarshalJSON preserves tagged variant field order and omitted optional labels.
func (issue IssueFingerprint) MarshalJSON() ([]byte, error) {
	switch issue.Kind {
	case "volume_issue":
		if issue.PublicationYear == nil {
			return nil, errWorksetJson
		}
		return encodeWorksetStruct(volumeIssueWire{issue.Kind, *issue.PublicationYear, issue.Volume, issue.Issue})
	case "date":
		return encodeWorksetStruct(dateIssueWire{issue.Kind, issue.Date})
	case "title":
		return encodeWorksetStruct(titleIssueWire{issue.Kind, issue.PublicationYear, issue.Title})
	default:
		return nil, errWorksetJson
	}
}

// UnmarshalJSON rejects unknown, duplicate and cross-variant fields.
func (issue *IssueFingerprint) UnmarshalJSON(body []byte) error {
	kind, err := worksetKind(body)
	if err != nil {
		return err
	}
	decoded := IssueFingerprint{Kind: kind}
	switch kind {
	case "volume_issue":
		var wire volumeIssueWire
		if err = decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.PublicationYear = &wire.PublicationYear
		decoded.Volume = wire.Volume
		decoded.Issue = wire.Issue
	case "date":
		var wire dateIssueWire
		if err = decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.Date = wire.Date
	case "title":
		var wire titleIssueWire
		if err = decodeWorksetStruct(body, &wire); err != nil {
			return err
		}
		decoded.PublicationYear = wire.PublicationYear
		decoded.Title = wire.Title
	default:
		return errWorksetJson
	}
	*issue = decoded
	return nil
}

// Anchor is the version-one successful scholarly issue boundary.
type Anchor struct {
	Version      uint32           `json:"version"`
	Issue        IssueFingerprint `json:"issue"`
	FromSyncDate *string          `json:"from_sync_date,omitempty"`
}

// MarshalJSON retains the original field order needed by durable checkpoint comparisons.
func (anchor Anchor) MarshalJSON() ([]byte, error) {
	type wire Anchor
	return encodeWorksetStruct(wire(anchor))
}

// UnmarshalJSON accepts strict struct maps and complete positional sequences.
func (anchor *Anchor) UnmarshalJSON(body []byte) error {
	type wire Anchor
	var decoded wire
	if err := decodeWorksetStruct(body, &decoded); err != nil {
		return err
	}
	*anchor = Anchor(decoded)
	return nil
}

// IsValid checks normalized identity without imposing a new consistency relation on the sync date.
func (anchor Anchor) IsValid() bool {
	if anchor.Version != 1 || anchor.FromSyncDate != nil && !validSortDate(*anchor.FromSyncDate) {
		return false
	}
	issue := anchor.Issue
	switch issue.Kind {
	case "volume_issue":
		return validVolumeIssue(issue)
	case "date":
		date := domain.NormalizeDate(issue.Date)
		return date != nil && date.Value == issue.Date
	case "title":
		return validTitleIssue(issue)
	default:
		return false
	}
}

// validTitleIssue retains optional-year and exact normalized nonempty title admission.
func validTitleIssue(issue IssueFingerprint) bool {
	return (issue.PublicationYear == nil || validYear(*issue.PublicationYear)) && issue.Title != "" && domain.NormalizeBibliographicText(issue.Title) == issue.Title
}
func validYear(year int64) bool { return year >= 1 && year <= 9999 }
func validSortDate(value string) bool {
	date := domain.NormalizeDate(value)
	return len(value) == 10 && date != nil && date.Value == value
}
func (anchor Anchor) clone() Anchor {
	anchor.FromSyncDate = clonePointer(anchor.FromSyncDate)
	anchor.Issue.PublicationYear = clonePointer(anchor.Issue.PublicationYear)
	anchor.Issue.Volume = clonePointer(anchor.Issue.Volume)
	anchor.Issue.Issue = clonePointer(anchor.Issue.Issue)
	return anchor
}
func copyAnchor(anchor *Anchor) *Anchor {
	if anchor == nil {
		return nil
	}
	copy := anchor.clone()
	return &copy
}
func jsonText(value any) *string {
	if text, ok := value.(string); ok {
		return domain.NormalizeText(text)
	}
	if number, ok := sourceNumber(value); ok {
		text := number.String()
		return &text
	}
	return nil
}
func signedNumber(value any) *int64 {
	number, ok := sourceNumber(value)
	if !ok {
		return nil
	}
	integer, ok := number.AsInt64()
	if !ok {
		return nil
	}
	return &integer
}

// CrossrefDate preserves the first integer-year candidate and its original calendar precision.
func CrossrefDate(work any) *string {
	for _, key := range []string{"published-online", "published-print", "published", "issued"} {
		dates := array(field(field(work, key), "date-parts"))
		if len(dates) == 0 {
			continue
		}
		if date, hasYear := crossrefDateCandidate(array(dates[0])); hasYear {
			return date
		}
	}
	return nil
}

// CrossrefIssueAnchor derives the same normalized issue identity used by persisted worksets.
func CrossrefIssueAnchor(work any) *Anchor {
	date := CrossrefDate(work)
	var year *int64
	if date != nil && len(*date) >= 4 {
		if parsed, err := strconv.ParseInt((*date)[:4], 10, 64); err == nil {
			year = &parsed
		}
	}
	return IssueAnchorFromFields(year, date, nil, jsonText(field(work, "volume")), jsonText(field(work, "issue")))
}

// IssueAnchorFromFields chooses year/volume/issue, then date, then normalized issue title.
func IssueAnchorFromFields(year *int64, date, title, volume, issue *string) *Anchor {
	volume = normalizeIssueLabel(volume)
	issue = normalizeIssueLabel(issue)
	fingerprint, ok := issueFingerprintFromFields(year, date, title, volume, issue)
	if !ok {
		return nil
	}
	syncYear := year
	if syncYear == nil || !validYear(*syncYear) {
		syncYear = fingerprint.PublicationYear
		if fingerprint.Kind == "date" && len(fingerprint.Date) >= 4 {
			if parsed, err := strconv.ParseInt(fingerprint.Date[:4], 10, 64); err == nil {
				syncYear = &parsed
			}
		}
	}
	anchor := &Anchor{Version: 1, Issue: fingerprint}
	if syncYear != nil {
		sync := fmt.Sprintf("%04d-01-01", *syncYear)
		anchor.FromSyncDate = &sync
	}
	return anchor
}

// validVolumeIssue checks the original year and optional label identity rules.
func validVolumeIssue(issue IssueFingerprint) bool {
	if issue.PublicationYear == nil || !validYear(*issue.PublicationYear) || issue.Volume == nil && issue.Issue == nil {
		return false
	}
	for _, label := range []*string{issue.Volume, issue.Issue} {
		if !validIssueLabel(label) {
			return false
		}
	}
	return true
}

// validIssueLabel accepts absent labels and requires exact nonempty normalization otherwise.
func validIssueLabel(label *string) bool {
	return label == nil || *label != "" && domain.NormalizeBibliographicLabel(*label) == *label
}

// crossrefDateCandidate stops fallback after the first integer year even when its calendar date is invalid.
func crossrefDateCandidate(parts []any) (*string, bool) {
	if len(parts) == 0 {
		return nil, false
	}
	year := signedNumber(parts[0])
	if year == nil {
		return nil, false
	}
	var month, day *int64
	if len(parts) > 1 {
		month = signedNumber(parts[1])
	}
	if len(parts) > 2 {
		day = signedNumber(parts[2])
	}
	candidate := fmt.Sprintf("%04d", *year)
	if month != nil {
		candidate += fmt.Sprintf("-%02d", *month)
		if day != nil {
			candidate += fmt.Sprintf("-%02d", *day)
		}
	}
	if date := domain.NormalizeDate(candidate); date != nil {
		return &date.Value, true
	}
	return nil, true
}

// normalizeIssueLabel owns a normalized nonempty optional issue label.
func normalizeIssueLabel(value *string) *string {
	if value == nil {
		return nil
	}
	label := domain.NormalizeBibliographicLabel(*value)
	if label == "" {
		return nil
	}
	return &label
}

// issueFingerprintFromFields preserves year-label, date and title precedence.
func issueFingerprintFromFields(year *int64, date, title, volume, issue *string) (IssueFingerprint, bool) {
	fingerprint := IssueFingerprint{}
	if year != nil && validYear(*year) && (volume != nil || issue != nil) {
		fingerprint = IssueFingerprint{Kind: "volume_issue", PublicationYear: clonePointer(year), Volume: volume, Issue: issue}
	} else if date != nil {
		fingerprint = IssueFingerprint{Kind: "date", Date: *date}
	} else if title != nil {
		normalized := domain.NormalizeBibliographicText(*title)
		if normalized == "" {
			return IssueFingerprint{}, false
		}
		fingerprint = IssueFingerprint{Kind: "title", Title: normalized}
		if year != nil && validYear(*year) {
			fingerprint.PublicationYear = clonePointer(year)
		}
	} else {
		return IssueFingerprint{}, false
	}
	return fingerprint, true
}
