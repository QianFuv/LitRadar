package cnki

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	platform "github.com/QianFuv/LitRadar/internal/platform/transport"
	"github.com/QianFuv/LitRadar/internal/sources/jfbym"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const maximumResponseBytes = 2 * 1024 * 1024
const browserUserAgent = "Mozilla/5.0 (compatible; LitRadar/0.1; +https://github.com/QianFuv/LitRadar)"

// LiveConfig selects the per-attempt budget and optional in-memory recognition credential.
type LiveConfig struct {
	TimeoutSeconds uint64
	CaptchaToken   *string
}

func (config LiveConfig) String() string {
	return fmt.Sprintf("LiveDomesticCnkiConfig { timeout_seconds: %d, captcha_token: [REDACTED] }", config.TimeoutSeconds)
}
func (config LiveConfig) GoString() string     { return config.String() }
func (config LiveConfig) LogValue() slog.Value { return slog.StringValue(config.String()) }

// LiveTransport preserves domestic-only network routing, cookies, captcha replay and cooldowns.
type LiveTransport struct{ state *liveState }
type liveState struct {
	gate         chan struct{}
	client       *http.Client
	config       LiveConfig
	proxy        transport.Proxy
	deadline     time.Time
	shared       *sharedCaptcha
	attemptMutex sync.Mutex
	attempts     []scholarly.Attempt
	solver       func(time.Time) (jfbym.Solver, func(), error)
}
type sharedCaptcha struct {
	gate            chan struct{}
	session         *CaptchaSession
	generation      uint64
	cooldownMutex   sync.Mutex
	cooldownStarted time.Time
	cooldown        transport.Delay
}

func newSharedCaptcha() *sharedCaptcha {
	return &sharedCaptcha{gate: make(chan struct{}, 1), session: NewCaptchaSession(CaptchaSolveBudget)}
}
func enter(ctx context.Context, gate chan struct{}) error {
	if ctx.Err() != nil {
		return deadlineError()
	}
	select {
	case gate <- struct{}{}:
		if ctx.Err() != nil {
			<-gate
			return deadlineError()
		}
		return nil
	case <-ctx.Done():
		return deadlineError()
	}
}
func leave(gate chan struct{}) { <-gate }
func deadlineError() error {
	return &Error{Kind: "Request", Message: transport.ErrArticleDeadline.Error()}
}
func sleepRetry(ctx context.Context, delay time.Duration, deadline time.Time) error {
	if !transport.RetrySleep(ctx, delay, deadline) {
		return deadlineError()
	}
	return nil
}
func cloneConfig(config LiveConfig) LiveConfig {
	config.TimeoutSeconds = max(config.TimeoutSeconds, 1)
	if config.CaptchaToken != nil {
		token := *config.CaptchaToken
		config.CaptchaToken = &token
		if strings.TrimSpace(token) == "" {
			config.CaptchaToken = nil
		}
	}
	return config
}
func newHttpClient(proxy transport.Proxy) (*http.Client, error) {
	wire, err := proxy.ClientTransport()
	if err != nil {
		return nil, &Error{Kind: "Request", Message: "domestic CNKI HTTP client initialization failed"}
	}
	return &http.Client{Transport: wire, Jar: platform.NewCookieJar()}, nil
}

// NewLiveTransport creates an explicit-proxy client with a shared optional article deadline.
func NewLiveTransport(config LiveConfig, proxy transport.Proxy, deadline time.Time) (*LiveTransport, error) {
	client, err := newHttpClient(proxy)
	if err != nil {
		return nil, err
	}
	config = cloneConfig(config)
	state := &liveState{gate: make(chan struct{}, 1), client: client, config: config, proxy: proxy, deadline: deadline, shared: newSharedCaptcha(), attempts: []scholarly.Attempt{}}
	state.solver = func(deadline time.Time) (jfbym.Solver, func(), error) {
		solver, err := jfbym.NewLive(valueOrEmpty(config.CaptchaToken), 30, proxy, deadline)
		if err != nil {
			return nil, func() {}, &Error{Kind: "Request", Message: err.Error()}
		}
		return solver, solver.Close, nil
	}
	return &LiveTransport{state: state}, nil
}
func (live LiveTransport) String() string {
	live.state.attemptMutex.Lock()
	count := len(live.state.attempts)
	live.state.attemptMutex.Unlock()
	return fmt.Sprintf("LiveDomesticCnkiTransport { timeout_seconds: %d, captcha_token: [REDACTED], attempt_count: %d }", live.state.config.TimeoutSeconds, count)
}
func (live LiveTransport) GoString() string     { return live.String() }
func (live LiveTransport) LogValue() slog.Value { return slog.StringValue(live.String()) }

