package scholarly

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func loopbackLive(t *testing.T, handler http.HandlerFunc, keys int, workers int) (*LiveTransport, *httptest.Server) {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	config := ConfigFromValuePools(3, "", "", "fixture@example.test").WithScheduleEpoch(saturatedUint64(unixScheduleTime().millis()))
	for index := range keys {
		config.OpenAlexApiKeys = append(config.OpenAlexApiKeys, fmt.Sprintf("oa-secret-%d", index))
		config.SemanticScholarApiKeys = append(config.SemanticScholarApiKeys, fmt.Sprintf("s2-secret-%d", index))
	}
	live, err := NewLiveTransport(config, workers, transport.Proxy{})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(live.CloseIdleConnections)
	live.crossrefBase = server.URL + "/v1"
	live.openAlexBase = server.URL
	live.semanticBase = server.URL + "/graph/v1"
	for index := range live.semantic.Slots {
		live.semantic.Slots[index].Next = unixScheduleTime()
	}
	return live, server
}

func TestLiveSourceWireQueriesAndCursorTermination(t *testing.T) {
	var received []string
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		if request.UserAgent() != "LitRadar/0.1" {
			t.Error("missing source user agent")
		}
		if request.Header.Get("Accept") != "*/*" {
			t.Error("missing reqwest default Accept header")
		}
		received = append(received, request.URL.RequestURI())
		if strings.Contains(request.URL.Path, "journals") {
			io.WriteString(response, `{"message":{"total-results":0,"items":[],"next-cursor":null}}`)
		} else {
			response.Header().Set("x-ratelimit-remaining", "1000")
			io.WriteString(response, `{"results":[{"id":"W1"}],"meta":{"next_cursor":"stale"}}`)
		}
	}, 1, 1)
	client := NewClient(live, true)
	if _, err := client.FetchCrossrefPage(context.Background(), "1234-5678", CrossrefQuery{CreatedFrom: 0, CreatedUntil: 1}); err != nil {
		t.Fatal(err)
	}
	page, err := client.FetchOpenAlexWorksBySourcePage(context.Background(), " https://openalex.org/S1/// ", nil, nil)
	if err != nil || page.NextCursor != nil || len(page.Items) != 1 {
		t.Fatalf("%#v %v", page, err)
	}
	if len(received) != 2 || !strings.HasPrefix(received[0], "/v1/journals/1234-5678/works?rows=225&filter=type%3Ajournal-article%2Cfrom-created-date%3A1970-01-01T00%3A00%3A00%2Cuntil-created-date%3A1970-01-01T00%3A00%3A01&mailto=fixture%40example.test") || !strings.Contains(received[1], "filter=primary_location.source.id%3AS1%2Ctype%3Aarticle%7Cbook-chapter&per-page=200&cursor=*&sort=publication_date%3Adesc&select=") {
		t.Fatalf("wire queries: %q", received)
	}
	encoded, _ := json.Marshal(live.Attempts())
	if strings.Contains(string(encoded), "oa-secret") || strings.Contains(string(encoded), "fixture@example") {
		t.Fatal("attempt leaked credentials")
	}
}

func TestSemanticAuthenticationIsolationAndRequestBody(t *testing.T) {
	var keys []string
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		key := request.Header.Get("x-api-key")
		keys = append(keys, key)
		if request.Method != "POST" || request.Header.Get("Content-Type") != "application/json" || request.URL.Query().Get("fields") != SemanticScholarFields {
			t.Error("invalid S2 request")
		}
		body, _ := io.ReadAll(request.Body)
		if string(body) != `{"ids":["DOI:10.1/a"]}` {
			t.Errorf("wrong body %s", body)
		}
		if key == "s2-secret-0" {
			response.WriteHeader(401)
			io.WriteString(response, `{"error":"private upstream diagnostic"}`)
		} else {
			io.WriteString(response, `[{"externalIds":{"DOI":"10.1/a"}}]`)
		}
	}, 2, 1)
	client := NewClient(live, true)
	result, err := client.FetchSemanticScholarByDois(context.Background(), []string{"DOI:10.1/A"}, 100)
	if err != nil || result["10.1/a"] == nil || len(keys) != 2 || keys[0] != "s2-secret-0" || keys[1] != "s2-secret-1" {
		t.Fatalf("%#v %v keys=%q", result, err, keys)
	}
	if !live.semantic.Slots[0].IsDisabled || live.semantic.Slots[1].IsDisabled {
		t.Fatal("authentication health crossed key boundary")
	}
	attempts := live.Attempts()
	if len(attempts) != 2 || attempts[0].DidSucceed || !attempts[1].DidRetry || *attempts[0].Error != "http_status" {
		t.Fatalf("wrong attempts %#v", attempts)
	}
}

