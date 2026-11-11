package api

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"

	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	acquisition "github.com/QianFuv/LitRadar/apps/server/internal/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	metadata "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"github.com/QianFuv/LitRadar/apps/server/internal/index"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
)

const cfpCursorContext = "litradar.cfp.cursor.v1"
const cfpCursorDetail = "CFP page changed or cursor is invalid; reload from the first page"

var cfpRefreshCapability = sync.OnceValue(func() func(string) bool {
	return cfpRefreshLookup(acquisition.Registry())
})

type cfpHandlers struct {
	storage       config.Config
	repository    *storage.Repository
	codec         *secrets.Codec
	authenticator *Authenticator
	pool          *executor.Pool
}

type cfpStateCounts map[domain.State]int

func (counts cfpStateCounts) MarshalJSON() ([]byte, error) {
	var result strings.Builder
	result.WriteByte('{')
	for _, state := range []domain.State{domain.Open, domain.Upcoming, domain.Closed, domain.Historical, domain.InvitationOnly, domain.Undated, domain.Uncertain} {
		if count, exists := counts[state]; exists {
			if result.Len() > 1 {
				result.WriteByte(',')
			}
			encoded, err := jsonvalue.EncodeJson(count)
			if err != nil {
				return nil, err
			}
			result.WriteString(`"` + string(state) + `":` + encoded)
		}
	}
	result.WriteByte('}')
	return []byte(result.String()), nil
}

type cfpJournalSummary struct {
	CatalogId       string         `json:"catalogId" nullable:"false"`
	CatalogAliases  []string       `json:"catalogAliases" nullable:"false"`
	AllIssns        []string       `json:"allIssns" nullable:"false"`
	TitleAliases    []string       `json:"titleAliases" nullable:"false"`
	Title           string         `json:"title" nullable:"false"`
	Area            *string        `json:"area" required:"false" nullable:"true"`
	Coverage        string         `json:"coverage" nullable:"false" enum:"adapted,unadapted"`
	CheckedOn       *string        `json:"checkedOn" required:"false" nullable:"true"`
	SourceUrl       *string        `json:"sourceUrl" required:"false" nullable:"true"`
	SourceStatement *string        `json:"sourceStatement" required:"false" nullable:"true"`
	NoticeCount     int            `json:"noticeCount" nullable:"false" minimum:"0" format:"int64"`
	CurrentCount    int            `json:"currentCount" nullable:"false" minimum:"0" format:"int64"`
	StateCounts     cfpStateCounts `json:"stateCounts" nullable:"false"`
	CanRefresh      bool           `json:"canRefresh" nullable:"false"`
	RefreshStatus   string         `json:"refreshStatus" nullable:"false" enum:"snapshot,success,failed,refreshing,unsupported,unadapted"`
	LastAttempt     *int64         `json:"lastAttempt" required:"false" nullable:"true" format:"int64"`
	LastSuccess     *int64         `json:"lastSuccess" required:"false" nullable:"true" format:"int64"`
	LastError       *string        `json:"lastError" required:"false" nullable:"true"`
}

type cfpCatalogSummary struct {
	Journals        int `json:"journals" nullable:"false" minimum:"0" format:"int64"`
	AdaptedJournals int `json:"adaptedJournals" nullable:"false" minimum:"0" format:"int64"`
	Notices         int `json:"notices" nullable:"false" minimum:"0" format:"int64"`
	CurrentNotices  int `json:"currentNotices" nullable:"false" minimum:"0" format:"int64"`
}
type cfpCatalogResponse struct {
	Database    string              `json:"database" nullable:"false"`
	EvaluatedAt int64               `json:"evaluatedAt" nullable:"false" format:"int64"`
	Summary     cfpCatalogSummary   `json:"summary" nullable:"false"`
	Items       []cfpJournalSummary `json:"items" nullable:"false"`
}

// NoticeWire retains notice fields without promoting its standalone JSON encoder.
type NoticeWire domain.Notice
type cfpNoticeView struct {
	NoticeWire
	State         domain.State `json:"state" nullable:"false" enum:"open,upcoming,closed,historical,invitation_only,undated,uncertain"`
	EntryDeadline *domain.Date `json:"entryDeadline" required:"false" nullable:"false" allowNull:"true"`
}
type cfpNoticePage struct {
	Journal     cfpJournalSummary `json:"journal" nullable:"false"`
	EvaluatedAt int64             `json:"evaluatedAt" nullable:"false" format:"int64"`
	Items       []cfpNoticeView   `json:"items" nullable:"false"`
	Page        metadata.PageMeta `json:"page" nullable:"false"`
}
type cfpCursor struct {
	Database      string `json:"database"`
	CatalogId     string `json:"catalog_id"`
	IncludeClosed bool   `json:"include_closed"`
	Revision      string `json:"revision"`
	EvaluatedAt   int64  `json:"evaluated_at"`
	Position      uint64 `json:"position"`
	Order         string `json:"order"`
}