// Clone shares the current cookie client and captcha cooldown while copying attempt history.
func (live *LiveTransport) Clone() *LiveTransport {
	enter(context.Background(), live.state.gate)
	defer leave(live.state.gate)
	return &LiveTransport{state: &liveState{gate: make(chan struct{}, 1), client: live.state.client, config: cloneConfig(live.state.config), proxy: live.state.proxy, deadline: live.state.deadline, shared: live.state.shared, attempts: live.Attempts(), solver: live.state.solver}}
}

// Close releases idle connections without affecting an active clone's requests.
func (live *LiveTransport) Close() {
	enter(context.Background(), live.state.gate)
	defer leave(live.state.gate)
	live.state.client.CloseIdleConnections()
}

// ResetTransientState replaces this client's cookies and resets the shared captcha generation.
// Existing clones retain their prior cookie jar and all clones retain the published cooldown.
func (live *LiveTransport) ResetTransientState(ctx context.Context) error {
	if err := enter(ctx, live.state.gate); err != nil {
		return err
	}
	defer leave(live.state.gate)
	client, err := newHttpClient(live.state.proxy)
	if err != nil {
		return err
	}
	if err := enter(ctx, live.state.shared.gate); err != nil {
		client.CloseIdleConnections()
		return err
	}
	live.state.shared.session = NewCaptchaSession(CaptchaSolveBudget)
	live.state.shared.generation++
	leave(live.state.shared.gate)
	live.state.client = client
	return nil
}

// Attempts returns a deep copy of secret-redacted source events.
func (live *LiveTransport) Attempts() []scholarly.Attempt {
	live.state.attemptMutex.Lock()
	defer live.state.attemptMutex.Unlock()
	return copyAttempts(live.state.attempts)
}

