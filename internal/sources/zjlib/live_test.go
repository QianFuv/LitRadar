package zjlib

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func loopbackLive(t *testing.T, handler http.Handler, config LiveConfig, deadline time.Time) (*LiveTransport, *httptest.Server) {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	allowed := endpoints{bases: [4]string{server.URL + "/www", server.URL + "/share", server.URL + "/login", server.URL + "/proxy"}, entry: server.URL + "/share/entry/area/35594/2120", referer: server.URL + "/proxy/kns55/"}
	live, err := newLiveTransport(config, transport.Proxy{}, deadline, allowed)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(live.Close)
	return live, server
}
func TestLiveFullLoginSearchAndDownload(t *testing.T) {
	var serverUrl string
	var mutex sync.Mutex
	paths := []string{}
	polls := 0
	live, server := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		mutex.Lock()
		defer mutex.Unlock()
		paths = append(paths, request.URL.Path)
		if request.Header.Get("User-Agent") != browserUserAgent || request.Header.Get("Accept-Language") != "zh-CN;q=0.9" {
			t.Error("browser headers missing")
		}
		switch request.URL.Path {
		case "/www/bff-api/reader-sso-service/portal-pc-api/login/zfb-qr":
			if request.Header.Get("Bff-Org-Id") != "1916318653650423810" {
				t.Error("organization header missing")
			}
			fmt.Fprint(writer, `{"success":true,"data":{"uuid":"qr","qrCode":"image","status":"WAITING_SCAN"}}`)
		case "/www/bff-api/reader-sso-service/portal-pc-api/qr/status":
			if request.URL.Query().Get("uuid") != "qr" {
				t.Error("QR query missing")
			}
			polls++
			if polls == 1 {
				fmt.Fprint(writer, `{"data":{"status":"WAITING_SCAN"}}`)
			} else {
				fmt.Fprint(writer, `{"data":{"status":"COMPLETE","data":"private-token"}}`)
			}
		case "/www/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl":
			if request.Header.Get("Bff-User-Token") != "private-token" || !strings.Contains(request.Header.Get("Cookie"), "userToken=private-token") {
				t.Error("login token did not reach SSO")
			}
			if request.URL.Query().Get("referURL") != serverUrl+"/share/entry/area/35594/2120" {
				t.Error("wrong SSO entry")
			}
			fmt.Fprintf(writer, `{"data":%q}`, serverUrl+"/share/protocolAuth")
		case "/share/protocolAuth":
			fmt.Fprintf(writer, `var sign='sync-sign';var url='/share/callback';var domainUrl='%s';var portalContextPath='/share/entry';sso-login/cookie/sync`, serverUrl)
		case "/share/entry/sso-login/cookie/sync":
			request.ParseForm()
			if request.PostForm.Get("sign") != "sync-sign" || request.PostForm.Get("url") != serverUrl+"/share/callback" || request.Header.Get("Origin") != serverUrl {
				t.Error("sync form/origin mismatch")
			}
			http.SetCookie(writer, &http.Cookie{Name: "share", Value: "yes", Path: "/"})
			http.Redirect(writer, request, "/share/synced", http.StatusFound)
		case "/share/synced":
			if request.Method != "GET" || request.Header.Get("Referer") != serverUrl+"/share/entry/sso-login/cookie/sync" {
				t.Error("redirect method/referer mismatch")
			}
		case "/share/entry/area/35594/2120":
			if !strings.Contains(request.Header.Get("Cookie"), "share=yes") {
				t.Error("share cookie missing")
			}
		case "/share/engine2/header/user-info":
			if request.Header.Get("X-Requested-With") != "XMLHttpRequest" || request.URL.Query().Get("t") == "" {
				t.Error("userinfo AJAX metadata missing")
			}
		case "/share/sso/api/auth/library/vpn358":
			if request.URL.Query().Get("wfwfid") != "2120" {
				t.Error("proxy library id missing")
			}
			writer.Header().Set("Location", serverUrl+"/login/index.php?enc=private")
			writer.WriteHeader(302)
		case "/login/index.php":
			http.SetCookie(writer, &http.Cookie{Name: "vpn358_sid", Value: "private-session", Path: "/"})
			http.Redirect(writer, request, "/proxy/kns55/", 302)
		case "/proxy/kns55/":
			if !strings.Contains(request.Header.Get("Cookie"), "vpn358_sid=private-session") {
				t.Error("proxy session missing")
			}
		case "/proxy/kns55/brief/result.aspx":
			request.ParseForm()
			if request.Method != "POST" || request.PostForm.Get("txt_1_value1") != "Study & Methods" || request.PostForm.Get("{key}_logical") != "and" || request.Header.Get("Origin") != serverUrl {
				t.Error("result form mismatch")
			}
		case "/proxy/kns55/request/SearchHandler.ashx":
			request.ParseForm()
			if request.PostForm.Get("txt_1_extension") != "xls" || request.PostForm.Get("__") == "" || request.Header.Get("X-Requested-With") != "XMLHttpRequest" {
				t.Error("handler form mismatch")
			}
		case "/proxy/kns55/brief/brief.aspx":
			if request.URL.Query().Get("pagename") != "ASP.brief_result_aspx" {
				t.Error("brief query mismatch")
			}
			fmt.Fprint(writer, `<tr><a href="/proxy/kns55/detail/detail.aspx?FileName=A">Study &amp; Methods</a><a href="/proxy/wrong-download.aspx">PDF</a></tr>`)
		case "/proxy/kns55/detail/detail.aspx":
			if !strings.Contains(request.Header.Get("Referer"), "brief.aspx?") {
				t.Error("last brief referer missing")
			}
			fmt.Fprint(writer, `<meta name="citation_title" content="Study &amp; Methods"><meta name="citation_author" content="Ada"><meta name="citation_journal_title" content="Journal"><a href="/proxy/download.aspx?dflag=pdfdown">PDF</a>`)
		case "/proxy/download.aspx":
			if !strings.Contains(request.Header.Get("Referer"), "detail.aspx?") {
				t.Error("detail referer missing")
			}
			writer.Header().Set("Content-Type", "application/octet-stream")
			fmt.Fprint(writer, "%PDF-1.4\nfixture")
		default:
			t.Errorf("unexpected network request %s", request.URL.Path)
			writer.WriteHeader(500)
		}
	}), DefaultLiveConfig(), time.Time{})
	serverUrl = server.URL
	client := NewClient(live)
	if _, err := client.StartQrLogin(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := client.PollQrLogin(context.Background(), 2, 0.1); err != nil {
		t.Fatal(err)
	}
	if _, err := client.WarmUpFulltextSession(context.Background()); err != nil {
		t.Fatal(err)
	}
	mutex.Lock()
	before := len(paths)
	mutex.Unlock()
	if _, err := client.WarmUpFulltextSession(context.Background()); err != nil {
		t.Fatal(err)
	}
	mutex.Lock()
	if len(paths) != before {
		t.Error("fresh session performed network warmup")
	}
	mutex.Unlock()
	pdf, err := client.DownloadMatchingPdf(context.Background(), ArticleIdentity{Title: "Study & Methods", Authors: "Ada", JournalTitle: "Journal"}, 10)
	if err != nil {
		t.Fatal(err)
	}
	if pdf.Filename != "Study & Methods.pdf" || string(pdf.Content) != "%PDF-1.4\nfixture" {
		t.Fatalf("wrong document: %s", pdf.Filename)
	}
	clone := live.Clone()
	if !clone.HasUnexpiredCookie("vpn358_sid", 0) {
		t.Error("clone lost shared cookie jar")
	}
	fresh := NewClient(NewFixtureTransport(Success))
	fresh.LoadStateData(client.StateData())
	if fresh.StateData()["bff_user_token"] != "private-token" {
		t.Error("network session could not be restored")
	}
}

