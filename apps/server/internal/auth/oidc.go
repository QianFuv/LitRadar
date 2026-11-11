package auth

import (
	"context"
	"crypto/hmac"
	"encoding/base64"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	openid "github.com/coreos/go-oidc/v3/oidc"
	"golang.org/x/oauth2"
)

// OidcError exposes only a bounded public failure code.
type OidcError string

func (failure OidcError) Error() string { return string(failure) }

// OidcState binds a short-lived browser flow to its nonce, verifier and optional local session.
type OidcState struct {
	State           string                `json:"state"`
	Nonce           string                `json:"nonce"`
	Verifier        string                `json:"verifier"`
	Next            string                `json:"next"`
	UserId          identity.Id           `json:"user_id,omitempty"`
	TokenGeneration int64                 `json:"token_generation,omitempty"`
	ExpiresAt       int64                 `json:"expires_at"`
	Configuration   string                `json:"configuration"`
	Native          *NativeOIDCParameters `json:"native,omitempty"`
}

func (state OidcState) String() string   { return "OidcState([REDACTED])" }
func (state OidcState) GoString() string { return state.String() }

// LogValue keeps flow credentials out of structured logging.
func (state OidcState) LogValue() slog.Value { return slog.StringValue(state.String()) }

type oidcSigner interface{ SignOidcState([]byte) ([]byte, error) }

type oidcConfiguration struct{ issuer, clientId, clientSecret, displayName, baseUrl string }

func (configuration oidcConfiguration) enabled() bool {
	return configuration.issuer != "" && configuration.clientId != "" && configuration.clientSecret != "" && configuration.baseUrl != ""
}
func (configuration oidcConfiguration) binding() string {
	return HashToken(configuration.issuer + "\x00" + configuration.clientId + "\x00" + configuration.baseUrl)
}

// Oidc owns the bounded discovery cache and reuses the ordinary login session service.
type Oidc struct {
	service       *Service
	settings      *settings.Repository
	signer        oidcSigner
	isDevelopment bool
	client        *http.Client
	mutex         sync.Mutex
	cached        *openid.Provider
	cachedIssuer  string
	cacheUntil    time.Time
	nativeMutex   sync.Mutex
	nativeCodes   map[string]nativeOIDCGrant
}

// NewOidc borrows runtime settings and a deployment-key signer without owning their lifecycle.
func NewOidc(service *Service, repository *settings.Repository, signer oidcSigner, isDevelopment bool) *Oidc {
	return &Oidc{service: service, settings: repository, signer: signer, isDevelopment: isDevelopment, client: &http.Client{Timeout: 10 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}}
}

func (oidc *Oidc) configuration(ctx context.Context) (oidcConfiguration, error) {
	values, err := oidc.settings.Load(ctx)
	if err != nil {
		return oidcConfiguration{}, OidcError("unavailable")
	}
	var configuration oidcConfiguration
	for _, value := range values {
		switch value.Field {
		case "oidc_issuer_url":
			configuration.issuer = value.Value
		case "oidc_client_id":
			configuration.clientId = value.Value
		case "oidc_client_secret":
			configuration.clientSecret = value.Value
		case "oidc_display_name":
			configuration.displayName = value.Value
		case "public_base_url":
			configuration.baseUrl = value.Value
		}
	}
	for _, endpoint := range []string{configuration.issuer, configuration.baseUrl} {
		if err := settings.ValidateOidcUrl(endpoint, oidc.isDevelopment); err != nil {
			return oidcConfiguration{}, OidcError("unavailable")
		}
	}
	return configuration, nil
}

// Providers exposes enablement and the administrator-supplied label without client credentials.
func (oidc *Oidc) Providers(ctx context.Context) (bool, string, error) {
	configuration, err := oidc.configuration(ctx)
	return configuration.enabled(), configuration.displayName, err
}

// oidcTransport bounds provider documents, token responses and JWKS before decoding.
type oidcTransport struct{ http.RoundTripper }