// DrainAttempts transfers captured events and leaves an empty buffer.
func (live *LiveTransport) DrainAttempts() []scholarly.Attempt {
	live.state.attemptMutex.Lock()
	defer live.state.attemptMutex.Unlock()
	result := live.state.attempts
	live.state.attempts = []scholarly.Attempt{}
	return result
}
func (live *LiveTransport) record(endpoint, method, url string, status *uint16, isSuccess, didRetry bool, message *string) {
	live.state.attemptMutex.Lock()
	defer live.state.attemptMutex.Unlock()
	live.state.attempts = append(live.state.attempts, scholarly.Attempt{Service: "cnki", Endpoint: endpoint, Method: method, Url: RedactUrl(url), StatusCode: status, DidSucceed: isSuccess, DidRetry: didRetry, Error: message})
}
func textPointer(value string) *string { return &value }
func requestContext(parent context.Context, caller time.Time) (context.Context, context.CancelFunc, time.Time) {
	deadline := transport.LogicalDeadline(caller)
	if limit, ok := parent.Deadline(); ok && limit.Before(deadline) {
		deadline = limit
	}
	ctx, cancel := context.WithDeadline(parent, deadline)
	return ctx, cancel, deadline
}
func (shared *sharedCaptcha) requestUrl(ctx context.Context, url string) (string, uint64, error) {
	if err := enter(ctx, shared.gate); err != nil {
		return "", 0, err
	}
	defer leave(shared.gate)
	request, err := shared.session.AttachCaptchaId(url)
	return request, shared.generation, err
}
func subtractElapsed(delay transport.Delay, elapsed time.Duration) transport.Delay {
	if elapsed <= 0 {
		return delay
	}
	seconds := uint64(elapsed / time.Second)
	nanos := uint32(elapsed % time.Second)
	if delay.Seconds < seconds || delay.Seconds == seconds && delay.Nanoseconds <= nanos {
		return transport.Delay{}
	}
	delay.Seconds -= seconds
	if delay.Nanoseconds < nanos {
		delay.Seconds--
		delay.Nanoseconds += 1_000_000_000
	}
	delay.Nanoseconds -= nanos
	return delay
}
func largerDelay(first, second transport.Delay) transport.Delay {
	if first.Seconds > second.Seconds || first.Seconds == second.Seconds && first.Nanoseconds > second.Nanoseconds {
		return first
	}
	return second
}
func (shared *sharedCaptcha) publishRateLimit(status uint16, headers http.Header, fallback time.Duration) (transport.Delay, bool) {
	if status != 429 {
		return transport.Delay{}, false
	}
	delay, _ := transport.RetryAfter(headers)
	delay = largerDelay(delay, transport.Delay{Seconds: uint64(fallback / time.Second), Nanoseconds: uint32(fallback % time.Second)})
	shared.cooldownMutex.Lock()
	defer shared.cooldownMutex.Unlock()
	now := time.Now()
	remaining := subtractElapsed(shared.cooldown, now.Sub(shared.cooldownStarted))
	shared.cooldown = largerDelay(delay, remaining)
	shared.cooldownStarted = now
	return delay, true
}
func (shared *sharedCaptcha) waitCooldown(ctx context.Context, deadline time.Time) error {
	for {
		if ctx.Err() != nil {
			return deadlineError()
		}
		shared.cooldownMutex.Lock()
		delay := subtractElapsed(shared.cooldown, time.Since(shared.cooldownStarted))
		shared.cooldownMutex.Unlock()
		if delay.Seconds == 0 && delay.Nanoseconds == 0 {
			return nil
		}
		if err := sleepRetry(ctx, delay.Duration(), deadline); err != nil {
			return err
		}
	}
}
func (live *LiveTransport) send(ctx context.Context, deadline time.Time, method, url string, body []byte, headers http.Header) (*http.Response, context.CancelFunc, error) {
	timeout, err := transport.RequestTimeout((transport.Delay{Seconds: live.state.config.TimeoutSeconds}).Duration(), deadline)
	if err != nil || ctx.Err() != nil {
		return nil, func() {}, deadlineError()
	}
	attempt, cancel := context.WithTimeout(ctx, timeout)
	response, err := transport.RedirectSequence(attempt, live.state.client.Transport, live.state.client.Jar, live.state.proxy, method, url, string(body), headers, func(next string, previousCount int) error {
		if previousCount >= 10 {
			return errors.New("domestic CNKI redirect limit exceeded")
		}
		if _, err := parseDomesticUrl(next); err != nil {
			return errors.New("domestic CNKI redirect URL rejected")
		}
		return nil
	})
	if err != nil {
		if response != nil && response.Body != nil {
			response.Body.Close()
		}
		cancel()
		return nil, func() {}, err
	}
	return response, cancel, nil
}

// requestText owns request cancellation and the three independent retry budgets.
func (live *LiveTransport) requestText(parent context.Context, method, url string, data []scholarly.QueryPair, referer *string, endpoint string) (string, error) {
	ctx, cancel, deadline := requestContext(parent, live.state.deadline)
	defer cancel()
	base, headers, body, err := prepareTextRequest(method, url, data, referer)
	if err != nil {
		return "", err
	}
	requestUrl, generation, err := live.state.shared.requestUrl(ctx, base)
	if err != nil {
		return "", err
	}
	var budget requestBudget
	for {
		if _, ok := budget.nextAttempt(); !ok {
			break
		}
		text, finalUrl, status, shouldRetry, err := live.readTextAttempt(ctx, deadline, method, requestUrl, body, headers, endpoint, &budget)
		if err != nil {
			return "", err
		}
		if shouldRetry {
			continue
		}
		didRetry := budget.didRetry()
		if LooksLikeCaptchaChallenge(text, finalUrl) {
			live.record(endpoint, method, requestUrl, &status, false, didRetry, textPointer("captcha challenge"))
			requestUrl, generation, err = live.replayTextCaptcha(ctx, text, finalUrl, base, generation, deadline, &budget)
			if err != nil {
				return "", err
			}
			continue
		}
		shouldRetry, err = live.validateTextAttempt(ctx, deadline, method, requestUrl, endpoint, text, status, &budget)
		if err != nil {
			return "", err
		}
		if shouldRetry {
			continue
		}
		live.record(endpoint, method, requestUrl, &status, true, didRetry, nil)
		return text, nil
	}
	return "", &Error{Kind: "Request", Message: "domestic CNKI request retries exhausted"}
}