func TestLiveQrResponseErrors(t *testing.T) {
	cases := []struct {
		name, body, kind, message string
		status                    int
	}{
		{"http", `secret`, "Request", "start QR login failed with HTTP 500:", 500},
		{"json", `broken`, "Parse", "start QR login returned non-JSON response.", 200},
		{"array", `[]`, "Parse", "start QR login returned non-object JSON response.", 200},
		{"failure", `{"success":false}`, "Request", "start QR login failed.", 200},
		{"data", `{"data":null}`, "Parse", "start QR login response did not contain object data.", 200},
		{"missing", `{"data":{"uuid":"x"}}`, "Parse", "QR login response did not contain uuid/qrCode.", 200},
		{"large", strings.Repeat("x", responseMaximumBytes+1), "Parse", "start QR login response exceeded the configured size limit.", 200},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			live, _ := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
				writer.WriteHeader(test.status)
				fmt.Fprint(writer, test.body)
			}), DefaultLiveConfig(), time.Time{})
			_, err := live.StartQrLogin(context.Background())
			if err == nil || err.(*Error).Kind != test.kind || !strings.HasPrefix(err.Error(), test.message) {
				t.Fatalf("unexpected error: %v", err)
			}
		})
	}
}
func TestLivePollTerminalAndTimeout(t *testing.T) {
	for _, status := range []string{"EXPIRED", "CANCEL", "CANCELED", "FAIL", "FAILED", "COMPLETE", "WAITING_SCAN"} {
		t.Run(status, func(t *testing.T) {
			live, _ := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
				fmt.Fprintf(writer, `{"data":{"status":%q}}`, status)
			}), DefaultLiveConfig(), time.Time{})
			_, err := live.PollQrLogin(context.Background(), "qr", 0, 0.1)
			if err == nil {
				t.Fatal("expected terminal error")
			}
			expected := "Request"
			if status == "COMPLETE" {
				expected = "Parse"
			}
			if status == "WAITING_SCAN" {
				expected = "Timeout"
			}
			if err.(*Error).Kind != expected {
				t.Fatalf("wrong classification %v", err)
			}
		})
	}
}
func TestLiveRejectsRedirectBeforeCrossFamilyTransmission(t *testing.T) {
	var requests atomic.Int32
	live, server := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		requests.Add(1)
		http.Redirect(writer, request, "/share/stolen", 302)
	}), DefaultLiveConfig(), time.Time{})
	_, err := live.DownloadPdf(context.Background(), server.URL+"/proxy/download.aspx?token=private", nil, nil)
	if err == nil || requests.Load() != 1 {
		t.Fatalf("redirect escaped family: requests=%d error=%v", requests.Load(), err)
	}
	if strings.Contains(err.Error(), "private") {
		t.Error("error leaked query")
	}
}
func TestLiveDocumentBoundsAndRecognition(t *testing.T) {
	for _, test := range []struct {
		name, body, contentType string
		limit                   int64
		success                 bool
	}{
		{"pdf-type", "not magic", "application/pdf", 10, true}, {"magic", "%PDF-x", "text/plain", 6, true}, {"empty-pdf", "", "application/pdf", 1, true}, {"not-pdf", "html", "text/html", 10, false}, {"oversize", "%PDF-xx", "application/pdf", 6, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			live, server := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
				writer.Header().Set("Content-Type", test.contentType)
				fmt.Fprint(writer, test.body)
			}), LiveConfig{TimeoutSeconds: 5, MaximumDocumentBytes: test.limit}, time.Time{})
			pdf, err := live.DownloadPdf(context.Background(), server.URL+"/proxy/download.aspx?filetitle=Named+Article&token=private", nil, nil)
			if (err == nil) != test.success {
				t.Fatalf("unexpected result %v", err)
			}
			if test.success && pdf.Filename != "Named Article.pdf" {
				t.Error("query filename lost")
			}
			if err != nil && strings.Contains(err.Error(), "private") {
				t.Error("query leaked")
			}
		})
	}
}
func TestLiveDeadlineAndCancellation(t *testing.T) {
	var requests atomic.Int32
	live, _ := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) { requests.Add(1) }), DefaultLiveConfig(), time.Now().Add(-time.Second))
	_, err := live.StartQrLogin(context.Background())
	if err == nil || requests.Load() != 0 || !strings.Contains(err.Error(), "deadline expired") {
		t.Fatalf("expired deadline performed work: %v", err)
	}
	started := make(chan struct{})
	blocking, _ := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) { close(started); <-request.Context().Done() }), DefaultLiveConfig(), time.Time{})
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { _, err := blocking.StartQrLogin(ctx); done <- err }()
	<-started
	cancel()
	select {
	case err := <-done:
		if err == nil {
			t.Error("canceled request succeeded")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("cancellation did not stop request")
	}
}

