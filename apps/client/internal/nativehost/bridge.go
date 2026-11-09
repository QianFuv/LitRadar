package nativehost

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"
	"unicode"
	"unicode/utf8"
)

const (
	nativeLoginPath = "/api/auth/native/login"
	logoutPath      = "/api/auth/logout"

	// maxResponseBytes bounds one buffered response body.
	// ponytail: whole bodies are buffered for the JS bridge; stream if exports outgrow this.
	maxResponseBytes = 64 << 20
	// maxDeviceNameRunes matches the server limit for native login device names.
	maxDeviceNameRunes = 93
	// revokeOnForgetTimeout bounds the best-effort token revocation when the
	// server is switched.
	revokeOnForgetTimeout = 5 * time.Second
	userAgent             = "LitRadar-Client"
	fallbackDeviceName    = "LitRadar client"
)

var allowedMethods = map[string]bool{
	http.MethodGet:    true,
	http.MethodHead:   true,
	http.MethodPost:   true,
	http.MethodPut:    true,
	http.MethodPatch:  true,
	http.MethodDelete: true,
}

// forwardedRequestHeaders lists the frontend headers sent to the server. Others,
// including Authorization and Cookie, are dropped.
var forwardedRequestHeaders = []string{"Accept", "Content-Type", "If-None-Match"}

// exposedResponseHeaders lists the server headers the frontend client reads.
var exposedResponseHeaders = []string{"Content-Disposition", "Content-Type", "Retry-After", "X-Request-Id"}

// Request is one HTTP request issued by the frontend through [Bridge.Send].
type Request struct {
	// Method is the HTTP method.
	Method string `json:"method"`
	// URL is the absolute request URL on the configured server.
	URL string `json:"url"`
	// Headers holds request headers; only a fixed allowlist is forwarded.
	Headers map[string]string `json:"headers"`
	// Body is the text request body; empty means no body.
	Body string `json:"body"`
}

// Response is the server response returned to the frontend.
type Response struct {
	// Status is the HTTP status code.
	Status int `json:"status"`
	// Headers holds the response headers the frontend client reads.
	Headers map[string]string `json:"headers"`
	// Body is the response body; Wails encodes it as base64.
	Body []byte `json:"body"`
}

// Options configures a [Bridge] with the platform facilities it depends on.
type Options struct {
	// Secrets stores bearer tokens per server origin.
	Secrets SecretStore
	// ConfigDirectory returns the directory of the host configuration file. It is
	// resolved on first use because mobile hosts learn it after startup.
	ConfigDirectory func() (string, error)
	// DeviceName labels the personal access token created by native login.
	DeviceName string
	// Client performs server requests; nil selects a client that never follows redirects.
	Client *http.Client
	// SaveFile asks the user where to save a download and writes it, reporting
	// false when the user cancels; nil when the platform saves downloads elsewhere.
	SaveFile func(ctx context.Context, filename string, data []byte) (bool, error)
	// OpenURL opens an external URL in the system browser.
	OpenURL func(rawURL string) error
	// OidcCallbacks receives single sign-on callbacks; nil disables native single sign-on.
	OidcCallbacks OidcCallbackSource
	// FocusApp brings the app back to the foreground after the browser step of
	// single sign-on; nil when the platform returns to the app by itself.
	FocusApp func()
}

// Bridge is the Wails service that owns server traffic for the native hosts.
// Every exported method is callable from the frontend.
type Bridge struct {
	options    Options
	client     *http.Client
	deviceName string

	mutex          sync.Mutex
	config         *ConfigStore
	origin         string
	isOriginLoaded bool
}

// NewBridge returns a bridge configured by options.
func NewBridge(options Options) *Bridge {
	client := options.Client
	if client == nil {
		client = newHTTPClient()
	}
	return &Bridge{options: options, client: client, deviceName: sanitizeDeviceName(options.DeviceName)}
}

