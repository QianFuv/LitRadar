package api

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"path/filepath"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	acquisition "github.com/QianFuv/LitRadar/internal/cfp"
	"github.com/QianFuv/LitRadar/internal/domain/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
	metadata "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/index"
	"github.com/QianFuv/LitRadar/internal/openapi"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

const cfpCursorContext = "litradar.cfp.cursor.v1"
const cfpCursorDetail = "CFP page changed or cursor is invalid; reload from the first page"

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
			encoded, err := auth.EncodeJson(count)
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
	CatalogId       string         `json:"catalogId"`
	CatalogAliases  []string       `json:"catalogAliases"`
	AllIssns        []string       `json:"allIssns"`
	TitleAliases    []string       `json:"titleAliases"`
	Title           string         `json:"title"`
	Area            *string        `json:"area"`
	Coverage        string         `json:"coverage"`
	CheckedOn       *string        `json:"checkedOn"`
	SourceUrl       *string        `json:"sourceUrl"`
	SourceStatement *string        `json:"sourceStatement"`
	NoticeCount     int            `json:"noticeCount"`
	CurrentCount    int            `json:"currentCount"`
	StateCounts     cfpStateCounts `json:"stateCounts"`
	CanRefresh      bool           `json:"canRefresh"`
	RefreshStatus   string         `json:"refreshStatus"`
	LastAttempt     *int64         `json:"lastAttempt"`
	LastSuccess     *int64         `json:"lastSuccess"`
	LastError       *string        `json:"lastError"`
}