func TestLiveCookieRestoreUsesDomainRatherThanName(t *testing.T) {
	live, err := NewLiveTransport(DefaultLiveConfig(), transport.Proxy{}, time.Time{})
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	live.LoadCookies([]Cookie{{Name: "session", Value: "empty-domain", Path: "/", Secure: false}, {Name: "userToken", Value: "proxy-token", Domain: "http-10--18--17--173.elib.zyproxy.zjlib.cn", Path: "/", Secure: true}})
	for _, test := range []struct{ origin, name, value string }{{WwwBase, "session", "empty-domain"}, {ProxyBase, "userToken", "proxy-token"}} {
		location, _ := url.Parse(test.origin)
		found := false
		for _, cookie := range live.state.redirect.Jar.Cookies(location) {
			if cookie.Name == test.name && cookie.Value == test.value {
				found = true
			}
		}
		if !found {
			t.Errorf("cookie %s not restored at intended domain", test.name)
		}
	}
}
func TestLiveRedirectAndTruncatedPdfClassification(t *testing.T) {
	for _, test := range []struct{ name, expected string }{{"redirect", "error following redirect"}, {"truncated", "request or response body error"}} {
		t.Run(test.name, func(t *testing.T) {
			live, server := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				if test.name == "redirect" {
					http.Redirect(writer, request, "/share/no", 302)
					return
				}
				writer.Header().Set("Content-Type", "application/pdf")
				writer.Header().Set("Content-Length", "40")
				fmt.Fprint(writer, "%PDF-short")
			}), DefaultLiveConfig(), time.Time{})
			_, err := live.DownloadPdf(context.Background(), server.URL+"/proxy/download.aspx", nil, nil)
			if err == nil || err.Error() != test.expected {
				t.Fatalf("want %q, got %v", test.expected, err)
			}
		})
	}
}