func TestSemanticInvalidJsonIgnoresRetryAfter(t *testing.T) {
	var calls atomic.Int32
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		if calls.Add(1) == 1 {
			response.Header().Set("Retry-After", "18446744073709551615")
			io.WriteString(response, "invalid JSON")
		} else {
			io.WriteString(response, "[]")
		}
	}, 2, 1)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, err := NewClient(live, true).FetchSemanticScholarByDois(ctx, []string{"10.1/a"}, 100)
	if err != nil || calls.Load() != 2 {
		t.Fatalf("invalid JSON incorrectly trusted Retry-After: %v calls=%d", err, calls.Load())
	}
	if attempts := live.Attempts(); *attempts[0].Error != "invalid_json" {
		t.Fatalf("wrong invalid JSON attempt: %#v", attempts[0])
	}
}

// TestLiveOversizedBodyDoesNotSwitchKeys allows scheduled admission before checking terminal rejection.
func TestLiveOversizedBodyDoesNotSwitchKeys(t *testing.T) {
	for _, service := range []string{OpenAlex, SemanticScholar, Crossref} {
		for _, status := range []int{200, 401, 429} {
			t.Run(fmt.Sprintf("%s/%d", service, status), func(t *testing.T) {
				var calls atomic.Int32
				live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
					calls.Add(1)
					response.Header().Set("Content-Length", "16777217")
					response.Header().Set("Retry-After", "18446744073709551615")
					response.WriteHeader(status)
				}, 2, 1)
				if service == SemanticScholar {
					for index := range live.semantic.Slots {
						live.semantic.Slots[index].Next = unixScheduleTime().subtract(milliseconds(1))
					}
				}
				ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
				defer cancel()
				var err error
				switch service {
				case OpenAlex:
					_, err = NewClient(live, true).FetchOpenAlexByDois(ctx, []string{"10.1/a"}, 10)
				case SemanticScholar:
					_, err = NewClient(live, true).FetchSemanticScholarByDois(ctx, []string{"10.1/a"}, 10)
				case Crossref:
					_, err = NewClient(live, true).FetchCrossrefPage(ctx, "X", CrossrefQuery{CreatedFrom: 0, CreatedUntil: 1})
				}
				if err == nil || calls.Load() != 1 || len(live.Attempts()) != 1 {
					t.Fatalf("oversize retried: %v calls=%d", err, calls.Load())
				}
				if service == SemanticScholar && status == 401 && live.semantic.Slots[0].IsDisabled {
					t.Fatal("oversized authentication body disabled key")
				}
			})
		}
	}
}

func TestLiveRejectsRedirectCredentialForwarding(t *testing.T) {
	var forwarded atomic.Int32
	destination := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { forwarded.Add(1) }))
	defer destination.Close()
	for _, service := range []string{OpenAlex, SemanticScholar, Crossref} {
		t.Run(service, func(t *testing.T) {
			live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
				response.Header().Set("Location", destination.URL)
				response.WriteHeader(307)
				io.WriteString(response, `{}`)
			}, 1, 1)
			var err error
			switch service {
			case OpenAlex:
				_, err = NewClient(live, true).FetchOpenAlexByDois(context.Background(), []string{"a"}, 1)
			case SemanticScholar:
				_, err = NewClient(live, true).FetchSemanticScholarByDois(context.Background(), []string{"a"}, 1)
			case Crossref:
				_, err = NewClient(live, true).FetchCrossrefPage(context.Background(), "X", CrossrefQuery{})
			}
			var failure *Error
			if !errors.As(err, &failure) || failure.Kind != "HttpStatus" || failure.StatusCode != 307 {
				t.Fatalf("redirect not preserved: %v", err)
			}
		})
	}
	if forwarded.Load() != 0 {
		t.Fatal("redirect received credentials or request body")
	}
}

func TestLiveMalformedRedirectRemainsTerminal(t *testing.T) {
	for _, service := range []string{OpenAlex, SemanticScholar, Crossref} {
		for _, location := range []string{"http://[", "/x%GG", "http://host:bad/", ""} {
			t.Run(service+"/"+location, func(t *testing.T) {
				var calls atomic.Int32
				live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
					calls.Add(1)
					response.Header().Set("Location", location)
					response.WriteHeader(302)
					io.WriteString(response, `{}`)
				}, 1, 1)
				ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
				defer cancel()
				_, err := live.Request(ctx, Request{Service: service, Endpoint: map[string]string{OpenAlex: "works", SemanticScholar: "paper_batch", Crossref: "journal_works"}[service], Issn: "X", Dois: []string{"10.1/a"}})
				var failure *Error
				if !errors.As(err, &failure) || failure.Kind != "HttpStatus" || failure.StatusCode != 302 || calls.Load() != 1 {
					t.Fatalf("redirect response changed into retry: %v calls=%d", err, calls.Load())
				}
			})
		}
	}
}