type cfpCatalogSummary struct {
	Journals        int `json:"journals"`
	AdaptedJournals int `json:"adaptedJournals"`
	Notices         int `json:"notices"`
	CurrentNotices  int `json:"currentNotices"`
}
type cfpCatalogResponse struct {
	Database    string              `json:"database"`
	EvaluatedAt int64               `json:"evaluatedAt"`
	Summary     cfpCatalogSummary   `json:"summary"`
	Items       []cfpJournalSummary `json:"items"`
}
type cfpNoticeWire domain.Notice
type cfpNoticeView struct {
	cfpNoticeWire
	State         domain.State `json:"state"`
	EntryDeadline *domain.Date `json:"entryDeadline"`
}
type cfpNoticePage struct {
	Journal     cfpJournalSummary `json:"journal"`
	EvaluatedAt int64             `json:"evaluatedAt"`
	Items       []cfpNoticeView   `json:"items"`
	Page        metadata.PageMeta `json:"page"`
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

func (handlers *cfpHandlers) routes() []route {
	return []route{
		{openapi.Operation{Method: "GET", Path: "/api/cfp/journals", Id: "list_cfp_journals"}, handlers.catalog},
		{openapi.Operation{Method: "GET", Path: "/api/cfp/journals/{catalog_id}/notices", Id: "list_cfp_notices"}, handlers.notices},
	}
}

func (handlers *cfpHandlers) catalog(writer http.ResponseWriter, request *http.Request) {
	query, failure := extractQuery(request.URL.RawQuery, map[string]queryKind{"db": queryText, "q": queryText})
	if failure == nil && query.text("db") == nil {
		failure = queryRejection("missing field `db`")
	}
	if failure != nil {
		failure.write(writer)
		return
	}
	if _, failure = handlers.authenticator.requireUser(request); failure != nil {
		failure.write(writer)
		return
	}
	if search := query.text("q"); search != nil && utf8.RuneCountInString(*search) > 256 {
		badRequest("CFP journal search exceeds 256 characters").write(writer)
		return
	}
	handlers.respond(writer, request, func() (any, *apiError) {
		entries, failure := handlers.catalogMembers(*query.text("db"))
		if failure != nil {
			return nil, failure
		}
		snapshots, err := handlers.repository.LoadJournals(context.WithoutCancel(request.Context()))
		if err != nil {
			return nil, internalError()
		}
		return cfpCatalog(*query.text("db"), query.text("q"), entries, snapshots, time.Now().Unix())
	})
}

func (handlers *cfpHandlers) notices(writer http.ResponseWriter, request *http.Request) {
	catalogId := request.PathValue("catalog_id")
	if !utf8.ValidString(catalogId) {
		(&apiError{status: 400, detail: "Invalid URL: Invalid UTF-8 in `catalog_id`", isPlain: true}).write(writer)
		return
	}
	query, failure := extractQuery(request.URL.RawQuery, map[string]queryKind{"db": queryText, "include_closed": queryBoolean, "limit": queryUnsigned, "cursor": queryText})
	if failure == nil && query.text("db") == nil {
		failure = queryRejection("missing field `db`")
	}
	if failure != nil {
		failure.write(writer)
		return
	}
	if _, failure = handlers.authenticator.requireUser(request); failure != nil {
		failure.write(writer)
		return
	}
	limit, exists := query["limit"].(uint64)
	if !exists {
		limit = 50
	}
	if limit < 1 || limit > 200 {
		badRequest("CFP page limit must be between 1 and 200").write(writer)
		return
	}
	cursor := query.text("cursor")
	if cursor != nil && len(*cursor) > 4096 {
		cfpCursorError().write(writer)
		return
	}
	includeClosed, _ := query["include_closed"].(bool)
	handlers.respond(writer, request, func() (any, *apiError) {
		entries, failure := handlers.catalogMembers(*query.text("db"))
		if failure != nil {
			return nil, failure
		}
		var entry *sources.JournalCatalogEntry
		for index := range entries {
			if entries[index].CatalogId == catalogId || slices.Contains(entries[index].CatalogAliases, catalogId) {
				entry = &entries[index]
				break
			}
		}
		if entry == nil {
			return nil, &apiError{status: 404, detail: "CFP journal catalog member not found"}
		}
		snapshots, err := handlers.repository.LoadJournals(context.WithoutCancel(request.Context()))
		if err != nil {
			return nil, internalError()
		}
		snapshot, failure := cfpFindSnapshot(*entry, snapshots)
		if failure != nil {
			return nil, failure
		}
		return cfpPage(handlers.codec, *query.text("db"), *entry, snapshot, includeClosed, cursor, limit, time.Now().Unix())
	})
}

func (handlers *cfpHandlers) respond(writer http.ResponseWriter, request *http.Request, operation func() (any, *apiError)) {
	type result struct {
		payload any
		failure *apiError
	}
	value, err := executor.Run(request.Context(), handlers.pool, func() (result, error) {
		payload, failure := operation()
		return result{payload, failure}, nil
	})
	if err != nil {
		mapExecutorError(err).write(writer)
	} else if value.failure != nil {
		value.failure.write(writer)
	} else {
		writeResponse(writer, value.payload)
	}
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

func cfpSummary(entry sources.JournalCatalogEntry, snapshot *storage.JournalSnapshot, evaluatedAt, now int64) cfpJournalSummary {
	result := cfpJournalSummary{CatalogId: entry.CatalogId, CatalogAliases: entry.CatalogAliases, AllIssns: entry.AllIssns, TitleAliases: entry.TitleAliases, Title: entry.Title, Area: entry.Area, Coverage: "unadapted", RefreshStatus: "unadapted", StateCounts: cfpStateCounts{}}
	if snapshot == nil {
		return result
	}
	result.Coverage, result.CheckedOn, result.SourceUrl, result.SourceStatement = "adapted", &snapshot.CheckedOn, snapshot.SourceUrl, snapshot.SourceStatement
	result.NoticeCount = len(snapshot.Notices)
	for _, notice := range snapshot.Notices {
		state := notice.State(time.Unix(evaluatedAt, 0))
		result.StateCounts[state]++
		if !state.IsArchived() {
			result.CurrentCount++
		}
	}
	for _, registration := range acquisition.Registry() {
		if slices.Contains(registration.CatalogIds, snapshot.JournalKey) {
			result.CanRefresh = registration.CanRefresh()
			break
		}
	}
	var latest *storage.SourceStatus
	for index := range snapshot.Sources {
		source := &snapshot.Sources[index]
		if latest == nil || latest.LastAttempt == nil || source.LastAttempt != nil && *source.LastAttempt >= *latest.LastAttempt {
			latest = source
		}
	}
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
	return result
}

func cfpCatalog(database string, query *string, entries []sources.JournalCatalogEntry, snapshots []storage.JournalSnapshot, now int64) (*cfpCatalogResponse, *apiError) {
	result := &cfpCatalogResponse{Database: database, EvaluatedAt: now, Items: []cfpJournalSummary{}}
	search := ""
	if query != nil {
		search = sources.Lowercase(strings.TrimSpace(*query))
	}
	for _, entry := range entries {
		snapshot, failure := cfpFindSnapshot(entry, snapshots)
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
		fields := append([]string{item.Title, item.CatalogId}, item.AllIssns...)
		fields = append(fields, item.TitleAliases...)
		if item.Area != nil {
			fields = append(fields, *item.Area)
		}
		if search == "" || slices.ContainsFunc(fields, func(field string) bool { return strings.Contains(sources.Lowercase(field), search) }) {
			result.Items = append(result.Items, item)
		}
	}
	slices.SortFunc(result.Items, func(first, second cfpJournalSummary) int {
		if first.Coverage != second.Coverage {
			if first.Coverage == "adapted" {
				return -1
			}
			return 1
		}
		if order := strings.Compare(sources.Lowercase(first.Title), sources.Lowercase(second.Title)); order != 0 {
			return order
		}
		return strings.Compare(first.CatalogId, second.CatalogId)
	})
	return result, nil
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
	encoded, err := auth.EncodeJson([2]any{revisionEntry(entry), revisions})
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
	kind := structBody("NoticeCursor", bodyFieldOf("database", stringBody), bodyFieldOf("catalog_id", stringBody), bodyFieldOf("include_closed", boolBody), bodyFieldOf("revision", stringBody), bodyFieldOf("evaluated_at", integerBody), bodyFieldOf("position", bodyType{kind: "unsigned"}), bodyFieldOf("order", stringBody))
	kind.deniesUnknown = true
	scanner := bodyScanner{body: []byte(plaintext)}
	value, err := scanner.typed(kind, "", 0)
	if err != nil || scanner.peek() != 0 || scanner.position != len(scanner.body) {
		return nil, cfpCursorError()
	}
	fields := value.(map[string]any)
	return &cfpCursor{fields["database"].(string), fields["catalog_id"].(string), fields["include_closed"].(bool), fields["revision"].(string), fields["evaluated_at"].(int64), fields["position"].(uint64), fields["order"].(string)}, nil
}

func cfpPage(codec *secrets.Codec, database string, entry sources.JournalCatalogEntry, snapshot *storage.JournalSnapshot, includeClosed bool, ciphertext *string, limit uint64, now int64) (*cfpNoticePage, *apiError) {
	revision, err := cfpRevision(entry, snapshot)
	if err != nil {
		return nil, internalError()
	}
	cursor := cfpCursor{database, entry.CatalogId, includeClosed, revision, now, 0, "source-v1"}
	if ciphertext != nil {
		decoded, failure := decodeCfpCursor(codec, *ciphertext)
		if failure != nil {
			return nil, failure
		}
		if decoded.Database != database || decoded.CatalogId != entry.CatalogId || decoded.IncludeClosed != includeClosed || decoded.Revision != revision || decoded.Order != "source-v1" || decoded.EvaluatedAt > now || decoded.EvaluatedAt < now-900 {
			return nil, cfpCursorError()
		}
		cursor = *decoded
	}
	items := []cfpNoticeView{}
	if snapshot != nil {
		for _, notice := range snapshot.Notices {
			state := notice.State(time.Unix(cursor.EvaluatedAt, 0))
			if includeClosed || !state.IsArchived() {
				items = append(items, cfpNoticeView{cfpNoticeWire(notice), state, notice.EntryDeadline()})
			}
		}
	}
	if cursor.Position > uint64(len(items)) {
		return nil, cfpCursorError()
	}
	total, offset := int64(len(items)), int64(cursor.Position)
	next := min(cursor.Position+limit, uint64(len(items)))
	hasMore := next < uint64(len(items))
	var nextCursor *string
	if hasMore {
		cursor.Position = next
		encoded, err := auth.EncodeJson(cursor)
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