func TestLiveInvalidUrlPrecedesExpiredSharedDeadline(t *testing.T) {
	live, err := NewLiveTransport(DefaultLiveConfig(), transport.Proxy{}, time.Now().Add(-time.Second))
	if err != nil {
		t.Fatal(err)
	}
	defer live.Close()
	_, err = live.DownloadPdf(context.Background(), "https://example.com/a.pdf", nil, nil)
	if err == nil || err.(*Error).Kind != "Parse" {
		t.Fatalf("URL validation lost priority: %v", err)
	}
}

func TestLiveProxyLoginRetriesOnlyRecognizedLoop(t *testing.T) {
	for _, mode := range []string{"recover", "exhaust", "self", "foreign", "missing-cookie", "wrong-host", "missing-location", "hop-limit"} {
		t.Run(mode, func(t *testing.T) {
			var attempts atomic.Int32
			var requests atomic.Int32
			live, _ := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				requests.Add(1)
				base := "http://" + request.Host
				switch request.URL.Path {
				case "/www/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl":
					fmt.Fprintf(writer, `{"data":%q}`, base+"/share/sso")
				case "/share/sso", "/share/entry/area/35594/2120", "/share/engine2/header/user-info":
				case "/share/sso/api/auth/library/vpn358":
					attempts.Add(1)
					http.Redirect(writer, request, "/login/index.php?enc=sensitive", 302)
				case "/login/index.php":
					switch mode {
					case "self":
						http.Redirect(writer, request, "/login/INDEX.php?changed=yes", 302)
					case "foreign":
						http.Redirect(writer, request, "https://example.com/private", 302)
					case "wrong-host":
						fmt.Fprint(writer, "login page")
					case "missing-location":
						writer.WriteHeader(302)
					case "hop-limit":
						http.Redirect(writer, request, "/login/hop1", 302)
					default:
						if mode == "recover" && attempts.Load() == 2 {
							http.SetCookie(writer, &http.Cookie{Name: "vpn358_sid", Value: "session", Path: "/"})
						}
						http.Redirect(writer, request, "/proxy/kns55/", 302)
					}
				case "/proxy/kns55/":
					if mode == "exhaust" || mode == "recover" && attempts.Load() == 1 {
						http.Redirect(writer, request, "/login/index.php?enc=rotated", 302)
					}
				case "/login/hop1":
					http.Redirect(writer, request, "/login/hop2", 302)
				case "/login/hop2":
					http.Redirect(writer, request, "/login/hop3", 302)
				case "/login/hop3":
					http.Redirect(writer, request, "/login/hop4", 302)
				case "/login/hop4":
					http.Redirect(writer, request, "/login/hop5", 302)
				default:
					t.Errorf("unexpected proxy request %s", request.URL.Path)
					writer.WriteHeader(500)
				}
			}), DefaultLiveConfig(), time.Time{})
			final, err := live.WarmUpFulltextSession(context.Background(), "token")
			if mode == "recover" {
				if err != nil || attempts.Load() != 2 || !strings.HasSuffix(final, "/proxy/kns55/") {
					t.Fatalf("loop did not recover: %d %v", attempts.Load(), err)
				}
				return
			}
			if err == nil {
				t.Fatal("invalid proxy flow succeeded")
			}
			expected := int32(1)
			if mode == "exhaust" {
				expected = 3
			}
			if attempts.Load() != expected {
				t.Fatalf("wrong retry count %d: %v", attempts.Load(), err)
			}
			messages := map[string]string{"exhaust": "not accepted after 3 login attempts", "self": "unexpected self-redirect", "foreign": "unexpected endpoint", "missing-cookie": "did not set vpn358_sid", "wrong-host": "ended outside the proxy host", "missing-location": "valid Location", "hop-limit": "exceeded 4 redirect hops"}
			if !strings.Contains(err.Error(), messages[mode]) {
				t.Fatalf("wrong failure: %v", err)
			}
		})
	}
}

