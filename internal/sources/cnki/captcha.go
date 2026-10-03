package cnki

import (
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/sources/jfbym"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const CaptchaSolveBudget = 5

// CaptchaPuzzle carries memory-only challenge data with secret-safe formatting.
type CaptchaPuzzle struct {
	ChallengeUrl, CaptchaType, Ident, CaptchaId, ReturnUrl   string
	SecretKey, Token, OriginalImageBase64, JigsawImageBase64 string
}

func (puzzle CaptchaPuzzle) String() string {
	return fmt.Sprintf("DomesticCaptchaPuzzle { captcha_type_len: %d, captcha_id_len: %d, secret_key_len: %d, token_len: %d, original_len: %d, jigsaw_len: %d }", len(puzzle.CaptchaType), len(puzzle.CaptchaId), len(puzzle.SecretKey), len(puzzle.Token), len(puzzle.OriginalImageBase64), len(puzzle.JigsawImageBase64))
}
func (puzzle CaptchaPuzzle) GoString() string     { return puzzle.String() }
func (puzzle CaptchaPuzzle) LogValue() slog.Value { return slog.StringValue(puzzle.String()) }

func parseChallengeUrl(value string) (string, error) {
	decoded := decodeHtml(value)
	if strings.HasPrefix(decoded, "/verify/home") {
		decoded = KnsBase + decoded
	}
	parsed, err := parseDomesticUrl(decoded)
	if err != nil {
		return "", err
	}
	if parsed.Hostname() != "kns.cnki.net" || parsed.Pathname() != "/verify/home" {
		return "", &Error{Kind: "Request", Message: "domestic CNKI challenge URL is not allowed"}
	}
	return parsed.Href(false), nil
}

// LooksLikeCaptchaChallenge preserves domestic-content and overseas exclusions.
func LooksLikeCaptchaChallenge(text, url string) bool {
	if ContainsOverseasHost(text) || ContainsOverseasHost(url) || looksLikeContent(text) {
		return false
	}
	urlPath := url
	if index := strings.IndexAny(url, "?#"); index >= 0 {
		urlPath = url[:index]
	}
	return containsMarker(domain.Lowercase(text), "captcha") || containsMarker(text, "访问异常", "安全验证", `"code":-403`, "/verify/home") || strings.Contains(domain.Lowercase(urlPath), "/verify/home")
}

// ExtractChallengeUrl accepts only the domestic verification endpoint, without executing scripts.
func ExtractChallengeUrl(text, url string) (*string, error) {
	parse := func(value string) (*string, error) {
		parsed, err := parseChallengeUrl(value)
		if err != nil {
			return nil, err
		}
		return &parsed, nil
	}
	if strings.Contains(url, "/verify/home") {
		return parse(url)
	}
	if payload, err := transport.ParseJson([]byte(text)); err == nil {
		if message, ok := field(payload, "message").(string); ok && strings.Contains(message, "/verify/home") {
			return parse(message)
		}
	}
	for _, marker := range []string{"https://kns.cnki.net/verify/home?", "/verify/home?"} {
		if start := strings.Index(text, marker); start >= 0 {
			rest := text[start:]
			if end := strings.IndexFunc(rest, func(character rune) bool { return unicode.IsSpace(character) || character == '"' || character == '\'' }); end >= 0 {
				rest = rest[:end]
			}
			return parse(rest)
		}
	}
	return nil, nil
}
func queryMap(url string) (map[string]string, error) {
	parsed, err := parseDomesticUrl(decodeHtml(url))
	if err != nil {
		return nil, err
	}
	values := map[string]string{}
	for _, pair := range queryPairs(parsed.Search()) {
		values[pair.Name] = pair.Value
	}
	return values, nil
}
func stringField(value any, key string) string { text, _ := field(value, key).(string); return text }
func sortedValues(value any) []any {
	object, _ := value.(map[string]any)
	keys := make([]string, 0, len(object))
	for key := range object {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	values := make([]any, 0, len(keys))
	for _, key := range keys {
		values = append(values, object[key])
	}
	return values
}
func hasImage(value any) bool {
	object, _ := value.(map[string]any)
	_, exists := object["originalImageBase64"]
	return exists
}
func puzzleContainer(body any) any {
	if hasImage(body) {
		return body
	}
	for _, key := range []string{"repData", "data"} {
		value := field(body, key)
		if hasImage(value) {
			return value
		}
		for _, nested := range sortedValues(value) {
			if hasImage(nested) {
				return nested
			}
		}
	}
	for _, value := range sortedValues(body) {
		if hasImage(value) {
			return value
		}
	}
	return nil
}
func validatePuzzle(puzzle CaptchaPuzzle) error {
	if puzzle.OriginalImageBase64 == "" || puzzle.JigsawImageBase64 == "" || puzzle.SecretKey == "" || puzzle.Token == "" || puzzle.CaptchaId == "" {
		return &Error{Kind: "Parse", Message: "domestic captcha puzzle missing required fields"}
	}
	if len(puzzle.SecretKey) != 16 {
		return &Error{Kind: "Parse", Message: fmt.Sprintf("domestic captcha secretKey length is %d, expected 16", len(puzzle.SecretKey))}
	}
	return nil
}

// ParseCaptchaPuzzle preserves container precedence, decoded query overrides and byte-length validation.
func ParseCaptchaPuzzle(challengeUrl string, body any) (CaptchaPuzzle, error) {
	challenge, err := parseChallengeUrl(challengeUrl)
	if err != nil {
		return CaptchaPuzzle{}, err
	}
	query, err := queryMap(challenge)
	if err != nil {
		return CaptchaPuzzle{}, err
	}
	container := puzzleContainer(body)
	if container == nil {
		return CaptchaPuzzle{}, &Error{Kind: "Parse", Message: "domestic captcha puzzle missing image container"}
	}
	kind, exists := query["captchaType"]
	if !exists {
		kind, exists = field(container, "captchaType").(string)
		if !exists {
			kind = "blockPuzzle"
		}
	}
	id, exists := query["captchaId"]
	if !exists {
		id = stringField(container, "captchaId")
	}
	puzzle := CaptchaPuzzle{ChallengeUrl: challenge, CaptchaType: kind, Ident: query["ident"], CaptchaId: id, ReturnUrl: query["returnUrl"], SecretKey: stringField(container, "secretKey"), Token: stringField(container, "token"), OriginalImageBase64: jfbym.StripDataUrlBase64(stringField(container, "originalImageBase64")), JigsawImageBase64: jfbym.StripDataUrlBase64(stringField(container, "jigsawImageBase64"))}
	return puzzle, validatePuzzle(puzzle)
}

// CaptchaGetRequestBody generates the timestamp-based request identity used by the upstream client.
func CaptchaGetRequestBody(challengeUrl string) (map[string]any, error) {
	query, err := queryMap(challengeUrl)
	if err != nil {
		return nil, err
	}
	kind, exists := query["captchaType"]
	if !exists {
		kind = "blockPuzzle"
	}
	now := time.Now()
	nanos := max(now.UnixNano(), 0)
	millis := max(now.UnixMilli(), 0)
	return map[string]any{"captchaType": kind, "clientUid": fmt.Sprintf("%032x", nanos), "ts": millis, "ident": query["ident"], "captchaId": query["captchaId"]}, nil
}

// CaptchaCheckRequestBody submits the encrypted point with the original five-field shape.
func CaptchaCheckRequestBody(puzzle CaptchaPuzzle, pointJson string) map[string]any {
	return map[string]any{"captchaType": puzzle.CaptchaType, "pointJson": pointJson, "token": puzzle.Token, "ident": puzzle.Ident, "returnUrl": puzzle.ReturnUrl}
}
func successValue(value any) bool {
	switch typed := value.(type) {
	case bool:
		return typed
	case string:
		return typed == "true"
	case int:
		return typed == 1
	case int64:
		return typed == 1
	case uint64:
		return typed == 1
	case json.Number:
		number, err := domain.ParseNumber(typed)
		if err != nil {
			return false
		}
		integer, ok := number.AsInt64()
		return ok && integer == 1
	case domain.Number:
		integer, ok := typed.AsInt64()
		return ok && integer == 1
	}
	return false
}

// CaptchaCheckSucceeded recognizes exact boolean, text or integer success markers.
func CaptchaCheckSucceeded(body any) bool {
	if successValue(field(body, "success")) {
		return true
	}
	object, _ := body.(map[string]any)
	container, exists := object["repData"]
	if !exists {
		container = object["data"]
	}
	return successValue(field(container, "result"))
}

// CaptchaSession serializes solve budgets and keeps the successful challenge id in memory.
type CaptchaSession struct{ state *captchaState }

type captchaState struct {
	mutex                      *sync.Mutex
	solveGate                  chan struct{}
	captchaId                  string
	solveAttempts, solveBudget uint64
}

// NewCaptchaSession creates a session with at least one fresh puzzle attempt.
func NewCaptchaSession(budget uint64) *CaptchaSession {
	return &CaptchaSession{state: &captchaState{mutex: &sync.Mutex{}, solveGate: make(chan struct{}, 1), solveBudget: max(budget, 1)}}
}
func (session CaptchaSession) String() string {
	session.state.mutex.Lock()
	defer session.state.mutex.Unlock()
	return fmt.Sprintf("DomesticCaptchaSession { has_captcha_id: %t, captcha_id_len: %d, solve_attempts: %d, solve_budget: %d }", session.state.captchaId != "", len(session.state.captchaId), session.state.solveAttempts, session.state.solveBudget)
}
func (session CaptchaSession) GoString() string     { return session.String() }
func (session CaptchaSession) LogValue() slog.Value { return slog.StringValue(session.String()) }

// RemainingBudget returns the unused count without revealing a challenge identifier.
func (session *CaptchaSession) RemainingBudget() uint64 {
	session.state.mutex.Lock()
	defer session.state.mutex.Unlock()
	return session.state.solveBudget - session.state.solveAttempts
}

// Clone copies the current budget and retained id into an independent session.
func (session *CaptchaSession) Clone() *CaptchaSession {
	session.state.mutex.Lock()
	defer session.state.mutex.Unlock()
	return &CaptchaSession{state: &captchaState{mutex: &sync.Mutex{}, solveGate: make(chan struct{}, 1), captchaId: session.state.captchaId, solveAttempts: session.state.solveAttempts, solveBudget: session.state.solveBudget}}
}

// AttachCaptchaId preserves an existing exact captchaId key and untouched URL query encoding.
func (session *CaptchaSession) AttachCaptchaId(url string) (string, error) {
	parsed, err := parseDomesticUrl(url)
	if err != nil {
		return "", err
	}
	session.state.mutex.Lock()
	id := session.state.captchaId
	session.state.mutex.Unlock()
	if id == "" {
		return parsed.Href(false), nil
	}
	for _, pair := range queryPairs(parsed.Search()) {
		if pair.Name == "captchaId" {
			return parsed.Href(false), nil
		}
	}
	query := strings.TrimPrefix(parsed.Search(), "?")
	if query != "" {
		query += "&"
	}
	query += scholarly.EncodeQuery([]scholarly.QueryPair{{Name: "captchaId", Value: id}})
	parsed.SetSearch(query)
	return parsed.Href(false), nil
}

// EnsureAccess only consumes the budget when a verification challenge is detected.
func (session *CaptchaSession) EnsureAccess(ctx context.Context, text, url string, solver jfbym.Solver, fetch func(context.Context, string) (CaptchaPuzzle, error), submit func(context.Context, CaptchaPuzzle, string) (bool, error)) error {
	if !LooksLikeCaptchaChallenge(text, url) {
		return nil
	}
	challenge, err := ExtractChallengeUrl(text, url)
	if err != nil {
		return err
	}
	if challenge == nil {
		return &Error{Kind: "Request", Message: "domestic CNKI verification required but challenge URL is missing"}
	}
	return session.SolveChallenge(ctx, *challenge, solver, fetch, submit)
}

// SolveChallenge fetches one fresh puzzle for each candidate and retains only an accepted id.
func (session *CaptchaSession) SolveChallenge(ctx context.Context, url string, solver jfbym.Solver, fetch func(context.Context, string) (CaptchaPuzzle, error), submit func(context.Context, CaptchaPuzzle, string) (bool, error)) error {
	select {
	case session.state.solveGate <- struct{}{}:
		defer func() { <-session.state.solveGate }()
	case <-ctx.Done():
		return &Error{Kind: "Request", Message: "article access deadline expired"}
	}
	for {
		if err := ctx.Err(); err != nil {
			return &Error{Kind: "Request", Message: "article access deadline expired"}
		}
		session.state.mutex.Lock()
		if session.state.solveAttempts >= session.state.solveBudget {
			attempts := session.state.solveAttempts
			session.state.mutex.Unlock()
			return &Error{Kind: "Request", Message: fmt.Sprintf("domestic CNKI captcha solve budget exhausted after %d attempts", attempts)}
		}
		session.state.solveAttempts++
		attempt := session.state.solveAttempts
		session.state.mutex.Unlock()
		puzzle, err := fetch(ctx, url)
		if err != nil {
			return err
		}
		if err = validatePuzzle(puzzle); err != nil {
			return err
		}
		distance, err := solver.SolveDualImage(ctx, puzzle.JigsawImageBase64, puzzle.OriginalImageBase64)
		if err != nil {
			return &Error{Kind: "Request", Message: err.Error()}
		}
		candidates, err := jfbym.PointXCandidates(distance)
		if err != nil {
			return &Error{Kind: "Request", Message: err.Error()}
		}
		point, err := jfbym.EncryptPointJson(puzzle.SecretKey, candidates[(attempt-1)%uint64(len(candidates))], 5)
		if err != nil {
			return &Error{Kind: "Request", Message: err.Error()}
		}
		accepted, err := submit(ctx, puzzle, point)
		if err != nil {
			return err
		}
		if accepted {
			session.state.mutex.Lock()
			session.state.captchaId = puzzle.CaptchaId
			session.state.mutex.Unlock()
			return nil
		}
	}
}
