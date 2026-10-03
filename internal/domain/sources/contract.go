package sources

import (
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
)

// IndexContractVersion identifies the canonical content and progress contract.
const IndexContractVersion = 3

// JournalRankings carries optional curated classifications.
type JournalRankings struct {
	UtdRank     *string `json:"utd_rank"`
	UtdRating   *string `json:"utd_rating"`
	AbsRank     *string `json:"abs_rank"`
	AbsRating   *string `json:"abs_rating"`
	FmsRank     *string `json:"fms_rank"`
	FmsRating   *string `json:"fms_rating"`
	FmscnRank   *string `json:"fmscn_rank"`
	FmscnRating *string `json:"fmscn_rating"`
}

// JournalCatalogEntry is the maintained provider-independent identity of one journal.
type JournalCatalogEntry struct {
	CatalogId      string          `json:"catalog_id"`
	CatalogAliases []string        `json:"catalog_aliases"`
	Title          string          `json:"title"`
	Issn           *string         `json:"issn"`
	Eissn          *string         `json:"eissn"`
	AllIssns       []string        `json:"all_issns"`
	TitleAliases   []string        `json:"title_aliases"`
	Area           *string         `json:"area"`
	Rankings       JournalRankings `json:"rankings"`
}

// JournalDraft records only observed bibliographic identity, never provider URLs.
type JournalDraft struct {
	CatalogId            string   `json:"catalog_id"`
	ObservedTitle        *string  `json:"observed_title"`
	ObservedIssns        []string `json:"observed_issns"`
	ObservedTitleAliases []string `json:"observed_title_aliases"`
}

// IssueDraft describes a canonical issue without a durable database identity.
type IssueDraft struct {
	CatalogId       string  `json:"catalog_id"`
	PublicationYear *int64  `json:"publication_year"`
	Title           *string `json:"title"`
	Volume          *string `json:"volume"`
	Number          *string `json:"number"`
	Date            *string `json:"date"`
}

// ArticleAuthorDraft retains one author's position and canonical display name.
type ArticleAuthorDraft struct {
	DisplayName string `json:"display_name"`
}

// ArticleDraft carries provider-neutral content without durable identifiers or links.
type ArticleDraft struct {
	CatalogId       string               `json:"catalog_id"`
	Title           string               `json:"title"`
	PublicationYear *int64               `json:"publication_year"`
	Date            *string              `json:"date"`
	IssueTitle      *string              `json:"issue_title"`
	Volume          *string              `json:"volume"`
	IssueNumber     *string              `json:"issue_number"`
	Authors         []ArticleAuthorDraft `json:"authors"`
	StartPage       *string              `json:"start_page"`
	EndPage         *string              `json:"end_page"`
	AbstractText    *string              `json:"abstract_text"`
	Doi             *string              `json:"doi"`
	Pmid            *string              `json:"pmid"`
	OpenAccess      *bool                `json:"open_access"`
	InPress         *bool                `json:"in_press"`
	RetractionDois  []string             `json:"retraction_dois"`
}

// IndexSyncMode selects initial, incremental or complete historical discovery.
type IndexSyncMode string

const (
	Bootstrap   IndexSyncMode = "bootstrap"
	Incremental IndexSyncMode = "incremental"
	FullRescan  IndexSyncMode = "full_rescan"
)

// IndexFetchContext distinguishes the committed anchor from unacknowledged traversal progress.
type IndexFetchContext struct {
	Mode                IndexSyncMode
	CommittedAnchor     *string
	TraversalCheckpoint *string
}

// ProgressState indicates whether the provider's frozen window is complete.
type ProgressState string

const (
	Continue ProgressState = "continue"
	Complete ProgressState = "complete"
)

// ProviderProgress contains either a continuation checkpoint or an optional completed anchor.
type ProviderProgress struct {
	State      ProgressState `json:"state"`
	Checkpoint *string       `json:"checkpoint,omitempty"`
	NextAnchor *string       `json:"next_anchor,omitempty"`
}

// ProviderBatch returns one canonical content page and its provider-owned progress.
type ProviderBatch struct {
	CatalogId string           `json:"catalog_id"`
	Journal   JournalDraft     `json:"journal"`
	Issues    []IssueDraft     `json:"issues"`
	Articles  []ArticleDraft   `json:"articles"`
	Progress  ProviderProgress `json:"progress"`
}

// ArticleLocator contains the local metadata needed to resolve an ephemeral destination.
type ArticleLocator struct {
	ArticleId       identity.Id
	CatalogId       string
	JournalTitle    string
	JournalIssns    []string
	Title           string
	PublicationYear *int64
	Date            *string
	Authors         []string
	Volume          *string
	IssueNumber     *string
	StartPage       *string
	EndPage         *string
	Doi             *string
	Pmid            *string
}

// CapabilityKind selects an independently registered provider operation.
type CapabilityKind uint8

const (
	IndexContent CapabilityKind = iota
	ArticleAbstract
	ArticleFullText
)

// ArticleAccessContext carries the requesting user and shared monotonic deadline.
type ArticleAccessContext struct {
	UserId   *identity.Id
	Deadline time.Time
}

// ArticleRedirect is a destination used only for the current response.
type ArticleRedirect struct{ Location string }

// ArticleFullTextDocument holds bounded content and a safe optional download basename.
type ArticleFullTextDocument struct {
	ContentType string
	Filename    *string
	Bytes       []byte
}

// ArticleFullTextResolution contains exactly one ephemeral redirect or document.
type ArticleFullTextResolution struct {
	Redirect *ArticleRedirect
	Document *ArticleFullTextDocument
}
