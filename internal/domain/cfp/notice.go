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
	kind := sourceKind(source.TypeText)
	if kind == "" {
		return nil
	}
	stage := sourceEntryStage(source, kind)
	status := sourceStatus(source)
	if !isAdmissibleSource(source, stage) {
		return nil
	}
	dates := ParseDates(source.DateText, stage)
	if dates == nil {
		if !source.IsHistorical && (status == nil || *status != Closed) {
			return nil
		}
		dates = []Date{}
	}
	title := CleanText(source.Title)
	return &Notice{Id: source.SourceUrl + "#" + title, Title: title, Scope: CleanText(source.Scope), Requirements: CleanText(source.Requirements), Kind: kind, SourceUrl: source.SourceUrl, CheckedOn: source.CheckedOn, Dates: dates, EntryStage: stage, TopicYear: sourceTopicYear(source, kind), TimeZone: source.TimeZone, SourceStatus: status, IsHistorical: source.IsHistorical, RawDateText: CleanText(source.RawDateText)}
}

// sourceKind retains the precedence of mixed source classifications.
func sourceKind(typeText string) Kind {
	switch {
	case matches(`proposal|提案|专刊建议`, typeText):
		return ProposalKind
	case matches(`conference|workshop|会议|研讨会|工作坊`, typeText):
		return ConferenceLinked
	case matches(`special|专[题刊栏辑]|特[刊辑]`, typeText):
		return SpecialIssue
	case matches(`general|regular|常规|年度|重点选题`, typeText):
		return General
	default:
		return ""
	}
}

// sourceEntryStage applies the explicit stage after the kind-derived default.
func sourceEntryStage(source Source, kind Kind) Stage {
	stage := Paper
	if kind == ProposalKind {
		stage = Proposal
	}
	if source.EntryStage != nil {
		stage = *source.EntryStage
	}
	return stage
}

// sourceStatus gives explicit closure priority over invitation constraints.
func sourceStatus(source Source) *State {
	statusText := ""
	if source.StatusText != nil {
		statusText = *source.StatusText
	}
	if matches(`\bclosed\b|已截[稿止]|征稿结束|已结束`, statusText) {
		value := Closed
		return &value
	} else if matches(`invit(?:ation|e|ed)[\s-]*only|invitation (?:is )?required|invitation basis|invited (?:submissions|papers) only|only (?:invited|by invitation)|仅限受邀|仅接受邀请`, statusText+"\n"+source.Title) {
		value := InvitationOnly
		return &value
	}
	return nil
}

// isAdmissibleSource validates literal source fields without normalizing identity.
func isAdmissibleSource(source Source, stage Stage) bool {
	if !stage.IsSubmission() || !IsSourceUrl(source.SourceUrl) || len(source.CatalogIds) == 0 || strings.TrimSpace(source.Title) == "" || matches(`^(just a moment|access denied|404|暂未适配)`, strings.TrimSpace(source.Title)) {
		return false
	}
	for _, id := range source.CatalogIds {
		if strings.TrimSpace(id) == "" {
			return false
		}
	}
	return isValidSourceCalendar(source)
}

// isValidSourceCalendar checks the declared timezone and padded review date.
func isValidSourceCalendar(source Source) bool {
	if source.TimeZone != nil {
		if _, err := cron.Location(*source.TimeZone); err != nil {
			return false
		}
	}
	if !expression(`^\d{4}-\d{2}-\d{2}$`).MatchString(source.CheckedOn) {
		return false
	}
	if _, err := time.Parse("2006-01-02", source.CheckedOn); err != nil {
		return false
	}
	return true
}

// sourceTopicYear extracts annual general-call years without changing other kinds.
func sourceTopicYear(source Source, kind Kind) *int32 {
	var topicYear *int32
	if kind == General && matches(`年度|年.*选题`, source.Title) {
		if found := expression(`20\d{2}`).FindString(source.Title); found != "" {
			if year, err := strconv.ParseInt(found, 10, 32); err == nil {
				value := int32(year)
				topicYear = &value
			}
		}
	}
	return topicYear
}

// EntryDeadline gives mandatory abstract/proposal gates precedence over full papers.
func (notice Notice) EntryDeadline() *Date {
	var earliest *Date
	for _, date := range notice.Dates {
		if isMandatoryGate(date) && precedesGate(date, earliest) {
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

// isMandatoryGate distinguishes initial abstract/proposal admission from optional dates.
func isMandatoryGate(date Date) bool {
	return !date.IsOptional && (date.Stage == Abstract || date.Stage == Proposal)
}

// precedesGate resolves equal-day gates in favor of an exclusive deadline.
func precedesGate(date Date, earliest *Date) bool {
	return earliest == nil || date.Date < earliest.Date || date.Date == earliest.Date && date.IsExclusive && !earliest.IsExclusive
}

// State evaluates source constraints and timezone uncertainty at one fixed instant.
func (notice Notice) State(now time.Time) State {
	if notice.SourceStatus != nil && *notice.SourceStatus == Closed {
		return Closed
	}
	if notice.RawDateText != "" {
		return notice.uncertainState()
	}
	today := now.UTC()
	if notice.TimeZone != nil {
		if zone, err := cron.Location(*notice.TimeZone); err == nil {
			today = now.In(zone)
		}
	}
	deadline := notice.EntryDeadline()
	start := firstStageDate(notice.Dates, Opens)
	if notice.TimeZone == nil {
		if state, isUncertain := notice.unknownZoneState(now, deadline, start); isUncertain {
			return state
		}
	}
	return notice.calendarState(today, deadline, start)
}

// uncertainState preserves archival/source precedence for unresolved deadlines.
func (notice Notice) uncertainState() State {
	if notice.IsHistorical {
		return Historical
	}
	if notice.SourceStatus != nil {
		return *notice.SourceStatus
	}
	return Uncertain
}

// firstStageDate returns a copy of the first matching persisted date.
func firstStageDate(dates []Date, stage Stage) *Date {
	for _, date := range dates {
		if date.Stage == stage {
			copy := date
			return &copy
		}
	}
	return nil
}

// isDeadlineClosed applies inclusive and exclusive day boundaries at an instant.
func isDeadlineClosed(deadline *Date, at time.Time) bool {
	if deadline == nil {
		return false
	}
	date := at.Format("2006-01-02")
	return date > deadline.Date || deadline.IsExclusive && date == deadline.Date
}

// unknownZoneState separates deadline fallback from opening uncertainty.
func (notice Notice) unknownZoneState(now time.Time, deadline, start *Date) (State, bool) {
	earliest, latest := now.UTC().Add(14*time.Hour), now.UTC().Add(-12*time.Hour)
	if isDeadlineClosed(deadline, earliest) != isDeadlineClosed(deadline, latest) {
		return notice.uncertainState(), true
	}
	if start != nil && (earliest.Format("2006-01-02") < start.Date) != (latest.Format("2006-01-02") < start.Date) {
		return Uncertain, true
	}
	return "", false
}

// calendarState retains closure, archive, invitation and opening precedence.
func (notice Notice) calendarState(today time.Time, deadline, start *Date) State {
	switch {
	case isDeadlineClosed(deadline, today) || notice.TopicYear != nil && today.Year() > int(*notice.TopicYear):
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