// newHTTPClient returns the default server client. Request deadlines come from
// the frontend through the call context; the transport only bounds connection setup.
func newHTTPClient() *http.Client {
	dialer := &net.Dialer{Timeout: 15 * time.Second, KeepAlive: 30 * time.Second}
	return &http.Client{
		Transport: &http.Transport{
			Proxy:               http.ProxyFromEnvironment,
			DialContext:         dialer.DialContext,
			ForceAttemptHTTP2:   true,
			TLSHandshakeTimeout: 15 * time.Second,
			MaxIdleConnsPerHost: 4,
			IdleConnTimeout:     90 * time.Second,
		},
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
}

// ServerOrigin returns the configured server origin, or "" before setup.
func (bridge *Bridge) ServerOrigin() (string, error) {
	bridge.mutex.Lock()
	defer bridge.mutex.Unlock()
	return bridge.loadOriginLocked()
}

// ConfigureServer validates rawOrigin, probes its readiness endpoint, and stores
// it as the server origin. It returns the canonical origin.
func (bridge *Bridge) ConfigureServer(ctx context.Context, rawOrigin string) (string, error) {
	origin, err := NormalizeServerOrigin(rawOrigin)
	if err != nil {
		return "", err
	}
	if err := bridge.probeReadiness(ctx, origin); err != nil {
		return "", err
	}
	bridge.mutex.Lock()
	defer bridge.mutex.Unlock()
	config, err := bridge.configStoreLocked()
	if err != nil {
		return "", err
	}
	if err := config.Save(origin); err != nil {
		return "", err
	}
	bridge.origin = origin
	bridge.isOriginLoaded = true
	return origin, nil
}

// ForgetServer revokes and deletes the token of the configured server on a best
// effort basis, then clears the stored origin so the host returns to setup.
func (bridge *Bridge) ForgetServer(ctx context.Context) error {
	origin, err := bridge.ServerOrigin()
	if err != nil || origin == "" {
		return err
	}
	token, hasToken, err := bridge.options.Secrets.Get(tokenKey(origin))
	if err == nil && hasToken {
		revokeContext, cancel := context.WithTimeout(ctx, revokeOnForgetTimeout)
		_, _ = bridge.do(revokeContext, http.MethodPost, origin+logoutPath, nil, "", token)
		cancel()
	}
	if err := bridge.options.Secrets.Delete(tokenKey(origin)); err != nil {
		return newBridgeError(CodeStorageFailure, "delete token: "+err.Error())
	}
	bridge.mutex.Lock()
	defer bridge.mutex.Unlock()
	config, err := bridge.configStoreLocked()
	if err != nil {
		return err
	}
	if err := config.Clear(); err != nil {
		return err
	}
	bridge.origin = ""
	return nil
}

// Send performs one frontend request against the configured server with the
// stored bearer token.
func (bridge *Bridge) Send(ctx context.Context, request Request) (Response, error) {
	origin, err := bridge.requireOrigin()
	if err != nil {
		return Response{}, err
	}
	method := strings.ToUpper(request.Method)
	if method == "" {
		method = http.MethodGet
	}
	if !allowedMethods[method] {
		return Response{}, newBridgeError(CodeInvalidRequest, "unsupported request method")
	}
	target, err := resolveServerURL(origin, request.URL)
	if err != nil {
		return Response{}, err
	}
	token, err := bridge.storedToken(origin)
	if err != nil {
		return Response{}, err
	}
	return bridge.do(ctx, method, target.String(), request.Headers, request.Body, token)
}

// Login signs in with a username or email and password through the native login
// endpoint and stores the issued token. The returned body omits the token.
func (bridge *Bridge) Login(ctx context.Context, username, password string) (Response, error) {
	origin, err := bridge.requireOrigin()
	if err != nil {
		return Response{}, err
	}
	payload, err := json.Marshal(map[string]string{
		"username":    username,
		"password":    password,
		"device_name": bridge.deviceName,
	})
	if err != nil {
		return Response{}, newBridgeError(CodeInvalidRequest, "encode login request")
	}
	response, err := bridge.do(ctx, http.MethodPost, origin+nativeLoginPath,
		map[string]string{"Content-Type": "application/json"}, string(payload), "")
	if err != nil || !isSuccess(response.Status) {
		return response, err
	}
	return bridge.adoptLoginResponse(origin, response)
}

// adoptLoginResponse stores the token of a successful native login response
// for origin and returns the response with the token removed from its body.
func (bridge *Bridge) adoptLoginResponse(origin string, response Response) (Response, error) {
	var login struct {
		Token     string          `json:"token"`
		ExpiresAt json.RawMessage `json:"expires_at"`
		User      json.RawMessage `json:"user"`
	}
	if json.Unmarshal(response.Body, &login) != nil || login.Token == "" {
		return Response{}, newBridgeError(CodeInvalidRequest, "server returned a malformed login response")
	}
	if err := bridge.options.Secrets.Set(tokenKey(origin), login.Token); err != nil {
		return Response{}, newBridgeError(CodeStorageFailure, "store token: "+err.Error())
	}
	body, err := json.Marshal(map[string]json.RawMessage{"expires_at": login.ExpiresAt, "user": login.User})
	if err != nil {
		return Response{}, newBridgeError(CodeInvalidRequest, "encode login response")
	}
	response.Body = body
	return response, nil
}

// Logout revokes the stored token on the server and deletes it locally even
// when the server cannot be reached. The response reports whether revocation
// was confirmed.
func (bridge *Bridge) Logout(ctx context.Context) (Response, error) {
	origin, err := bridge.requireOrigin()
	if err != nil {
		return Response{}, err
	}
	token, err := bridge.storedToken(origin)
	if err != nil {
		return Response{}, err
	}
	if token == "" {
		return Response{
			Status:  http.StatusOK,
			Headers: map[string]string{"Content-Type": "application/json"},
			Body:    []byte(`{"ok":true}`),
		}, nil
	}
	response, requestErr := bridge.do(ctx, http.MethodPost, origin+logoutPath, nil, "", token)
	if err := bridge.options.Secrets.Delete(tokenKey(origin)); err != nil {
		return Response{}, newBridgeError(CodeStorageFailure, "delete token: "+err.Error())
	}
	return response, requestErr
}

// ForgetToken deletes the stored token of the configured server without
// contacting it, after the server rejected the token.
func (bridge *Bridge) ForgetToken() error {
	origin, err := bridge.ServerOrigin()
	if err != nil || origin == "" {
		return err
	}
	if err := bridge.options.Secrets.Delete(tokenKey(origin)); err != nil {
		return newBridgeError(CodeStorageFailure, "delete token: "+err.Error())
	}
	return nil
}

// SaveDownload asks the user where to save a downloaded file and writes it. It
// returns false when the user cancels the dialog.
func (bridge *Bridge) SaveDownload(ctx context.Context, filename string, data []byte) (bool, error) {
	if bridge.options.SaveFile == nil {
		return false, newBridgeError(CodeUnsupported, "this platform saves downloads in the frontend host")
	}
	name := strings.TrimSpace(filename)
	if index := strings.LastIndexAny(name, `/\`); index >= 0 {
		name = name[index+1:]
	}
	if name == "" || name == "." || name == ".." {
		name = "download"
	}
	return bridge.options.SaveFile(ctx, name, data)
}

// OpenExternal opens an http or https URL in the system browser.
func (bridge *Bridge) OpenExternal(rawURL string) error {
	parsed, err := url.Parse(rawURL)
	if err != nil || (parsed.Scheme != "https" && parsed.Scheme != "http") || parsed.Host == "" {
		return newBridgeError(CodeInvalidRequest, "only http and https links can be opened")
	}
	if bridge.options.OpenURL == nil {
		return newBridgeError(CodeUnsupported, "opening links is not supported")
	}
	return bridge.options.OpenURL(parsed.String())
}

// probeReadiness confirms that origin answers the LitRadar readiness endpoint.
func (bridge *Bridge) probeReadiness(ctx context.Context, origin string) error {
	response, err := bridge.do(ctx, http.MethodGet, origin+readinessPath, nil, "", "")
	if err != nil {
		return err
	}
	var readiness struct {
		Status string `json:"status"`
	}
	if json.Unmarshal(response.Body, &readiness) != nil || readiness.Status == "" {
		return newBridgeError(CodeNotLitRadarServer, "readiness endpoint did not answer like a LitRadar server")
	}
	if response.Status != http.StatusOK || readiness.Status != "ok" {
		return newBridgeError(CodeServerNotReady, "server reported status "+readiness.Status)
	}
	return nil
}

// do sends one request and buffers the response. A non-empty token is sent as
// the bearer credential.
func (bridge *Bridge) do(ctx context.Context, method, target string, headers map[string]string, body, token string) (Response, error) {
	var bodyReader io.Reader
	if body != "" {
		bodyReader = strings.NewReader(body)
	}
	request, err := http.NewRequestWithContext(ctx, method, target, bodyReader)
	if err != nil {
		return Response{}, newBridgeError(CodeInvalidRequest, "build request")
	}
	for _, name := range forwardedRequestHeaders {
		if value, ok := lookupHeader(headers, name); ok {
			request.Header.Set(name, value)
		}
	}
	request.Header.Set("User-Agent", userAgent)
	if token != "" {
		request.Header.Set("Authorization", "Bearer "+token)
	}
	response, err := bridge.client.Do(request)
	if err != nil {
		if ctx.Err() != nil {
			return Response{}, ctx.Err()
		}
		return Response{}, newBridgeError(CodeServerUnreachable, describeTransportError(err))
	}
	defer response.Body.Close()
	content, err := io.ReadAll(io.LimitReader(response.Body, maxResponseBytes+1))
	if err != nil {
		if ctx.Err() != nil {
			return Response{}, ctx.Err()
		}
		return Response{}, newBridgeError(CodeServerUnreachable, "read response: "+err.Error())
	}
	if len(content) > maxResponseBytes {
		return Response{}, newBridgeError(CodeResponseTooLarge, "response body exceeds the client limit")
	}
	exposed := make(map[string]string, len(exposedResponseHeaders))
	for _, name := range exposedResponseHeaders {
		if value := response.Header.Get(name); value != "" {
			exposed[name] = value
		}
	}
	return Response{Status: response.StatusCode, Headers: exposed, Body: bytes.Clone(content)}, nil
}

// describeTransportError strips the request URL from a client error.
func describeTransportError(err error) string {
	var urlError *url.Error
	if errors.As(err, &urlError) {
		return urlError.Err.Error()
	}
	return err.Error()
}

// lookupHeader finds a header by case-insensitive name.
func lookupHeader(headers map[string]string, name string) (string, bool) {
	for key, value := range headers {
		if strings.EqualFold(key, name) {
			return value, true
		}
	}
	return "", false
}

// requireOrigin returns the configured origin or a not-configured error.
func (bridge *Bridge) requireOrigin() (string, error) {
	origin, err := bridge.ServerOrigin()
	if err != nil {
		return "", err
	}
	if origin == "" {
		return "", newBridgeError(CodeServerNotConfigured, "no server is configured")
	}
	return origin, nil
}

// storedToken returns the token of origin, or "" when none is stored.
func (bridge *Bridge) storedToken(origin string) (string, error) {
	token, hasToken, err := bridge.options.Secrets.Get(tokenKey(origin))
	if err != nil {
		return "", newBridgeError(CodeStorageFailure, "read token: "+err.Error())
	}
	if !hasToken {
		return "", nil
	}
	return token, nil
}

// loadOriginLocked reads the stored origin once; the caller holds the mutex.
func (bridge *Bridge) loadOriginLocked() (string, error) {
	if bridge.isOriginLoaded {
		return bridge.origin, nil
	}
	config, err := bridge.configStoreLocked()
	if err != nil {
		return "", err
	}
	origin, err := config.Load()
	if err != nil {
		return "", err
	}
	bridge.origin = origin
	bridge.isOriginLoaded = true
	return origin, nil
}

// configStoreLocked resolves the config store on first use; the caller holds the mutex.
func (bridge *Bridge) configStoreLocked() (*ConfigStore, error) {
	if bridge.config != nil {
		return bridge.config, nil
	}
	directory, err := bridge.options.ConfigDirectory()
	if err != nil || directory == "" {
		return nil, newBridgeError(CodeStorageFailure, "config directory is unavailable")
	}
	bridge.config = NewConfigStore(directory)
	return bridge.config, nil
}

// isSuccess reports whether status is a 2xx status.
func isSuccess(status int) bool {
	return status >= 200 && status < 300
}

// sanitizeDeviceName removes control characters and bounds the device label to
// the server limit, falling back to a generic label.
func sanitizeDeviceName(name string) string {
	cleaned := strings.Map(func(character rune) rune {
		if unicode.IsControl(character) || character == utf8.RuneError {
			return ' '
		}
		return character
	}, name)
	cleaned = strings.Join(strings.Fields(cleaned), " ")
	if utf8.RuneCountInString(cleaned) > maxDeviceNameRunes {
		cleaned = strings.TrimSpace(string([]rune(cleaned)[:maxDeviceNameRunes]))
	}
	if cleaned == "" {
		return fallbackDeviceName
	}
	return cleaned
}