type sourcePanicTransport struct {
	delegate http.RoundTripper
	didPanic atomic.Bool
}

func (wire *sourcePanicTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	if wire.didPanic.CompareAndSwap(false, true) {
		panic("private upstream diagnostic")
	}
	return wire.delegate.RoundTrip(request)
}

func TestOpenAlexWorkerPanicIsJoinedAndReleasesAdmission(t *testing.T) {
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) { io.WriteString(response, `{"results":[]}`) }, 2, 2)
	live.client.Transport = &sourcePanicTransport{delegate: live.client.Transport}
	_, err := live.RequestOpenAlexDoiBatches(context.Background(), [][]string{{"10.1/a"}, {"10.1/b"}, {"10.1/c"}})
	var failure *Error
	if !errors.As(err, &failure) || failure.Message != "bounded OpenAlex worker failed" {
		t.Fatalf("panic not mapped safely: %v", err)
	}
	for _, slot := range live.openAlex.state.Slots {
		if slot.InFlight != 0 {
			t.Fatal("panic leaked quota reservation")
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if _, err := live.RequestOpenAlexDoiBatches(ctx, [][]string{{"10.1/d"}, {"10.1/e"}}); err != nil {
		t.Fatal("panic leaked admission", err)
	}
}

func TestOpenAlexPublishesThrottleBeforeBody(t *testing.T) {
	release := make(chan struct{})
	var releaseOnce sync.Once
	defer releaseOnce.Do(func() { close(release) })
	var calls atomic.Int32
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		calls.Add(1)
		response.Header().Set("Retry-After", "7")
		response.WriteHeader(429)
		response.(http.Flusher).Flush()
		<-release
		io.WriteString(response, `{"error":"slow rate limit"}`)
	}, 1, 1)
	remaining := uint64(1000)
	live.openAlex.state.Slots[0].Remaining = &remaining
	cloned := live.Clone()
	result := make(chan error, 1)
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		_, err := NewClient(live, true).FetchOpenAlexByDois(ctx, []string{"a"}, 1)
		result <- err
	}()
	limit := time.Now().Add(time.Second)
	hasCooldown := false
	for time.Now().Before(limit) {
		live.openAlex.mutex.Lock()
		hasCooldown = live.openAlex.state.Slots[0].Cooldown != nil
		live.openAlex.mutex.Unlock()
		if hasCooldown {
			break
		}
		time.Sleep(time.Millisecond)
	}
	if !hasCooldown {
		releaseOnce.Do(func() { close(release) })
		t.Fatal("429 cooldown waits for response body")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	_, err := NewClient(cloned, true).FetchOpenAlexByDois(ctx, []string{"b"}, 1)
	if err == nil || calls.Load() != 1 {
		t.Fatalf("shared scheduler admitted throttled key: %v calls=%d", err, calls.Load())
	}
	releaseOnce.Do(func() { close(release) })
	select {
	case err := <-result:
		var failure *Error
		if !errors.As(err, &failure) || failure.StatusCode != 429 {
			t.Fatalf("last upstream error lost: %v", err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("source request did not finish")
	}
	live.openAlex.mutex.Lock()
	defer live.openAlex.mutex.Unlock()
	if live.openAlex.state.Slots[0].InFlight != 0 {
		t.Fatal("reservation leaked")
	}
}

func TestOpenAlexConcurrentBatchesReturnInputOrder(t *testing.T) {
	var arrived atomic.Int32
	all := make(chan struct{})
	var once sync.Once
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		if arrived.Add(1) == 3 {
			once.Do(func() { close(all) })
		}
		select {
		case <-all:
		case <-time.After(2 * time.Second):
			t.Error("batches did not overlap")
		}
		doi := strings.TrimPrefix(request.URL.Query().Get("filter"), "doi:")
		response.Header().Set("x-ratelimit-remaining", "1000")
		fmt.Fprintf(response, `{"results":[{"doi":%q}]}`, doi)
	}, 1, 3)
	remaining := uint64(1000)
	live.openAlex.state.Slots[0].Remaining = &remaining
	payloads, err := live.RequestOpenAlexDoiBatches(context.Background(), [][]string{{"first"}, {"second"}, {"third"}})
	if err != nil || len(payloads) != 3 {
		t.Fatalf("%#v %v", payloads, err)
	}
	for index, want := range []string{"first", "second", "third"} {
		if field(array(field(payloads[index], "results"))[0], "doi") != want {
			t.Fatal("batch completion order replaced input order")
		}
	}
	if arrived.Load() != 3 {
		t.Fatal("missing batch")
	}
}