// ResolveJournal searches canonical titles and aliases before ISSNs, validating detail identities.
func (live *LiveTransport) ResolveJournal(ctx context.Context, locator JournalLocator) (any, error) {
	if err := enter(ctx, live.state.gate); err != nil {
		return nil, err
	}
	defer leave(live.state.gate)
	return resolveJournal(locator, func(endpoint, url string, data []scholarly.QueryPair) (string, error) {
		method := "GET"
		var referer *string
		if endpoint == "journal_search" {
			method = "POST"
			referer = textPointer(NaviBase + "/knavi")
		}
		return live.requestText(ctx, method, url, data, referer, endpoint)
	})
}

// resolveJournal warms navigation only when the first pass admits no detail URLs.
func resolveJournal(locator JournalLocator, request func(string, string, []scholarly.QueryPair) (string, error)) (any, error) {
	queries := journalResolutionQueries(locator)
	seen := map[string]bool{}
	for pass := 0; pass < 2; pass++ {
		for _, query := range queries {
			details, err := resolveJournalQuery(locator, query, seen, request)
			if err != nil || details != nil {
				return details, err
			}
		}
		if len(seen) > 0 || len(queries) == 0 || pass == 1 {
			break
		}
		text, err := request("navigation", NaviBase+"/knavi/?uniplatform=NZKPT&language=CHS", nil)
		if err != nil {
			return nil, err
		}
		if err := validateResponse("navigation", text); err != nil {
			return nil, err
		}
	}
	return nil, nil
}

// YearIssues requests the original ordered form for one validated journal key.
func (live *LiveTransport) YearIssues(ctx context.Context, journal any) ([]any, error) {
	if err := enter(ctx, live.state.gate); err != nil {
		return nil, err
	}
	defer leave(live.state.gate)
	pykm := jsonText(field(journal, "pykm"))
	if pykm == nil {
		return nil, &Error{Kind: "Parse", Message: "domestic CNKI journal missing pykm"}
	}
	pcode := firstValue(jsonText(field(journal, "pcode")), textPointer("CJFD,CCJD"))
	data := []scholarly.QueryPair{{Name: "pIdx", Value: "0"}, {Name: "time", Value: stringField(journal, "time")}, {Name: "isEpublish", Value: "0"}, {Name: "pcode", Value: *pcode}}
	text, err := live.requestText(ctx, "POST", NaviBase+"/knavi/journals/"+*pykm+"/yearList", data, jsonText(field(journal, "detail_url")), "year_issues")
	if err != nil {
		return nil, err
	}
	return ParseYearIssues(text)
}

// IssueArticles uses the live year-issue token precedence and zero-based page form.
func (live *LiveTransport) IssueArticles(ctx context.Context, journal, issue any, page uint64) (IssueArticlePage, error) {
	if err := enter(ctx, live.state.gate); err != nil {
		return IssueArticlePage{}, err
	}
	defer leave(live.state.gate)
	pykm := jsonText(field(journal, "pykm"))
	if pykm == nil {
		return IssueArticlePage{}, &Error{Kind: "Parse", Message: "domestic CNKI journal missing pykm"}
	}
	token := firstValue(jsonText(field(issue, "year_issue")), jsonText(field(issue, "year_issue_id")))
	if token == nil {
		return IssueArticlePage{}, &Error{Kind: "Parse", Message: "domestic CNKI issue missing year_issue"}
	}
	pcode := firstValue(jsonText(field(journal, "pcode")), textPointer("CJFD,CCJD"))
	data := []scholarly.QueryPair{{Name: "yearIssue", Value: *token}, {Name: "pageIdx", Value: strconv.FormatUint(page, 10)}, {Name: "pcode", Value: *pcode}, {Name: "isEpublish", Value: "0"}, {Name: "language", Value: "CHS"}, {Name: "uniplatform", Value: "NZKPT"}}
	text, err := live.requestText(ctx, "POST", NaviBase+"/knavi/journals/"+*pykm+"/papers", data, jsonText(field(journal, "detail_url")), "issue_articles")
	if err != nil {
		return IssueArticlePage{}, err
	}
	return ParseIssueArticles(text, issue, page)
}

// ArticleDetail retrieves a domestic abstract and validates its permanent-missing markers.
func (live *LiveTransport) ArticleDetail(ctx context.Context, url string, platformId *string) (any, error) {
	if err := enter(ctx, live.state.gate); err != nil {
		return nil, err
	}
	defer leave(live.state.gate)
	text, err := live.requestText(ctx, "GET", url, nil, nil, "article_detail")
	if err != nil {
		return nil, err
	}
	return ParseArticleDetail(text, url)
}