func TestLiveSharedDeadlineStopsProxyRetrySleep(t *testing.T) {
	var attempts atomic.Int32
	live, _ := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		base := "http://" + request.Host
		switch request.URL.Path {
		case "/www/bff-api/portal-admin-service/open-api/build-and-share/ssoLoginUrl":
			fmt.Fprintf(writer, `{"data":%q}`, base+"/share/sso")
		case "/share/sso", "/share/entry/area/35594/2120", "/share/engine2/header/user-info":
		case "/share/sso/api/auth/library/vpn358":
			attempts.Add(1)
			http.Redirect(writer, request, "/login/index.php", 302)
		case "/login/index.php":
			http.Redirect(writer, request, "/proxy/kns55/", 302)
		case "/proxy/kns55/":
			http.Redirect(writer, request, "/login/index.php", 302)
		}
	}), DefaultLiveConfig(), time.Now().Add(150*time.Millisecond))
	_, err := live.WarmUpFulltextSession(context.Background(), "token")
	if err == nil || !strings.Contains(err.Error(), "deadline expired") || attempts.Load() > 1 {
		t.Fatalf("deadline retry boundary failed: %d %v", attempts.Load(), err)
	}
}

func TestLiveHttpRedirectNormalizesBeforeFamilyValidation(t *testing.T) {
	for _, test := range []struct {
		name, location string
		requests       int32
	}{{"fragment", "/proxy/next#fragment", 2}, {"empty", "", 2}, {"invalid", "https://[", 1}, {"backslash", `\proxy\next`, 2}} {
		t.Run(test.name, func(t *testing.T) {
			var count atomic.Int32
			live, server := loopbackLive(t, http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
				attempt := count.Add(1)
				writer.Header().Set("Content-Type", "application/pdf")
				if attempt == 1 {
					writer.Header()["Location"] = []string{test.location}
					writer.WriteHeader(302)
				}
				fmt.Fprint(writer, "%PDF")
			}), DefaultLiveConfig(), time.Time{})
			_, err := live.DownloadPdf(context.Background(), server.URL+"/proxy/start", nil, nil)
			if err != nil || count.Load() != test.requests {
				t.Fatalf("redirect normalization differs: count=%d error=%v", count.Load(), err)
			}
		})
	}
}
