package cfp

import (
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/cron"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

// CleanText preserves paragraph boundaries and original language.
func CleanText(value string) string {
	value = strings.ReplaceAll(strings.ReplaceAll(value, "\r\n", "\n"), "\r", "\n")
	return strings.TrimSpace(expression(`[\t \x{00a0}]+`).ReplaceAllString(value, " "))
}

// IsSourceUrl checks the original WHATWG URL syntax without performing network access.
func IsSourceUrl(value string) bool {
	parsed, err := whatwg.NewParser().Parse(value)
	return err == nil && (parsed.Scheme() == "http" || parsed.Scheme() == "https") && parsed.Hostname() != "" && parsed.Username() == "" && parsed.Password() == ""
}

// ParseSource validates a literal source and retains unresolved archival timelines.
func ParseSource(source Source) *Notice {
	var kind Kind
	switch {
	case matches(`proposal|提案|专刊建议`, source.TypeText):
		kind = ProposalKind
	case matches(`conference|workshop|会议|研讨会|工作坊`, source.TypeText):
		kind = ConferenceLinked
	case matches(`special|专[题刊栏辑]|特[刊辑]`, source.TypeText):
		kind = SpecialIssue
	case matches(`general|regular|常规|年度|重点选题`, source.TypeText):
		kind = General
	default:
		return nil
	}
	stage := Paper
	if kind == ProposalKind {
		stage = Proposal
	}
	if source.EntryStage != nil {
		stage = *source.EntryStage
	}
	statusText := ""
	if source.StatusText != nil {
		statusText = *source.StatusText
	}
	var status *State
	if matches(`\bclosed\b|已截[稿止]|征稿结束|已结束`, statusText) {
		value := Closed
		status = &value
	} else if matches(`invit(?:ation|e|ed)[\s-]*only|invitation (?:is )?required|invitation basis|invited (?:submissions|papers) only|only (?:invited|by invitation)|仅限受邀|仅接受邀请`, statusText+"\n"+source.Title) {
		value := InvitationOnly
		status = &value
	}
	if !stage.IsSubmission() || !IsSourceUrl(source.SourceUrl) || len(source.CatalogIds) == 0 || strings.TrimSpace(source.Title) == "" || matches(`^(just a moment|access denied|404|暂未适配)`, strings.TrimSpace(source.Title)) {
		return nil
	}
	for _, id := range source.CatalogIds {
		if strings.TrimSpace(id) == "" {
			return nil
		}
	}
	if source.TimeZone != nil {
		if _, err := cron.Location(*source.TimeZone); err != nil {
			return nil
		}
	}
	if !expression(`^\d{4}-\d{2}-\d{2}$`).MatchString(source.CheckedOn) {
		return nil
	}
	if _, err := time.Parse("2006-01-02", source.CheckedOn); err != nil {
		return nil
	}
	dates := ParseDates(source.DateText, stage)
	if dates == nil {
		if !source.IsHistorical && (status == nil || *status != Closed) {
			return nil
		}
		dates = []Date{}
	}
	var topicYear *int32
	if kind == General && matches(`年度|年.*选题`, source.Title) {
		if found := expression(`20\d{2}`).FindString(source.Title); found != "" {
			if year, err := strconv.ParseInt(found, 10, 32); err == nil {
				value := int32(year)
				topicYear = &value
			}
		}
	}
	title := CleanText(source.Title)
	return &Notice{Id: source.SourceUrl + "#" + title, Title: title, Scope: CleanText(source.Scope), Requirements: CleanText(source.Requirements), Kind: kind, SourceUrl: source.SourceUrl, CheckedOn: source.CheckedOn, Dates: dates, EntryStage: stage, TopicYear: topicYear, TimeZone: source.TimeZone, SourceStatus: status, IsHistorical: source.IsHistorical, RawDateText: CleanText(source.RawDateText)}
}

// EntryDeadline gives mandatory abstract/proposal gates precedence over full papers.
func (notice Notice) EntryDeadline() *Date {
	var earliest *Date
	for _, date := range notice.Dates {
		if !date.IsOptional && (date.Stage == Abstract || date.Stage == Proposal) && (earliest == nil || date.Date < earliest.Date || date.Date == earliest.Date && date.IsExclusive && !earliest.IsExclusive) {
			copy := date
			earliest = &copy
		}
	}
	if earliest != nil {
		return earliest
	}
	for _, date := range notice.Dates {
		if date.Stage == notice.EntryStage && !date.IsOptional {
			copy := date
			return &copy
		}
	}
	return nil
}

// State evaluates source constraints and timezone uncertainty at one fixed instant.
func (notice Notice) State(now time.Time) State {
	if notice.SourceStatus != nil && *notice.SourceStatus == Closed {
		return Closed
	}
	uncertain := func() State {
		if notice.IsHistorical {
			return Historical
		}
		if notice.SourceStatus != nil {
			return *notice.SourceStatus
		}
		return Uncertain
	}
	if notice.RawDateText != "" {
		return uncertain()
	}
	today := now.UTC()
	if notice.TimeZone != nil {
		if zone, err := cron.Location(*notice.TimeZone); err == nil {
			today = now.In(zone)
		}
	}
	deadline := notice.EntryDeadline()
	var start *Date
	for _, date := range notice.Dates {
		if date.Stage == Opens {
			copy := date
			start = &copy
			break
		}
	}
	isClosed := func(at time.Time) bool {
		if deadline == nil {
			return false
		}
		date := at.Format("2006-01-02")
		return date > deadline.Date || deadline.IsExclusive && date == deadline.Date
	}
	if notice.TimeZone == nil {
		earliest, latest := now.UTC().Add(14*time.Hour), now.UTC().Add(-12*time.Hour)
		if isClosed(earliest) != isClosed(latest) {
			return uncertain()
		}
		if start != nil && (earliest.Format("2006-01-02") < start.Date) != (latest.Format("2006-01-02") < start.Date) {
			return Uncertain
		}
	}
	switch {
	case isClosed(today) || notice.TopicYear != nil && today.Year() > int(*notice.TopicYear):
		return Closed
	case notice.IsHistorical:
		return Historical
	case notice.SourceStatus != nil && *notice.SourceStatus == InvitationOnly:
		return InvitationOnly
	case start != nil && today.Format("2006-01-02") < start.Date:
		return Upcoming
	case deadline != nil:
		return Open
	default:
		return Undated
	}
}