func (handlers *cfpHandlers) catalogMembers(database string) ([]sources.JournalCatalogEntry, *apiError) {
	catalogs, err := handlers.storage.ListProviderCatalogs()
	if err != nil {
		return nil, internalError()
	}
	for _, catalog := range catalogs {
		if catalog.Stem+".sqlite" == database && catalog.CsvFilename != nil {
			entries, err := index.ReadCatalogCsv(filepath.Join(handlers.storage.MetaDir, *catalog.CsvFilename))
			if err != nil {
				return nil, internalError()
			}
			return entries, nil
		}
	}
	return nil, &apiError{status: 404, detail: "CFP database catalog not found"}
}

func cfpFindSnapshot(entry sources.JournalCatalogEntry, snapshots []storage.JournalSnapshot) (*storage.JournalSnapshot, *apiError) {
	var result *storage.JournalSnapshot
	for index := range snapshots {
		for _, alias := range snapshots[index].CatalogIds {
			if alias == entry.CatalogId || slices.Contains(entry.CatalogAliases, alias) {
				if result != nil {
					return nil, internalError()
				}
				result = &snapshots[index]
				break
			}
		}
	}
	return result, nil
}

// cfpIndexSnapshots marks shared aliases as ambiguous without rejecting unrelated catalog entries.
func cfpIndexSnapshots(snapshots []storage.JournalSnapshot) map[string]*storage.JournalSnapshot {
	indexed := make(map[string]*storage.JournalSnapshot, len(snapshots))
	for index := range snapshots {
		snapshot := &snapshots[index]
		for _, alias := range snapshot.CatalogIds {
			if previous, exists := indexed[alias]; exists && previous != snapshot {
				indexed[alias] = nil
			} else {
				indexed[alias] = snapshot
			}
		}
	}
	return indexed
}

// cfpFindIndexedSnapshot retains exact alias matching and rejects distinct matching snapshots.
func cfpFindIndexedSnapshot(entry sources.JournalCatalogEntry, indexed map[string]*storage.JournalSnapshot) (*storage.JournalSnapshot, *apiError) {
	var result *storage.JournalSnapshot
	for _, aliases := range [][]string{{entry.CatalogId}, entry.CatalogAliases} {
		for _, alias := range aliases {
			snapshot, exists := indexed[alias]
			if !exists {
				continue
			}
			if snapshot == nil || result != nil && result != snapshot {
				return nil, internalError()
			}
			result = snapshot
		}
	}
	return result, nil
}

// cfpSummary projects catalog identity before optional snapshot and refresh metadata.
func cfpSummary(entry sources.JournalCatalogEntry, snapshot *storage.JournalSnapshot, evaluatedAt, now int64) cfpJournalSummary {
	result := cfpJournalSummary{CatalogId: entry.CatalogId, CatalogAliases: entry.CatalogAliases, AllIssns: entry.AllIssns, TitleAliases: entry.TitleAliases, Title: entry.Title, Area: entry.Area, Coverage: "unadapted", RefreshStatus: "unadapted", StateCounts: cfpStateCounts{}}
	if snapshot == nil {
		return result
	}
	result.Coverage, result.CheckedOn, result.SourceUrl, result.SourceStatement = "adapted", &snapshot.CheckedOn, snapshot.SourceUrl, snapshot.SourceStatement
	result.NoticeCount = len(snapshot.Notices)
	countCfpNotices(&result, snapshot, evaluatedAt)
	applyCfpRefresh(&result, snapshot, now)
	return result
}

// countCfpNotices evaluates all notices at the page's frozen timestamp.
func countCfpNotices(result *cfpJournalSummary, snapshot *storage.JournalSnapshot, evaluatedAt int64) {
	for _, notice := range snapshot.Notices {
		state := notice.State(time.Unix(evaluatedAt, 0))
		result.StateCounts[state]++
		if !state.IsArchived() {
			result.CurrentCount++
		}
	}
}

// latestCfpSource retains source order when no attempt timestamp is available.
func latestCfpSource(snapshot *storage.JournalSnapshot) *storage.SourceStatus {
	var latest *storage.SourceStatus
	for index := range snapshot.Sources {
		source := &snapshot.Sources[index]
		if latest == nil || latest.LastAttempt == nil || source.LastAttempt != nil && *source.LastAttempt >= *latest.LastAttempt {
			latest = source
		}
	}
	return latest
}

