package cnki

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"reflect"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/sources/jfbym"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const minimalArticle = `<input id="paramfilename" value="id"><h1 class="title">Article</h1>`

// loopbackLive retains public HTTPS authorities while routing sockets exclusively to a trusted local server.
func loopbackLive(t *testing.T, handler http.HandlerFunc, deadline time.Time) *LiveTransport {
	t.Helper()
	server := httptest.NewTLSServer(handler)
	t.Cleanup(server.Close)
	live, err := NewLiveTransport(LiveConfig{TimeoutSeconds: 2}, transport.Proxy{}, deadline)
	if err != nil {
		t.Fatal(err)
	}
	wire := live.state.client.Transport.(*transport.ClientTransport)
	roots := x509.NewCertPool()
	roots.AddCert(server.Certificate())
	wire.TLSClientConfig = &tls.Config{RootCAs: roots, ServerName: "127.0.0.1"}
	wire.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, network, server.Listener.Addr().String())
	}
	t.Cleanup(live.Close)
	return live
}

func TestLiveDomesticFormsAndCookieClone(t *testing.T) {
	var mutex sync.Mutex
	captures := []string{}
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
		body, _ := io.ReadAll(request.Body)
		mutex.Lock()
		captures = append(captures, request.Host+" "+request.Method+" "+request.URL.RequestURI()+" "+string(body))
		mutex.Unlock()
		if request.Header.Get("Accept") != "*/*" || request.Header.Get("User-Agent") != browserUserAgent {
			t.Error("missing browser request defaults")
		}
		switch request.URL.Path {
		case "/knavi/journals/J/yearList":
			if request.Header.Get("Referer") != NaviBase+"/knavi/detail?x=1" {
				t.Error(request.Header)
			}
			http.SetCookie(writer, &http.Cookie{Name: "shared", Value: "cookie", Domain: "cnki.net", Path: "/", Secure: true})
			io.WriteString(writer, `<div id="YearIssueTree"><a id="yq202501" value="opaque">No.1</a></div>`)
		case "/knavi/journals/J/papers":
			if cookie, err := request.Cookie("shared"); err != nil || cookie.Value != "cookie" {
				t.Error("cookie not shared")
			}
			io.WriteString(writer, `<input id="articleCount" value="0">`)
		default:
			if cookie, err := request.Cookie("shared"); err != nil || cookie.Value != "cookie" {
				t.Error("clone cookie not shared")
			}
			io.WriteString(writer, minimalArticle)
		}
	}, time.Time{})
	journal := map[string]any{"pykm": "J", "time": "a b+~", "detail_url": NaviBase + "/knavi/detail?x=1"}
	issues, err := live.YearIssues(context.Background(), journal)
	if err != nil || len(issues) != 1 {
		t.Fatalf("%v %v", issues, err)
	}
	issue := map[string]any{"year_issue": "opaque +~", "year_issue_id": "wrong"}
	if _, err := live.IssueArticles(context.Background(), journal, issue, 7); err != nil {
		t.Fatal(err)
	}
	clone := live.Clone()
	if _, err := clone.ArticleDetail(context.Background(), KnsBase+"/kcms2/article/abstract?v=id", nil); err != nil {
		t.Fatal(err)
	}
	want := []string{"navi.cnki.net POST /knavi/journals/J/yearList pIdx=0&time=a+b%2B%7E&isEpublish=0&pcode=CJFD%2CCCJD", "navi.cnki.net POST /knavi/journals/J/papers yearIssue=opaque+%2B%7E&pageIdx=7&pcode=CJFD%2CCCJD&isEpublish=0&language=CHS&uniplatform=NZKPT", "kns.cnki.net GET /kcms2/article/abstract?v=id "}
	if !reflect.DeepEqual(captures, want) {
		t.Fatalf("captures %q", captures)
	}
	if len(live.Attempts()) != 2 || len(clone.Attempts()) != 3 {
		t.Fatal("clones share attempt buffer")
	}
}

