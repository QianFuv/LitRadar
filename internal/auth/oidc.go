package auth

import (
	"context"
	"crypto/hmac"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"math"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
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

type oidcDiscovery struct {
	Issuer                string `json:"issuer"`
	AuthorizationEndpoint string `json:"authorization_endpoint"`
	TokenEndpoint         string `json:"token_endpoint"`
	UserinfoEndpoint      string `json:"userinfo_endpoint"`
}

// Oidc owns the bounded discovery cache and reuses the ordinary login session service.
type Oidc struct {
	service       *Service
	settings      *settings.Repository
	signer        oidcSigner
	isDevelopment bool
	client        *http.Client
	mutex         sync.Mutex
	cached        oidcDiscovery
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

func (oidc *Oidc) discovery(ctx context.Context, configuration oidcConfiguration) (oidcDiscovery, error) {
	oidc.mutex.Lock()
	defer oidc.mutex.Unlock()
	if oidc.cached.Issuer == configuration.issuer && time.Now().Before(oidc.cacheUntil) {
		return oidc.cached, nil
	}
	var document oidcDiscovery
	if err := oidc.getJson(ctx, strings.TrimRight(configuration.issuer, "/")+"/.well-known/openid-configuration", "", &document); err != nil {
		return document, OidcError("provider")
	}
	if document.Issuer != configuration.issuer {
		return document, OidcError("provider")
	}
	for _, endpoint := range []string{document.AuthorizationEndpoint, document.TokenEndpoint, document.UserinfoEndpoint} {
		if endpoint == "" || settings.ValidateOidcUrl(endpoint, oidc.isDevelopment) != nil {
			return document, OidcError("provider")
		}
	}
	oidc.cached, oidc.cacheUntil = document, time.Now().Add(5*time.Minute)
	return document, nil
}

func (oidc *Oidc) getJson(ctx context.Context, endpoint, bearer string, result any) error {
	request, err := http.NewRequestWithContext(ctx, "GET", endpoint, nil)
	if err != nil {
		return err
	}
	request.Header.Set("Accept", "application/json")
	if bearer != "" {
		request.Header.Set("Authorization", "Bearer "+bearer)
	}
	response, err := oidc.client.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return OidcError("provider")
	}
	data, err := io.ReadAll(io.LimitReader(response.Body, (1<<20)+1))
	if err != nil {
		return err
	}
	if len(data) > 1<<20 {
		return OidcError("provider")
	}
	return json.Unmarshal(data, result)
}

func oauthConfiguration(configuration oidcConfiguration, discovery oidcDiscovery) oauth2.Config {
	return oauth2.Config{ClientID: configuration.clientId, ClientSecret: configuration.clientSecret, RedirectURL: strings.TrimRight(configuration.baseUrl, "/") + "/api/auth/oidc/callback", Scopes: []string{"openid", "profile", "email"}, Endpoint: oauth2.Endpoint{AuthURL: discovery.AuthorizationEndpoint, TokenURL: discovery.TokenEndpoint, AuthStyle: oauth2.AuthStyleInHeader}}
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
	for _, value := range []*string{&state.State, &state.Nonce, &state.Verifier} {
		*value, err = randomHex(32)
		if err != nil {
			return "", "", err
		}
	}
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
	return oauth.AuthCodeURL(state.State, oauth2.S256ChallengeOption(state.Verifier), oauth2.SetAuthURLParam("nonce", state.Nonce)), cookie, nil
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
	token, err := oauth.Exchange(context.WithValue(ctx, oauth2.HTTPClient, oidc.client), code, oauth2.VerifierOption(state.Verifier))
	if err != nil {
		return domain.Authorization{}, OidcError("provider")
	}
	rawIdToken, ok := token.Extra("id_token").(string)
	if !ok {
		return domain.Authorization{}, OidcError("provider")
	}
	subject, err := validateIdToken(rawIdToken, configuration.issuer, configuration.clientId, state.Nonce, time.Now())
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
	if err := oidc.getJson(ctx, discovery.UserinfoEndpoint, token.AccessToken, &profile); err != nil || profile.Subject != subject {
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

func validateIdToken(token, issuer, clientId, nonce string, now time.Time) (string, error) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 || len(token) > 65536 {
		return "", OidcError("provider")
	}
	payload, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return "", OidcError("provider")
	}
	// ponytail: ID tokens arrive only over validated back-channel TLS; add JWKS verification if tokens ever arrive via the browser.
	var claims struct {
		Issuer          string          `json:"iss"`
		Subject         string          `json:"sub"`
		Audience        json.RawMessage `json:"aud"`
		AuthorizedParty string          `json:"azp"`
		ExpiresAt       float64         `json:"exp"`
		IssuedAt        float64         `json:"iat"`
		Nonce           string          `json:"nonce"`
	}
	if json.Unmarshal(payload, &claims) != nil {
		return "", OidcError("provider")
	}
	var audiences []string
	var audience string
	if json.Unmarshal(claims.Audience, &audience) == nil {
		audiences = []string{audience}
	} else if json.Unmarshal(claims.Audience, &audiences) != nil {
		return "", OidcError("provider")
	}
	hasAudience := false
	for _, audience := range audiences {
		if audience == clientId {
			hasAudience = true
		}
	}
	if claims.Issuer != issuer || claims.Subject == "" || len(claims.Subject) > 255 || !hasAudience || (len(audiences) > 1 && claims.AuthorizedParty != clientId) || (claims.AuthorizedParty != "" && claims.AuthorizedParty != clientId) || math.IsNaN(claims.ExpiresAt) || math.IsInf(claims.ExpiresAt, 0) || claims.ExpiresAt <= float64(now.Unix()) || claims.IssuedAt <= 0 || claims.IssuedAt > float64(now.Unix()+60) || claims.IssuedAt > claims.ExpiresAt {
		return "", OidcError("provider")
	}
	if !MatchOidcState(nonce, claims.Nonce) {
		return "", OidcError("state")
	}
	return claims.Subject, nil
}