func (live *LiveTransport) solveCaptcha(ctx context.Context, text, url string, generation uint64, deadline time.Time) error {
	if ctx.Err() != nil {
		return deadlineError()
	}
	if live.state.config.CaptchaToken == nil {
		return &Error{Kind: "Request", Message: "domestic CNKI captcha token is required"}
	}
	shared := live.state.shared
	if err := enter(ctx, shared.gate); err != nil {
		return err
	}
	defer leave(shared.gate)
	if shared.generation != generation {
		return nil
	}
	solver, closeSolver, err := live.state.solver(deadline)
	if err != nil {
		return err
	}
	defer closeSolver()
	err = shared.session.EnsureAccess(ctx, text, url, solver, func(ctx context.Context, challenge string) (CaptchaPuzzle, error) {
		response, finish, err := live.captchaRequest(ctx, deadline, "GET", challenge, nil, http.Header{"User-Agent": {browserUserAgent}}, "captcha page")
		if err != nil {
			return CaptchaPuzzle{}, err
		}
		response.Body.Close()
		finish()
		body, err := CaptchaGetRequestBody(challenge)
		if err != nil {
			return CaptchaPuzzle{}, err
		}
		payload, err := live.captchaJson(ctx, deadline, "/verify-api/get", challenge, body, "captcha puzzle")
		if err != nil {
			return CaptchaPuzzle{}, err
		}
		return ParseCaptchaPuzzle(challenge, payload)
	}, func(ctx context.Context, puzzle CaptchaPuzzle, point string) (bool, error) {
		payload, err := live.captchaJson(ctx, deadline, "/verify-api/web/check", puzzle.ChallengeUrl, CaptchaCheckRequestBody(puzzle, point), "captcha check")
		if err != nil {
			return false, err
		}
		return CaptchaCheckSucceeded(payload), nil
	})
	if err == nil {
		shared.generation++
	}
	return err
}
func (live *LiveTransport) captchaRequest(ctx context.Context, deadline time.Time, method, url string, body []byte, headers http.Header, endpoint string) (*http.Response, context.CancelFunc, error) {
	if err := live.state.shared.waitCooldown(ctx, deadline); err != nil {
		return nil, func() {}, err
	}
	response, finish, err := live.send(ctx, deadline, method, url, body, headers)
	if err != nil {
		return nil, func() {}, &Error{Kind: "Request", Message: "domestic CNKI " + endpoint + " request failed"}
	}
	if _, isLimited := live.state.shared.publishRateLimit(uint16(response.StatusCode), response.Header, time.Second); isLimited {
		response.Body.Close()
		finish()
		return nil, func() {}, httpStatusError(429)
	}
	if _, err := parseDomesticUrl(response.Request.URL.String()); err != nil {
		response.Body.Close()
		finish()
		return nil, func() {}, err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		finish()
		return nil, func() {}, httpStatusError(uint16(response.StatusCode))
	}
	return response, finish, nil
}
func (live *LiveTransport) captchaJson(ctx context.Context, deadline time.Time, path, referer string, body any, endpoint string) (any, error) {
	encoded, err := domain.Json(body)
	if err != nil {
		return nil, &Error{Kind: "Parse", Message: "domestic CNKI " + endpoint + " request is not valid JSON"}
	}
	headers := http.Header{"Content-Type": {"application/json;charset=UTF-8"}, "Origin": {KnsBase}, "Referer": {referer}, "X-Requested-With": {"XMLHttpRequest"}}
	response, finish, err := live.captchaRequest(ctx, deadline, "POST", KnsBase+path, encoded, headers, endpoint)
	if err != nil {
		return nil, err
	}
	defer finish()
	raw, err := transport.BoundedBytes(response, maximumResponseBytes)
	if err != nil {
		kind, message := "Request", "response read failed"
		if errors.Is(err, transport.ErrTooLarge) {
			kind, message = "Parse", "response exceeded the configured size limit"
		}
		return nil, &Error{Kind: kind, Message: "domestic CNKI " + endpoint + " " + message}
	}
	payload, err := transport.ParseJson(raw)
	if err != nil {
		return nil, &Error{Kind: "Parse", Message: "domestic CNKI " + endpoint + " response is not valid JSON"}
	}
	return payload, nil
}