func (transport oidcTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	response, err := transport.RoundTripper.RoundTrip(request)
	if err == nil {
		response.Body = http.MaxBytesReader(nil, response.Body, 1<<20)
	}
	return response, err
}

func (oidc *Oidc) providerContext(ctx context.Context) context.Context {
	client := *oidc.client
	wire := client.Transport
	if wire == nil {
		wire = http.DefaultTransport
	}
	client.Transport = oidcTransport{wire}
	return openid.ClientContext(ctx, &client)
}

func (oidc *Oidc) discovery(ctx context.Context, configuration oidcConfiguration) (*openid.Provider, error) {
	oidc.mutex.Lock()
	defer oidc.mutex.Unlock()
	if oidc.cached != nil && oidc.cachedIssuer == configuration.issuer && time.Now().Before(oidc.cacheUntil) {
		return oidc.cached, nil
	}
	provider, err := openid.NewProvider(oidc.providerContext(ctx), configuration.issuer)
	if err != nil {
		return nil, OidcError("provider")
	}
	var document struct {
		JWKSURL string `json:"jwks_uri"`
	}
	if provider.Claims(&document) != nil {
		return nil, OidcError("provider")
	}
	for _, endpoint := range []string{provider.Endpoint().AuthURL, provider.Endpoint().TokenURL, provider.UserInfoEndpoint(), document.JWKSURL} {
		if endpoint == "" || settings.ValidateOidcUrl(endpoint, oidc.isDevelopment) != nil {
			return nil, OidcError("provider")
		}
	}
	oidc.cached, oidc.cachedIssuer, oidc.cacheUntil = provider, configuration.issuer, time.Now().Add(5*time.Minute)
	return provider, nil
}

func oauthConfiguration(configuration oidcConfiguration, provider *openid.Provider) oauth2.Config {
	endpoint := provider.Endpoint()
	endpoint.AuthStyle = oauth2.AuthStyleInHeader
	return oauth2.Config{ClientID: configuration.clientId, ClientSecret: configuration.clientSecret, RedirectURL: strings.TrimRight(configuration.baseUrl, "/") + "/api/auth/oidc/callback", Scopes: []string{openid.ScopeOpenID, "profile", "email"}, Endpoint: endpoint}
}

// SafeOidcNext accepts only an absolute path on the current site, defaulting unsafe values to root.
func SafeOidcNext(next string) string {
	if next == "" || len(next) > 2048 || strings.ContainsAny(next, "\\\r\n\t") {
		return "/"
	}
	parsed, err := url.Parse(next)
	if err != nil || parsed.IsAbs() || parsed.Host != "" || !strings.HasPrefix(parsed.Path, "/") || strings.HasPrefix(parsed.Path, "//") || strings.ContainsAny(parsed.Path, "\\\r\n\t") {
		return "/"
	}
	return next
}

// Start signs a ten-minute flow cookie and builds an authorization-code request with PKCE S256.
func (oidc *Oidc) Start(ctx context.Context, next string, link *domain.Authorization) (string, string, error) {
	return oidc.start(ctx, next, link, nil)
}