// cfpRefreshLookup retains the first registration for each exact alias without exposing shared storage.
func cfpRefreshLookup(registrations []acquisition.SourceConfig) func(string) bool {
	capabilities := make(map[string]bool, len(registrations))
	for _, registration := range registrations {
		for _, alias := range registration.CatalogIds {
			if _, exists := capabilities[alias]; !exists {
				capabilities[alias] = registration.CanRefresh()
			}
		}
	}
	return func(journalKey string) bool { return capabilities[journalKey] }
}

// applyCfpRefresh reuses packaged capabilities while projecting live source status and lease expiry.
func applyCfpRefresh(result *cfpJournalSummary, snapshot *storage.JournalSnapshot, now int64) {
	result.CanRefresh = cfpRefreshCapability()(snapshot.JournalKey)
	latest := latestCfpSource(snapshot)
	if latest != nil {
		result.LastAttempt, result.LastSuccess, result.LastError = latest.LastAttempt, latest.LastSuccess, latest.LastError
		switch latest.Status {
		case "success", "failed", "unsupported", "refreshing":
			result.RefreshStatus = latest.Status
		default:
			result.RefreshStatus = "snapshot"
		}
		if latest.Status == "refreshing" && (latest.LeaseExpiresAt == nil || *latest.LeaseExpiresAt < now) {
			message := "Previous source refresh ended before publication"
			result.RefreshStatus, result.LastError = "failed", &message
		}
	}
}

func cfpCatalog(database string, query *string, entries []sources.JournalCatalogEntry, snapshots []storage.JournalSnapshot, now int64) (*cfpCatalogResponse, *apiError) {
	result := &cfpCatalogResponse{Database: database, EvaluatedAt: now, Items: []cfpJournalSummary{}}
	search := ""
	if query != nil {
		search = sources.Lowercase(strings.TrimSpace(*query))
	}
	indexed := cfpIndexSnapshots(snapshots)
	for _, entry := range entries {
		snapshot, failure := cfpFindIndexedSnapshot(entry, indexed)
		if failure != nil {
			return nil, failure
		}
		item := cfpSummary(entry, snapshot, now, now)
		result.Summary.Journals++
		if item.Coverage == "adapted" {
			result.Summary.AdaptedJournals++
		}
		result.Summary.Notices += item.NoticeCount
		result.Summary.CurrentNotices += item.CurrentCount
		if search == "" || cfpMatchesSearch(item, search) {
			result.Items = append(result.Items, item)
		}
	}
	titleKeys := map[string]string{}
	titleKey := func(title string) string {
		if normalized, exists := titleKeys[title]; exists {
			return normalized
		}
		normalized := sources.Lowercase(title)
		titleKeys[title] = normalized
		return normalized
	}
	slices.SortFunc(result.Items, func(first, second cfpJournalSummary) int {
		if first.Coverage != second.Coverage {
			if first.Coverage == "adapted" {
				return -1
			}
			return 1
		}
		if order := strings.Compare(titleKey(first.Title), titleKey(second.Title)); order != 0 {
			return order
		}
		return strings.Compare(first.CatalogId, second.CatalogId)
	})
	return result, nil
}

// cfpMatchesSearch checks the existing searchable fields without allocating a combined field list.
func cfpMatchesSearch(item cfpJournalSummary, search string) bool {
	matches := func(field string) bool { return strings.Contains(sources.Lowercase(field), search) }
	return matches(item.Title) || matches(item.CatalogId) || slices.ContainsFunc(item.AllIssns, matches) ||
		slices.ContainsFunc(item.TitleAliases, matches) || item.Area != nil && matches(*item.Area)
}

func cfpRevision(entry sources.JournalCatalogEntry, snapshot *storage.JournalSnapshot) (string, error) {
	var revisions any
	if snapshot != nil {
		values := make([][2]any, 0, len(snapshot.Sources))
		for _, source := range snapshot.Sources {
			values = append(values, [2]any{source.SourceKey, source.Revision})
		}
		revisions = values
	}
	type revisionEntry sources.JournalCatalogEntry
	if entry.CatalogAliases == nil {
		entry.CatalogAliases = []string{}
	}
	if entry.AllIssns == nil {
		entry.AllIssns = []string{}
	}
	if entry.TitleAliases == nil {
		entry.TitleAliases = []string{}
	}
	encoded, err := jsonvalue.EncodeJson([2]any{revisionEntry(entry), revisions})
	if err != nil {
		return "", err
	}
	digest := sha256.Sum256([]byte(encoded))
	return hex.EncodeToString(digest[:]), nil
}