// prepareTextRequest validates the URL and referer before encoding a POST body.
func prepareTextRequest(method, url string, data []scholarly.QueryPair, referer *string) (string, http.Header, []byte, error) {
	parsed, err := parseDomesticUrl(url)
	if err != nil {
		return "", nil, nil, err
	}
	base := parsed.Href(false)
	headers := http.Header{"User-Agent": {browserUserAgent}}
	if referer != nil {
		parsed, err := parseDomesticUrl(*referer)
		if err != nil {
			return "", nil, nil, err
		}
		headers.Set("Referer", parsed.Href(false))
	}
	var body []byte
	if method == "POST" {
		body = []byte(scholarly.EncodeQuery(data))
		headers.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	return base, headers, body, nil
}

// readTextAttempt admits one request and finishes its response before captcha processing.
func (live *LiveTransport) readTextAttempt(ctx context.Context, deadline time.Time, method, requestUrl string, body []byte, headers http.Header, endpoint string, budget *requestBudget) (string, string, uint16, bool, error) {
	if err := live.state.shared.waitCooldown(ctx, deadline); err != nil {
		return "", "", 0, false, err
	}
	didRetry := budget.didRetry()
	if ctx.Err() != nil {
		return "", "", 0, false, deadlineError()
	}
	response, finish, err := live.send(ctx, deadline, method, requestUrl, body, headers)
	if err != nil {
		live.record(endpoint, method, requestUrl, nil, false, didRetry, textPointer("request failed"))
		if budget.scheduleTransportRetry() {
			if err := sleepRetry(ctx, budget.transportRetryDelay(), deadline); err != nil {
				return "", "", 0, false, err
			}
			return "", "", 0, true, nil
		}
		return "", "", 0, false, &Error{Kind: "Request", Message: "domestic CNKI request failed"}
	}
	status := uint16(response.StatusCode)
	finalUrl := response.Request.URL.String()
	shouldRetry, err := live.admitTextResponse(response, finish, deadline, method, requestUrl, endpoint, finalUrl, budget)
	if err != nil || shouldRetry {
		return "", "", 0, shouldRetry, err
	}
	text, err := transport.BoundedText(response, maximumResponseBytes)
	finish()
	if err != nil {
		message := "domestic CNKI response read failed"
		if errors.Is(err, transport.ErrTooLarge) {
			message = "domestic CNKI response exceeded the configured size limit"
		}
		live.record(endpoint, method, requestUrl, &status, false, didRetry, &message)
		return "", "", 0, false, &Error{Kind: "Request", Message: message}
	}
	return text, finalUrl, status, false, nil
}

// validateTextAttempt checks overseas markers before HTTP status and endpoint structure.
func (live *LiveTransport) validateTextAttempt(ctx context.Context, deadline time.Time, method, requestUrl, endpoint, text string, status uint16, budget *requestBudget) (bool, error) {
	didRetry := budget.didRetry()
	if ContainsOverseasHost(text) {
		live.record(endpoint, method, requestUrl, &status, false, didRetry, textPointer("overseas host"))
		return false, &Error{Kind: "Request", Message: "domestic CNKI transport received overseas host"}
	}
	if status < 200 || status >= 300 {
		return live.retryTextStatus(ctx, deadline, method, requestUrl, endpoint, status, budget)
	}
	if err := validateResponse(endpoint, text); err != nil {
		live.record(endpoint, method, requestUrl, &status, false, didRetry, textPointer("invalid response"))
		var failure *Error
		if errors.As(err, &failure) && failure.Kind == "Parse" {
			if delay, ok := budget.ordinaryRetryDelay(); ok {
				if err := sleepRetry(ctx, delay, deadline); err != nil {
					return false, err
				}
				return true, nil
			}
		}
		return false, err
	}
	return false, nil
}

// retryTextStatus preserves the terminal 404 and 410 exceptions.
func (live *LiveTransport) retryTextStatus(ctx context.Context, deadline time.Time, method, requestUrl, endpoint string, status uint16, budget *requestBudget) (bool, error) {
	didRetry := budget.didRetry()
	live.record(endpoint, method, requestUrl, &status, false, didRetry, textPointer("HTTP status"))
	if status != 404 && status != 410 {
		if delay, ok := budget.ordinaryRetryDelay(); ok {
			if err := sleepRetry(ctx, delay, deadline); err != nil {
				return false, err
			}
			return true, nil
		}
	}
	return false, httpStatusError(status)
}

// admitTextResponse rejects redirected URLs and publishes 429 cooldown before body access.
func (live *LiveTransport) admitTextResponse(response *http.Response, finish func(), deadline time.Time, method, requestUrl, endpoint, finalUrl string, budget *requestBudget) (bool, error) {
	status := uint16(response.StatusCode)
	didRetry := budget.didRetry()
	if _, err := parseDomesticUrl(finalUrl); err != nil {
		response.Body.Close()
		finish()
		live.record(endpoint, method, requestUrl, &status, false, didRetry, textPointer("redirect URL rejected"))
		return false, &Error{Kind: "Request", Message: "domestic CNKI redirect URL is not allowed"}
	}
	if delay, isLimited := live.state.shared.publishRateLimit(status, response.Header, retryDelay(budget.ordinaryAttempts)); isLimited {
		response.Body.Close()
		finish()
		live.record(endpoint, method, requestUrl, &status, false, didRetry, textPointer("HTTP status"))
		if _, ok := budget.ordinaryRetryDelay(); ok && transport.CanWait(delay.Duration(), deadline) {
			return true, nil
		}
		return false, httpStatusError(429)
	}
	return false, nil
}

// journalResolutionQueries keeps title variants before the original ISSN order.
func journalResolutionQueries(locator JournalLocator) [][2]string {
	queries := [][2]string{}
	for _, title := range locator.Titles() {
		queries = append(queries, [2]string{title, "TI"})
		ascii := strings.NewReplacer("（", "(", "）", ")").Replace(title)
		if ascii != title {
			queries = append(queries, [2]string{ascii, "TI"})
		}
	}
	for _, issn := range locator.Issns() {
		queries = append(queries, [2]string{issn, "SN"})
	}
	return queries
}

// resolveJournalQuery visits unseen details once, recording admission before each fetch.
func resolveJournalQuery(locator JournalLocator, query [2]string, seen map[string]bool, request func(string, string, []scholarly.QueryPair) (string, error)) (any, error) {
	data := journalSearchPairs(query)
	text, err := request("journal_search", NaviBase+"/knavi/journals/searchbaseinfo", data)
	if err != nil {
		return nil, err
	}
	candidates, err := ParseJournalSearch(text)
	if err != nil {
		return nil, err
	}
	for _, candidate := range candidates {
		url, ok := field(candidate, "detail_url").(string)
		if !ok || seen[url] {
			continue
		}
		seen[url] = true
		details, err := resolveJournalDetail(locator, url, request)
		if err != nil || details != nil {
			return details, err
		}
	}
	return nil, nil
}

// journalSearchPairs preserves sorted search-form fields.
func journalSearchPairs(query [2]string) []scholarly.QueryPair {
	form := JournalSearchForm(query[0], query[1])
	keys := make([]string, 0, len(form))
	for key := range form {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	data := []scholarly.QueryPair{}
	for _, key := range keys {
		data = append(data, scholarly.QueryPair{Name: key, Value: form[key]})
	}
	return data
}

// resolveJournalDetail skips absent identity fields and returns only matching details.
func resolveJournalDetail(locator JournalLocator, url string, request func(string, string, []scholarly.QueryPair) (string, error)) (any, error) {
	detailText, err := request("journal_detail", url, nil)
	if err != nil {
		return nil, err
	}
	if inputValue(detailText, "pykm") == nil {
		return nil, nil
	}
	details, err := ParseJournalDetail(detailText)
	if err != nil {
		return nil, err
	}
	if JournalDetailMatches(details, locator) {
		return details, nil
	}
	return nil, nil
}

// replayTextCaptcha renews the request URL before admitting a captcha replay.
func (live *LiveTransport) replayTextCaptcha(ctx context.Context, text, finalUrl, base string, generation uint64, deadline time.Time, budget *requestBudget) (string, uint64, error) {
	if err := live.solveCaptcha(ctx, text, finalUrl, generation, deadline); err != nil {
		return "", 0, err
	}
	requestUrl, generation, err := live.state.shared.requestUrl(ctx, base)
	if err != nil {
		return "", 0, err
	}
	if err := budget.scheduleCaptchaReplay(); err != nil {
		return "", 0, err
	}
	return requestUrl, generation, nil
}