func (oidc *Oidc) start(ctx context.Context, next string, link *domain.Authorization, native *NativeOIDCParameters) (string, string, error) {
	configuration, err := oidc.configuration(ctx)
	if err != nil {
		return "", "", err
	}
	if !configuration.enabled() {
		return "", "", OidcError("unavailable")
	}
	discovery, err := oidc.discovery(ctx, configuration)
	if err != nil {
		return "", "", err
	}
	state := OidcState{Next: SafeOidcNext(next), ExpiresAt: time.Now().Add(10 * time.Minute).Unix(), Configuration: configuration.binding(), Native: native}
	for _, value := range []*string{&state.State, &state.Nonce} {
		*value, err = randomHex(32)
		if err != nil {
			return "", "", err
		}
	}
	state.Verifier = oauth2.GenerateVerifier()
	if link != nil {
		if link.User.AuthKind != "local" || link.User.IsOidcLinked {
			return "", "", domain.ErrAccountConflict
		}
		state.UserId, state.TokenGeneration, state.Next = link.User.Id, link.TokenGeneration, "/?oidc_link=linked"
	}
	data, err := json.Marshal(state)
	if err != nil {
		return "", "", err
	}
	signature, err := oidc.signer.SignOidcState(data)
	if err != nil {
		return "", "", err
	}
	cookie := base64.RawURLEncoding.EncodeToString(data) + "." + base64.RawURLEncoding.EncodeToString(signature)
	oauth := oauthConfiguration(configuration, discovery)
	return oauth.AuthCodeURL(state.State, oauth2.S256ChallengeOption(state.Verifier), openid.Nonce(state.Nonce)), cookie, nil
}

// ReadState authenticates flow state before exposing the captured linking identity.
func (oidc *Oidc) ReadState(encoded string) (*OidcState, error) {
	if len(encoded) > 4096 {
		return nil, OidcError("state")
	}
	payload, signature, ok := strings.Cut(encoded, ".")
	if !ok {
		return nil, OidcError("state")
	}
	data, err := base64.RawURLEncoding.DecodeString(payload)
	if err != nil {
		return nil, OidcError("state")
	}
	supplied, err := base64.RawURLEncoding.DecodeString(signature)
	if err != nil {
		return nil, OidcError("state")
	}
	expected, err := oidc.signer.SignOidcState(data)
	if err != nil || !hmac.Equal(expected, supplied) {
		return nil, OidcError("state")
	}
	var state OidcState
	if json.Unmarshal(data, &state) != nil || state.State == "" || state.Nonce == "" || state.Verifier == "" {
		return nil, OidcError("state")
	}
	if state.Native != nil && (!state.Native.Valid() || state.UserId != 0) {
		return nil, OidcError("state")
	}
	if time.Now().Unix() >= state.ExpiresAt || state.ExpiresAt > time.Now().Add(10*time.Minute).Unix()+5 {
		return &state, OidcError("state")
	}
	return &state, nil
}

// MatchOidcState compares the browser callback state without leaking a matching prefix.
func MatchOidcState(expected, supplied string) bool {
	return supplied != "" && hmac.Equal([]byte(expected), []byte(supplied))
}

// Finish exchanges a back-channel code, validates claims and resolves or links the external identity.
func (oidc *Oidc) Finish(ctx context.Context, state OidcState, code string, link *domain.Authorization, audit domain.AuditEvent) (domain.LoginSession, error) {
	if state.Native != nil {
		return domain.LoginSession{}, OidcError("state")
	}
	authorization, err := oidc.finishIdentity(ctx, state, code, link, audit)
	if err != nil {
		return domain.LoginSession{}, err
	}
	if link != nil {
		return domain.LoginSession{User: authorization.User}, nil
	}
	audit.Action = "oidc_login"
	return oidc.service.createLogin(ctx, authorization, audit)
}