func TestLiveDomesticResponsePrecedence(t *testing.T) {
	for _, test := range []struct {
		name   string
		status int
		body   string
		length int
		want   string
	}{{"missing", 200, "record does not exist", 0, "domestic CNKI article is permanently unavailable"}, {"http404", 404, "record does not exist", 0, "domestic CNKI HTTP status 404"}, {"overseas_before404", 404, "oversea.cnki.net", 0, "domestic CNKI transport received overseas host"}, {"captcha_before404", 404, "captcha", 0, "domestic CNKI captcha token is required"}, {"oversize_before404", 404, "", maximumResponseBytes + 1, "domestic CNKI response exceeded the configured size limit"}, {"truncated_before404", 404, "short", 100, "domestic CNKI response read failed"}} {
		t.Run(test.name, func(t *testing.T) {
			var count atomic.Int32
			live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
				count.Add(1)
				if test.length > 0 {
					writer.Header().Set("Content-Length", fmt.Sprint(test.length))
				}
				writer.WriteHeader(test.status)
				io.WriteString(writer, test.body)
			}, time.Now().Add(500*time.Millisecond))
			_, err := live.ArticleDetail(context.Background(), KnsBase+"/a", nil)
			if err == nil || err.Error() != test.want {
				t.Fatalf("%v want %s", err, test.want)
			}
			if count.Load() != 1 || len(live.Attempts()) != 1 {
				t.Fatal("terminal response retried")
			}
		})
	}
}

func TestLiveDomesticRejectsRedirectBeforeTransmission(t *testing.T) {
	for _, location := range []string{"https://oversea.cnki.net/private", "http://kns.cnki.net/private", "https://user:pass@kns.cnki.net/private", "https://kns.cnki.net:444/private"} {
		t.Run(location, func(t *testing.T) {
			var count atomic.Int32
			live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
				count.Add(1)
				writer.Header().Set("Location", location)
				writer.WriteHeader(302)
			}, time.Now().Add(500*time.Millisecond))
			_, err := live.ArticleDetail(context.Background(), KnsBase+"/a", nil)
			if err == nil || err.Error() != "article access deadline expired" {
				t.Fatal(err)
			}
			if count.Load() != 1 {
				t.Fatal("forbidden redirect transmitted")
			}
			attempts := live.Attempts()
			if len(attempts) != 1 || attempts[0].StatusCode != nil || *attempts[0].Error != "request failed" {
				t.Fatal(attempts)
			}
		})
	}
}

// TestLiveDomesticRateLimitBeforeBodyAndSharedCooldown verifies header-first shared throttling.
func TestLiveDomesticRateLimitBeforeBodyAndSharedCooldown(t *testing.T) {
	var requests atomic.Int32
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
		requests.Add(1)
		writer.Header().Set("Retry-After", "18446744073709551615")
		writer.Header().Set("Content-Length", fmt.Sprint(maximumResponseBytes+1))
		writer.WriteHeader(429)
		writer.(http.Flusher).Flush()
		<-request.Context().Done()
	}, time.Now().Add(time.Second))
	clone := live.Clone()
	started := time.Now()
	_, err := live.ArticleDetail(context.Background(), KnsBase+"/a", nil)
	if err == nil || err.Error() != "domestic CNKI HTTP status 429" {
		t.Fatal(err)
	}
	if time.Since(started) > 500*time.Millisecond {
		t.Fatal("429 consumed its body")
	}
	assertCloneCooldown(t, live, clone, &requests)
}