func cfpCursorError() *apiError { return &apiError{status: 409, detail: cfpCursorDetail} }

func decodeCfpCursor(codec *secrets.Codec, ciphertext string) (*cfpCursor, *apiError) {
	plaintext, err := codec.Decrypt(ciphertext, cfpCursorContext)
	if err != nil {
		return nil, cfpCursorError()
	}
	fields, err := storedFields(plaintext, []string{"database", "catalog_id", "include_closed", "revision", "evaluated_at", "position", "order"})
	if err != nil || len(fields) != 7 {
		return nil, cfpCursorError()
	}
	result := cfpCursor{}
	targets := map[string]any{"database": &result.Database, "catalog_id": &result.CatalogId, "include_closed": &result.IncludeClosed, "revision": &result.Revision, "evaluated_at": &result.EvaluatedAt, "position": &result.Position, "order": &result.Order}
	for name, target := range targets {
		raw, exists := fields[name]
		if !exists || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			return nil, cfpCursorError()
		}
		if err := json.Unmarshal(raw, target); err != nil {
			return nil, cfpCursorError()
		}
	}
	return &result, nil
}

func cfpPage(codec *secrets.Codec, database string, entry sources.JournalCatalogEntry, snapshot *storage.JournalSnapshot, includeClosed bool, ciphertext *string, limit uint64, now int64) (*cfpNoticePage, *apiError) {
	revision, err := cfpRevision(entry, snapshot)
	if err != nil {
		return nil, internalError()
	}
	cursor := cfpCursor{database, entry.CatalogId, includeClosed, revision, now, 0, "source-v1"}
	cursor, failure := resumeCfpCursor(codec, cursor, ciphertext, now)
	if failure != nil {
		return nil, failure
	}
	items := cfpNoticeItems(snapshot, includeClosed, cursor.EvaluatedAt)
	if cursor.Position > uint64(len(items)) {
		return nil, cfpCursorError()
	}
	total, offset := int64(len(items)), int64(cursor.Position)
	next := min(cursor.Position+limit, uint64(len(items)))
	hasMore := next < uint64(len(items))
	var nextCursor *string
	if hasMore {
		cursor.Position = next
		encoded, err := jsonvalue.EncodeJson(cursor)
		if err != nil {
			return nil, internalError()
		}
		value, err := codec.Encrypt(encoded, cfpCursorContext)
		if err != nil {
			return nil, internalError()
		}
		nextCursor = &value
	}
	return &cfpNoticePage{cfpSummary(entry, snapshot, cursor.EvaluatedAt, now), cursor.EvaluatedAt, items[offset:next], metadata.PageMeta{Total: &total, Limit: int64(limit), Offset: offset, NextCursor: nextCursor, HasMore: &hasMore}}, nil
}

// resumeCfpCursor validates encrypted continuation against the current page identity.
func resumeCfpCursor(codec *secrets.Codec, expected cfpCursor, ciphertext *string, now int64) (cfpCursor, *apiError) {
	if ciphertext == nil {
		return expected, nil
	}
	decoded, failure := decodeCfpCursor(codec, *ciphertext)
	if failure != nil {
		return expected, failure
	}
	if !matchesCfpCursor(*decoded, expected, now) {
		return expected, cfpCursorError()
	}
	return *decoded, nil
}

// matchesCfpCursor excludes limit while enforcing the inclusive fifteen-minute lifetime.
func matchesCfpCursor(decoded, expected cfpCursor, now int64) bool {
	return decoded.Database == expected.Database && decoded.CatalogId == expected.CatalogId && decoded.IncludeClosed == expected.IncludeClosed && decoded.Revision == expected.Revision && decoded.Order == "source-v1" && decoded.EvaluatedAt <= now && decoded.EvaluatedAt >= now-900
}

// cfpNoticeItems preserves source order and nonnil empty page items.
func cfpNoticeItems(snapshot *storage.JournalSnapshot, includeClosed bool, evaluatedAt int64) []cfpNoticeView {
	items := []cfpNoticeView{}
	if snapshot == nil {
		return items
	}
	for _, notice := range snapshot.Notices {
		state := notice.State(time.Unix(evaluatedAt, 0))
		if includeClosed || !state.IsArchived() {
			items = append(items, cfpNoticeView{NoticeWire(notice), state, notice.EntryDeadline()})
		}
	}
	return items
}