func (oidc *Oidc) finishIdentity(ctx context.Context, state OidcState, code string, link *domain.Authorization, audit domain.AuditEvent) (domain.Authorization, error) {
	configuration, err := oidc.configuration(ctx)
	if err != nil {
		return domain.Authorization{}, err
	}
	if !configuration.enabled() {
		return domain.Authorization{}, OidcError("unavailable")
	}
	if configuration.binding() != state.Configuration || time.Now().Unix() >= state.ExpiresAt || code == "" {
		return domain.Authorization{}, OidcError("state")
	}
	if state.UserId != 0 && (link == nil || state.UserId != link.User.Id || state.TokenGeneration != link.TokenGeneration) {
		return domain.Authorization{}, OidcError("expired_session")
	}
	if state.UserId == 0 && link != nil {
		return domain.Authorization{}, OidcError("state")
	}
	discovery, err := oidc.discovery(ctx, configuration)
	if err != nil {
		return domain.Authorization{}, err
	}
	oauth := oauthConfiguration(configuration, discovery)
	token, err := oauth.Exchange(oidc.providerContext(ctx), code, oauth2.VerifierOption(state.Verifier))
	if err != nil {
		return domain.Authorization{}, OidcError("provider")
	}
	rawIdToken, ok := token.Extra("id_token").(string)
	if !ok {
		return domain.Authorization{}, OidcError("provider")
	}
	subject, err := validateIDToken(oidc.providerContext(ctx), discovery, rawIdToken, configuration.issuer, configuration.clientId, state.Nonce, time.Now())
	if err != nil {
		return domain.Authorization{}, err
	}
	var profile struct {
		Subject           string `json:"sub"`
		Email             string `json:"email"`
		EmailVerified     bool   `json:"email_verified"`
		PreferredUsername string `json:"preferred_username"`
	}
	if token.AccessToken == "" || !strings.EqualFold(token.Type(), "Bearer") {
		return domain.Authorization{}, OidcError("provider")
	}
	info, err := discovery.UserInfo(oidc.providerContext(ctx), oauth2.StaticTokenSource(token))
	if err != nil || info.Subject != subject || info.Claims(&profile) != nil || profile.Subject != subject {
		return domain.Authorization{}, OidcError("provider")
	}
	if !profile.EmailVerified {
		return domain.Authorization{}, OidcError("email_unverified")
	}
	email, err := NormalizeEmail(&profile.Email)
	if err != nil || email == nil {
		return domain.Authorization{}, OidcError("provider")
	}
	username := ""
	if ValidUsername(profile.PreferredUsername) {
		username = profile.PreferredUsername
	}
	authorization, err := oidc.service.repository.CompleteOidc(ctx, configuration.issuer, subject, *email, username, link, oidc.service.now(), audit)
	if err != nil {
		switch {
		case errors.Is(err, domain.ErrEmailExists):
			return domain.Authorization{}, OidcError("email_conflict")
		case errors.Is(err, domain.ErrAccountConflict):
			return domain.Authorization{}, OidcError("account_conflict")
		case errors.Is(err, domain.ErrExpiredSession):
			return domain.Authorization{}, OidcError("expired_session")
		default:
			return domain.Authorization{}, err
		}
	}
	return authorization, nil
}

// Unlink removes external identity access while retaining local password access.
func (oidc *Oidc) Unlink(ctx context.Context, authorization domain.Authorization, audit domain.AuditEvent) (domain.User, error) {
	audit.Action = "oidc_unlink"
	return oidc.service.repository.UnlinkOidc(ctx, authorization, oidc.service.now(), audit)
}

// validateIDToken delegates signatures and standard claims to go-oidc and retains flow-specific checks.
func validateIDToken(ctx context.Context, provider *openid.Provider, token, issuer, clientID, nonce string, now time.Time) (string, error) {
	if len(token) > 65536 {
		return "", OidcError("provider")
	}
	verified, err := provider.Verifier(&openid.Config{ClientID: clientID, Now: func() time.Time { return now }}).Verify(ctx, token)
	if err != nil {
		return "", OidcError("provider")
	}
	var claims struct {
		AuthorizedParty string `json:"azp"`
	}
	if verified.Claims(&claims) != nil || verified.Issuer != issuer || verified.Subject == "" || len(verified.Subject) > 255 || (len(verified.Audience) > 1 && claims.AuthorizedParty != clientID) || (claims.AuthorizedParty != "" && claims.AuthorizedParty != clientID) || !verified.Expiry.After(now) || verified.IssuedAt.Unix() <= 0 || verified.IssuedAt.After(now.Add(time.Minute)) || verified.IssuedAt.After(verified.Expiry) {
		return "", OidcError("provider")
	}
	if !MatchOidcState(nonce, verified.Nonce) {
		return "", OidcError("state")
	}
	return verified.Subject, nil
}