// TestLiveDomesticCaptchaReplayKeepsCookieAndRedactsAttempts verifies the replay request contract.
func TestLiveDomesticCaptchaReplayKeepsCookieAndRedactsAttempts(t *testing.T) {
	var mutex sync.Mutex
	paths := []string{}
	var ordinary atomic.Int32
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
		mutex.Lock()
		paths = append(paths, request.URL.Path)
		mutex.Unlock()
		switch request.URL.Path {
		case "/article":
			ordinary.Add(1)
			if request.URL.Query().Get("captchaId") == "solved-private-id" {
				if cookie, err := request.Cookie("captcha"); err != nil || cookie.Value != "ok" {
					t.Error("captcha cookie not replayed")
				}
				io.WriteString(writer, minimalArticle)
				return
			}
			io.WriteString(writer, `{"code":-403,"message":"https://kns.cnki.net/verify/home?captchaId=solved-private-id&ident=private-ident"}`)
		case "/verify/home":
			if request.Header.Get("User-Agent") != browserUserAgent {
				t.Error("page UA")
			}
			http.SetCookie(writer, &http.Cookie{Name: "captcha", Value: "ok", Path: "/", Secure: true})
			io.WriteString(writer, "page body intentionally unread")
		case "/verify-api/get", "/verify-api/web/check":
			serveCaptchaJson(t, writer, request)
		default:
			t.Error(request.URL)
		}
	}, time.Now().Add(3*time.Second))
	live.state.config.CaptchaToken = textPointer("recognition-secret")
	live.state.solver = func(time.Time) (jfbym.Solver, func(), error) { return jfbym.NewFixture(10.4, 0), func() {}, nil }
	article, err := live.ArticleDetail(context.Background(), KnsBase+"/article", nil)
	if err != nil || field(article, "title") != "Article" {
		t.Fatalf("%v %v", article, err)
	}
	want := []string{"/article", "/verify/home", "/verify-api/get", "/verify-api/web/check", "/article"}
	if !reflect.DeepEqual(paths, want) || ordinary.Load() != 2 {
		t.Fatal(paths)
	}
	assertCaptchaAttemptRedaction(t, live)
}

func TestLiveDomesticRetryThenSuccess(t *testing.T) {
	var count atomic.Int32
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
		if count.Add(1) == 1 {
			io.WriteString(writer, "incomplete")
			return
		}
		io.WriteString(writer, minimalArticle)
	}, time.Now().Add(4*time.Second))
	article, err := live.ArticleDetail(context.Background(), KnsBase+"/article", nil)
	if err != nil || field(article, "title") != "Article" {
		t.Fatal(err)
	}
	attempts := live.Attempts()
	if len(attempts) != 2 || attempts[0].DidRetry || !attempts[1].DidRetry || *attempts[0].Error != "invalid response" {
		t.Fatal(attempts)
	}
}

func TestLiveDomesticResetLeavesCloneCookies(t *testing.T) {
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) { io.WriteString(writer, minimalArticle) }, time.Time{})
	location, _ := url.Parse(KnsBase)
	live.state.client.Jar.SetCookies(location, []*http.Cookie{{Name: "session", Value: "old", Path: "/"}})
	clone := live.Clone()
	live.state.shared.session.state.captchaId = "private"
	generation := live.state.shared.generation
	if err := live.ResetTransientState(context.Background()); err != nil {
		t.Fatal(err)
	}
	if len(live.state.client.Jar.Cookies(location)) != 0 || len(clone.state.client.Jar.Cookies(location)) != 1 {
		t.Fatal("reset cookie ownership changed")
	}
	if live.state.shared.generation != generation+1 || clone.state.shared.session.state.captchaId != "" {
		t.Fatal("captcha reset not shared")
	}
}

func TestLiveDomesticRedirectUpdatesReferer(t *testing.T) {
	var mutex sync.Mutex
	observed := []string{}
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
		mutex.Lock()
		observed = append(observed, request.Method+" "+request.Header.Get("Referer")+" "+request.Header.Get("Content-Type"))
		mutex.Unlock()
		switch request.URL.Path {
		case "/knavi/journals/J/yearList":
			writer.Header().Set("Location", "/first#fragment")
			writer.WriteHeader(302)
		case "/first":
			writer.Header().Set("Location", KnsBase+"/last")
			writer.WriteHeader(302)
		default:
			io.WriteString(writer, "YearIssueTree")
		}
	}, time.Time{})
	_, err := live.YearIssues(context.Background(), map[string]any{"pykm": "J", "detail_url": NaviBase + "/detail"})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"POST " + NaviBase + "/detail application/x-www-form-urlencoded", "GET " + NaviBase + "/knavi/journals/J/yearList ", "GET " + NaviBase + "/first "}
	if !reflect.DeepEqual(observed, want) {
		t.Fatalf("observed %q want %q", observed, want)
	}
}

