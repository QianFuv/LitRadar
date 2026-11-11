// Package cfp preserves original-language calls, calendar gates and source freshness.
package cfp

const ParserVersion uint32 = 1

type Kind string

const (
	SpecialIssue     Kind = "special_issue"
	General          Kind = "general"
	ProposalKind     Kind = "proposal"
	ConferenceLinked Kind = "conference_linked"
)

type Stage string

const (
	Paper        Stage = "paper"
	Abstract     Stage = "abstract"
	Proposal     Stage = "proposal"
	Opens        Stage = "opens"
	Revision     Stage = "revision"
	Decision     Stage = "decision"
	Publication  Stage = "publication"
	Event        Stage = "event"
	Registration Stage = "registration"
)

// IsSubmission distinguishes initial admission from later editorial milestones.
func (stage Stage) IsSubmission() bool {
	return stage == Paper || stage == Abstract || stage == Proposal
}

type State string

const (
	Open           State = "open"
	Upcoming       State = "upcoming"
	Closed         State = "closed"
	Historical     State = "historical"
	InvitationOnly State = "invitation_only"
	Undated        State = "undated"
	Uncertain      State = "uncertain"
)

func (state State) IsArchived() bool { return state == Closed || state == Historical }

// Source contains literal reviewed fields before normalization, in persisted field order.
type Source struct {
	CatalogIds   []string `json:"catalogIds"`
	JournalTitle string   `json:"journalTitle"`
	Title        string   `json:"title"`
	Scope        string   `json:"scope" default:"true"`
	Requirements string   `json:"requirements" default:"true"`
	TypeText     string   `json:"typeText"`
	DateText     string   `json:"dateText"`
	SourceURL    string   `json:"sourceUrl"`
	CheckedOn    string   `json:"checkedOn"`
	EntryStage   *Stage   `json:"entryStage"`
	TimeZone     *string  `json:"timeZone"`
	StatusText   *string  `json:"statusText"`
	IsHistorical bool     `json:"isHistorical" default:"true"`
	RawDateText  string   `json:"rawDateText" default:"true"`
}

type Date struct {
	Date         string `json:"date" nullable:"false"`
	Stage        Stage  `json:"stage" nullable:"false" enum:"paper,abstract,proposal,opens,revision,decision,publication,event,registration"`
	OriginalText string `json:"originalText" nullable:"false"`
	IsExclusive  bool   `json:"isExclusive" nullable:"false"`
	IsOptional   bool   `json:"isOptional" nullable:"false"`
}

// Notice stores normalized source content; State evaluates availability at read time.
type Notice struct {
	ID           string  `json:"id" nullable:"false"`
	Title        string  `json:"title" nullable:"false"`
	Scope        string  `json:"scope" nullable:"false"`
	Requirements string  `json:"requirements" nullable:"false"`
	Kind         Kind    `json:"kind" nullable:"false" enum:"special_issue,general,proposal,conference_linked"`
	SourceURL    string  `json:"sourceUrl" nullable:"false"`
	CheckedOn    string  `json:"checkedOn" nullable:"false"`
	Dates        []Date  `json:"dates" nullable:"false"`
	EntryStage   Stage   `json:"entryStage" nullable:"false" enum:"paper,abstract,proposal,opens,revision,decision,publication,event,registration"`
	TopicYear    *int32  `json:"topicYear" required:"false" nullable:"true" format:"int32"`
	TimeZone     *string `json:"timeZone" required:"false" nullable:"true"`
	SourceStatus *State  `json:"sourceStatus" required:"false" nullable:"true" enum:"open,upcoming,closed,historical,invitation_only,undated,uncertain"`
	IsHistorical bool    `json:"isHistorical" nullable:"false"`
	RawDateText  string  `json:"rawDateText" nullable:"false"`
}

type EmptyJournal struct {
	CatalogIds      []string `json:"catalogIds"`
	JournalTitle    string   `json:"journalTitle"`
	CheckedOn       string   `json:"checkedOn"`
	SourceURL       string   `json:"sourceUrl"`
	SourceStatement string   `json:"sourceStatement"`
	Notices         []Notice `json:"notices" default:"true"`
}

type Seed struct {
	FormatVersion    uint32         `json:"formatVersion"`
	Sources          []Source       `json:"sources"`
	EmptyJournals    []EmptyJournal `json:"emptyJournals"`
	ExpectedJournals *uint64        `json:"expectedJournals"`
	ExpectedNotices  *uint64        `json:"expectedNotices"`
}
