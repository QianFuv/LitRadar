package nativehost

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/url"
	"regexp"
	"time"

	"golang.org/x/oauth2"
)

const (
	nativeOidcLoginPath    = "/api/auth/oidc/native/login"
	nativeOidcExchangePath = "/api/auth/oidc/native/exchange"

	// OidcCallbackPath is the path of every native sign-in callback URI.
	OidcCallbackPath = "/oidc/callback"
	// AppLinkOidcRedirectURI is the callback URI the Android activity receives.
	AppLinkOidcRedirectURI = "litradar://oidc/callback"

	// oidcExchangeTimeout bounds the code exchange after the browser step.
	oidcExchangeTimeout = 60 * time.Second
)

// oidcSignInTimeout bounds the browser step of one native sign-in; tests shorten it.
var oidcSignInTimeout = 5 * time.Minute

// oidcReasonPattern bounds the error code a callback may report to the frontend.
var oidcReasonPattern = regexp.MustCompile(`^[a-z_]{1,64}$`)

// oidcCodePattern bounds the one-time code a callback may carry.
var oidcCodePattern = regexp.MustCompile(`^[A-Za-z0-9_-]{16,256}$`)

// errOidcSuperseded ends a waiting sign-in when a newer one starts.
var errOidcSuperseded = errors.New("a newer sign-in replaced this one")

// OidcCallbackListener receives the browser redirect that completes one native sign-in.
type OidcCallbackListener interface {
	// RedirectURI returns the callback URI the server redirects the browser to.
	RedirectURI() string
	// Wait blocks until the callback carrying the expected state arrives or ctx ends.
	Wait(ctx context.Context) (url.Values, error)
	// Close stops receiving callbacks; it is safe to call more than once.
	Close()
}

// OidcCallbackSource starts callback listeners for native sign-ins.
type OidcCallbackSource interface {
	// Listen starts receiving the callback whose state parameter equals state.
	Listen(state string) (OidcCallbackListener, error)
	// ConsumeInterruption reports, once, whether a callback arrived for a
	// sign-in this process no longer knows, such as one started before the
	// operating system killed the app.
	ConsumeInterruption() bool
}

// oidcProof is the PKCE verifier, its S256 challenge, and the state of one sign-in.
type oidcProof struct {
	verifier  string
	challenge string
	state     string
}

// newOidcProof generates a fresh verifier, challenge, and state.
func newOidcProof() oidcProof {
	verifier := oauth2.GenerateVerifier()
	return oidcProof{
		verifier:  verifier,
		challenge: oauth2.S256ChallengeFromVerifier(verifier),
		state:     oauth2.GenerateVerifier(),
	}
}

// readOidcCallback returns the one-time code of a callback query, or the
// bounded failure reason the server or identity provider reported instead.
func readOidcCallback(query url.Values) (code, reason string) {
	if failures, ok := query["error"]; ok {
		if len(failures) == 1 && oidcReasonPattern.MatchString(failures[0]) {
			return "", failures[0]
		}
		return "", "provider"
	}
	codes := query["code"]
	if len(codes) != 1 || !oidcCodePattern.MatchString(codes[0]) {
		return "", "invalid_callback"
	}
	return codes[0], ""
}

// buildOidcLoginURL returns the server URL that starts a native sign-in in the browser.
func buildOidcLoginURL(origin, redirectURI string, proof oidcProof) string {
	query := url.Values{
		"redirect_uri":   {redirectURI},
		"code_challenge": {proof.challenge},
		"state":          {proof.state},
	}
	return origin + nativeOidcLoginPath + "?" + query.Encode()
}

// LoginWithOidc signs in through the server's single sign-on in the system
// browser and stores the issued token. The verifier, the one-time code, and
// the token stay in the host; the returned body omits the token. The browser
// step ends after five minutes or when the frontend cancels the call.
func (bridge *Bridge) LoginWithOidc(ctx context.Context) (Response, error) {
	origin, err := bridge.requireOrigin()
	if err != nil {
		return Response{}, err
	}
	callbacks := bridge.options.OidcCallbacks
	if callbacks == nil || bridge.options.OpenURL == nil {
		return Response{}, newBridgeError(CodeUnsupported, "single sign-on is not supported on this platform")
	}
	proof := newOidcProof()
	listener, err := callbacks.Listen(proof.state)
	if err != nil {
		return Response{}, newOidcError("callback", "start callback listener: "+err.Error())
	}
	defer listener.Close()
	redirectURI := listener.RedirectURI()
	if err := bridge.options.OpenURL(buildOidcLoginURL(origin, redirectURI, proof)); err != nil {
		return Response{}, newOidcError("browser", "open system browser: "+err.Error())
	}

	waitContext, cancelWait := context.WithTimeout(ctx, oidcSignInTimeout)
	query, err := listener.Wait(waitContext)
	cancelWait()
	if err != nil {
		if ctx.Err() != nil {
			return Response{}, ctx.Err()
		}
		if errors.Is(err, context.DeadlineExceeded) {
			return Response{}, newBridgeError(CodeOidcTimeout, "the browser sign-in did not finish in time")
		}
		return Response{}, newOidcError("superseded", err.Error())
	}
	listener.Close()
	if bridge.options.FocusApp != nil {
		bridge.options.FocusApp()
	}
	code, reason := readOidcCallback(query)
	if reason != "" {
		return Response{}, newOidcError(reason, "sign-in callback reported "+reason)
	}

	payload, err := json.Marshal(map[string]string{
		"code":          code,
		"code_verifier": proof.verifier,
		"redirect_uri":  redirectURI,
		"device_name":   bridge.deviceName,
	})
	if err != nil {
		return Response{}, newBridgeError(CodeInvalidRequest, "encode sign-in exchange")
	}
	exchangeContext, cancelExchange := context.WithTimeout(ctx, oidcExchangeTimeout)
	defer cancelExchange()
	response, err := bridge.do(exchangeContext, http.MethodPost, origin+nativeOidcExchangePath,
		map[string]string{"Content-Type": "application/json"}, string(payload), "")
	if err != nil || !isSuccess(response.Status) {
		return response, err
	}
	return bridge.adoptLoginResponse(origin, response)
}

// ConsumeOidcInterruption reports, once, whether a sign-in callback arrived
// after the app lost the sign-in it belonged to, so the frontend can ask the
// user to start again.
func (bridge *Bridge) ConsumeOidcInterruption() bool {
	if bridge.options.OidcCallbacks == nil {
		return false
	}
	return bridge.options.OidcCallbacks.ConsumeInterruption()
}

// newOidcError returns a sign-in failure whose reason the frontend maps to a message.
func newOidcError(reason, message string) *BridgeError {
	return &BridgeError{Code: CodeOidcError, Message: message, Reason: reason}
}
