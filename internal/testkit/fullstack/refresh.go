package fullstack

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/cfp"
	cfpstorage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

type fixtureTransport struct{ client *http.Client }

// Fetch retains URL admission, one deadline and response ownership for the local fixture transport.
func (transport fixtureTransport) Fetch(ctx context.Context, configuration cfp.SourceConfig, location string, deadline time.Time) (cfp.Document, error) {
	parsed, err := whatwg.NewParser().Parse(location)
	if location != cfpUrl || err != nil || !configuration.PermitsUrl(parsed) {
		return cfp.Document{}, cfp.ErrDisallowedUrl
	}
	ctx, cancel := context.WithDeadline(ctx, deadline)
	defer cancel()
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, location, nil)
	if err != nil {
		return cfp.Document{}, cfp.ErrRequest
	}
	response, err := transport.client.Do(request)
	if err != nil {
		return cfp.Document{}, cfp.ErrRequest
	}
	defer response.Body.Close()
	return fixtureDocument(response)
}

func refresh(ctx context.Context, root string) (any, error) {
	root, err := validateRoot(root)
	if err != nil {
		return nil, err
	}
	listener, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.ParseIP("127.0.0.1")})
	if err != nil {
		return nil, err
	}
	defer listener.Close()
	if err := listener.SetDeadline(time.Now().Add(5 * time.Second)); err != nil {
		return nil, err
	}
	proxy := &url.URL{Scheme: "http", Host: listener.Addr().String()}
	transport := &http.Transport{Proxy: http.ProxyURL(proxy)}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 3 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	repository, err := cfpstorage.Open(config.FromProjectRoot(root).AuthDbPath)
	if err != nil {
		return nil, err
	}
	defer repository.Close()
	server := make(chan error, 1)
	go func() { server <- serveCfp(listener) }()
	catalog := fixtureCatalog()
	configuration := cfp.SourceConfig{SourceKey: "journal:" + catalog.CatalogId, CatalogIds: []string{catalog.CatalogId}, JournalTitle: catalog.Title, DiscoveryUrl: cfpUrl, Adapter: cfp.ElsevierCalls, ConfigVersion: 1, AllowedUrls: []cfp.UrlRule{{Host: "cfp-fixture.example", PathPrefix: "/calls"}}, IdentityTexts: []string{catalog.Title}, EmptyStatements: []string{}, RetainsPreviousNotices: false}
	result, err := cfp.RefreshSource(ctx, repository, configuration, fixtureTransport{client}, time.Now().Add(5*time.Second))
	if serverError := <-server; serverError != nil {
		return nil, serverError
	}
	if err != nil {
		return nil, err
	}
	if result.Status != "success" {
		return nil, errors.New("CFP source fixture did not publish")
	}
	return map[string]any{"status": "cfp_updated", "notices": result.Notices}, nil
}

func serveCfp(listener *net.TCPListener) error {
	connection, err := listener.AcceptTCP()
	if err != nil {
		return err
	}
	defer connection.Close()
	if err := connection.SetReadDeadline(time.Now().Add(2 * time.Second)); err != nil {
		return err
	}
	line, err := bufio.NewReader(io.LimitReader(connection, 4096)).ReadString('\n')
	if err != nil || !strings.HasPrefix(line, "GET "+cfpUrl+" ") {
		return errors.New("unexpected CFP proxy request")
	}
	if err := connection.SetWriteDeadline(time.Now().Add(2 * time.Second)); err != nil {
		return err
	}
	const body = "<h1>Journal of Reproducible Literature</h1><h2>Call for papers</h2><h3>Updated original CFP after backend refresh</h3><p>New original research scope from the HTTP source.</p><p>Submission deadline: 31 December 2099</p><h4>Submission instructions</h4><p>Original manuscripts are welcome.</p>"
	_, err = fmt.Fprintf(connection, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s", len(body), body)
	return err
}

// fixtureDocument keeps HTTP status before reading and encoding failure before size admission.
func fixtureDocument(response *http.Response) (cfp.Document, error) {
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return cfp.Document{}, cfp.SourceError{Kind: "http_status", Status: response.StatusCode}
	}
	content, err := io.ReadAll(io.LimitReader(response.Body, 65537))
	if err != nil || !utf8.Valid(content) {
		return cfp.Document{}, cfp.ErrEncoding
	}
	if len(content) > 65536 {
		return cfp.Document{}, cfp.ErrTooLarge
	}
	return cfp.Document{FinalUrl: response.Request.URL.String(), Text: string(content), Format: "html"}, nil
}