func TestOpenAlexCancellationReleasesReservation(t *testing.T) {
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		t.Error("cancelled future reservation reached network")
	}, 1, 1)
	live.openAlex.state.Slots[0].Next = unixScheduleTime().add(seconds(20))
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	_, err := NewClient(live, true).FetchOpenAlexByDois(ctx, []string{"a"}, 1)
	if err == nil || live.openAlex.state.Slots[0].InFlight != 0 || len(live.Attempts()) != 0 {
		t.Fatalf("reservation leak: %v %#v", err, live.openAlex.state.Slots[0])
	}
}

func TestLiveConfigAndTransportFormattingRedactSecrets(t *testing.T) {
	live, _ := loopbackLive(t, func(http.ResponseWriter, *http.Request) {}, 2, 1)
	for _, value := range []any{live, *live, live.config, &live.config} {
		for _, format := range []string{"%v", "%+v", "%#v"} {
			text := fmt.Sprintf(format, value)
			if strings.Contains(text, "oa-secret") || strings.Contains(text, "s2-secret") || strings.Contains(text, "fixture@example") {
				t.Fatalf("credential-bearing diagnostic %s", text)
			}
		}
	}
}

func TestOpenAlexFailureStopsNewBatchAdmissions(t *testing.T) {
	var calls atomic.Int32
	both := make(chan struct{})
	secondRelease := make(chan struct{})
	var once sync.Once
	defer once.Do(func() { close(secondRelease) })
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		if calls.Add(1) == 2 {
			close(both)
		}
		select {
		case <-both:
		case <-time.After(2 * time.Second):
			t.Error("expected two admitted requests")
		}
		if request.URL.Query().Get("filter") == "doi:first" {
			response.WriteHeader(400)
			io.WriteString(response, `{"error":"stop"}`)
		} else {
			<-secondRelease
			response.Header().Set("x-ratelimit-remaining", "1000")
			io.WriteString(response, `{"results":[]}`)
		}
	}, 1, 2)
	remaining := uint64(1000)
	live.openAlex.state.Slots[0].Remaining = &remaining
	result := make(chan error, 1)
	go func() {
		_, err := live.RequestOpenAlexDoiBatches(context.Background(), [][]string{{"first"}, {"second"}, {"third"}, {"fourth"}})
		result <- err
	}()
	select {
	case <-both:
	case <-time.After(3 * time.Second):
		t.Fatal("requests not admitted")
	}
	limit := time.Now().Add(time.Second)
	for time.Now().Before(limit) {
		live.openAlex.mutex.Lock()
		pending := live.openAlex.state.Slots[0].InFlight
		live.openAlex.mutex.Unlock()
		if pending == 1 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	once.Do(func() { close(secondRelease) })
	select {
	case err := <-result:
		var failure *Error
		if !errors.As(err, &failure) || failure.StatusCode != 400 {
			t.Fatalf("first indexed error lost: %v", err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("admitted work not joined")
	}
	if calls.Load() != 2 || len(live.Attempts()) != 2 {
		t.Fatalf("new work admitted after failure: %d", calls.Load())
	}
}

func TestLiveCloneKeepsSemanticHealthIndependent(t *testing.T) {
	live, _ := loopbackLive(t, func(response http.ResponseWriter, request *http.Request) {
		response.WriteHeader(401)
		io.WriteString(response, `{}`)
	}, 1, 1)
	cloned := live.Clone()
	_, err := NewClient(live, true).FetchSemanticScholarByDois(context.Background(), []string{"a"}, 1)
	if err == nil || !live.semantic.Slots[0].IsDisabled || cloned.semantic.Slots[0].IsDisabled {
		t.Fatal("Semantic Scholar health was not copied independently")
	}
	if len(cloned.Attempts()) != 0 {
		t.Fatal("clone shares mutable attempts")
	}
	if live.openAlex != cloned.openAlex {
		t.Fatal("OpenAlex quota must remain shared")
	}
}

func TestLiveConfigCanonicalEmptyPools(t *testing.T) {
	encoded, err := json.Marshal(LiveConfig{})
	if err != nil {
		t.Fatal(err)
	}
	var values map[string]any
	if err := json.Unmarshal(encoded, &values); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"openalex_api_keys", "semantic_scholar_api_keys", "crossref_mailtos"} {
		items, ok := values[key].([]any)
		if !ok || len(items) != 0 {
			t.Fatalf("%s missing empty array: %s", key, encoded)
		}
	}
}