func TestLiveDomesticRedirectNormalization(t *testing.T) {
	for _, test := range []struct {
		location, final string
		calls           int
		status          int
	}{
		{`/kns55/a\b`, KnsBase + "/kns55/a/b", 2, 200},
		{"/last#oversea", KnsBase + "/last", 2, 200},
		{"", KnsBase + "/start", 2, 200},
		{"http://[", KnsBase + "/start", 1, 302},
	} {
		t.Run(test.location, func(t *testing.T) {
			var calls atomic.Int32
			live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
				if calls.Add(1) == 1 {
					writer.Header().Set("Location", test.location)
					writer.WriteHeader(302)
					return
				}
				io.WriteString(writer, "done")
			}, time.Time{})
			response, cancel, err := live.send(context.Background(), time.Now().Add(3*time.Second), "GET", KnsBase+"/start", nil, http.Header{})
			defer cancel()
			if err != nil {
				t.Fatal(err)
			}
			defer response.Body.Close()
			if response.StatusCode != test.status || response.Request.URL.String() != test.final || int(calls.Load()) != test.calls {
				t.Fatalf("status=%d final=%s calls=%d", response.StatusCode, response.Request.URL, calls.Load())
			}
		})
	}
}

func TestLiveDomesticConcurrentClonesReuseCaptchaGeneration(t *testing.T) {
	var challenges, fetches atomic.Int32
	arrived := make(chan struct{})
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
		switch request.URL.Path {
		case "/article":
			if request.URL.Query().Get("captchaId") == "shared-id" {
				io.WriteString(writer, minimalArticle)
				return
			}
			if challenges.Add(1) == 2 {
				close(arrived)
			}
			select {
			case <-arrived:
			case <-request.Context().Done():
				return
			}
			io.WriteString(writer, `{"message":"https://kns.cnki.net/verify/home?captchaId=shared-id"}`)
		case "/verify/home":
			fetches.Add(1)
		case "/verify-api/get":
			io.WriteString(writer, `{"originalImageBase64":"original","jigsawImageBase64":"jigsaw","secretKey":"0123456789abcdef","token":"token"}`)
		case "/verify-api/web/check":
			io.WriteString(writer, `{"success":true}`)
		}
	}, time.Now().Add(3*time.Second))
	live.state.config.CaptchaToken = textPointer("token")
	live.state.solver = func(time.Time) (jfbym.Solver, func(), error) { return jfbym.NewFixture(1, 0), func() {}, nil }
	clone := live.Clone()
	results := make(chan error, 2)
	for _, client := range []*LiveTransport{live, clone} {
		go func() { _, err := client.ArticleDetail(context.Background(), KnsBase+"/article", nil); results <- err }()
	}
	for range 2 {
		if err := <-results; err != nil {
			t.Fatal(err)
		}
	}
	if fetches.Load() != 1 || live.state.shared.generation != 1 || live.state.shared.session.RemainingBudget() != 4 {
		t.Fatal("concurrent clones solved the same generation twice")
	}
}

type bodyStartedTransport struct {
	base    http.RoundTripper
	started chan struct{}
}

func (wire bodyStartedTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	response, err := wire.base.RoundTrip(request)
	if err == nil {
		response.Body = &bodyStartedReader{ReadCloser: response.Body, started: wire.started}
	}
	return response, err
}

type bodyStartedReader struct {
	io.ReadCloser
	started chan struct{}
	once    sync.Once
}

func (body *bodyStartedReader) Read(destination []byte) (int, error) {
	body.once.Do(func() { close(body.started) })
	return body.ReadCloser.Read(destination)
}
func TestLiveDomesticBodyCancellationIsBounded(t *testing.T) {
	started := make(chan struct{})
	live := loopbackLive(t, func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Length", "100")
		writer.WriteHeader(200)
		writer.(http.Flusher).Flush()
		<-request.Context().Done()
	}, time.Time{})
	live.state.client.Transport = bodyStartedTransport{base: live.state.client.Transport, started: started}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { _, err := live.ArticleDetail(ctx, KnsBase+"/article", nil); done <- err }()
	<-started
	cancel()
	select {
	case err := <-done:
		if err == nil || err.Error() != "domestic CNKI response read failed" {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("body cancellation ignored")
	}
	if len(live.Attempts()) != 1 {
		t.Fatal(live.Attempts())
	}
}

// assertCloneCooldown verifies shared throttling survives clone and reset operations.
func assertCloneCooldown(t *testing.T, live, clone *LiveTransport, requests *atomic.Int32) {
	t.Helper()
	_, err := clone.ArticleDetail(context.Background(), KnsBase+"/a", nil)
	if err == nil || err.Error() != "article access deadline expired" {
		t.Fatal(err)
	}
	if requests.Load() != 1 || len(clone.Attempts()) != 0 {
		t.Fatal("shared cooldown admitted a new request")
	}
	if err := live.ResetTransientState(context.Background()); err != nil {
		t.Fatal(err)
	}
	_, err = live.ArticleDetail(context.Background(), KnsBase+"/a", nil)
	if err == nil || requests.Load() != 1 {
		t.Fatal("reset cleared cooldown")
	}
}

// assertCaptchaAttemptRedaction checks retry history without exposing verification secrets.
func assertCaptchaAttemptRedaction(t *testing.T, live *LiveTransport) {
	t.Helper()
	attempts := live.Attempts()
	if len(attempts) != 2 || attempts[0].DidSucceed || !attempts[1].DidSucceed || !attempts[1].DidRetry {
		t.Fatal(attempts)
	}
	for _, attempt := range attempts {
		if strings.Contains(attempt.Url, "solved-private-id") || strings.Contains(attempt.Url, "private-ident") {
			t.Fatal(attempt)
		}
	}
}

// serveCaptchaJson checks the original puzzle and check payload contracts.
func serveCaptchaJson(t *testing.T, writer http.ResponseWriter, request *http.Request) {
	t.Helper()
	assertCaptchaJsonHeaders(t, request)
	var body map[string]any
	if json.NewDecoder(request.Body).Decode(&body) != nil {
		t.Error("invalid JSON")
	}
	if request.URL.Path == "/verify-api/get" {
		if body["captchaId"] != "solved-private-id" || body["ident"] != "private-ident" {
			t.Error(body)
		}
		json.NewEncoder(writer).Encode(map[string]any{"data": map[string]any{"originalImageBase64": "original", "jigsawImageBase64": "jigsaw", "secretKey": "0123456789abcdef", "token": "private-token"}})
	} else {
		if _, exists := body["captchaId"]; exists {
			t.Error("check leaked captchaId")
		}
		if len(body) != 5 || body["token"] != "private-token" {
			t.Error(body)
		}
		io.WriteString(writer, `{"success":true}`)
	}
}

// assertCaptchaJsonHeaders preserves AJAX headers and the absent recognition user agent.
func assertCaptchaJsonHeaders(t *testing.T, request *http.Request) {
	t.Helper()
	if request.Header.Get("Origin") != KnsBase || request.Header.Get("X-Requested-With") != "XMLHttpRequest" || request.Header.Get("Content-Type") != "application/json;charset=UTF-8" {
		t.Error(request.Header)
	}
	if request.Header.Get("User-Agent") != "" {
		t.Errorf("unexpected captcha JSON UA %q", request.Header.Get("User-Agent"))
	}
}
