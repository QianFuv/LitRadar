package auth

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"net/url"
	"regexp"
	"strconv"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
)

var nativeOIDCLoopback = regexp.MustCompile(`^http://127\.0\.0\.1:([1-9][0-9]{3,4})/oidc/callback$`)
var nativeOIDCState = regexp.MustCompile(`^[A-Za-z0-9._~-]{16,128}$`)
var nativeOIDCVerifier = regexp.MustCompile(`^[A-Za-z0-9._~-]{43,128}$`)

// NativeOIDCParameters binds a native client's PKCE challenge and callback to signed browser state.
type NativeOIDCParameters struct {
	RedirectURI   string `json:"redirect_uri"`
	CodeChallenge string `json:"code_challenge"`
	State         string `json:"state"`
}

// Valid admits only the exact native callback allowlist and canonical S256 challenges.
func (parameters NativeOIDCParameters) Valid() bool {
	if parameters.RedirectURI != "litradar://oidc/callback" {
		match := nativeOIDCLoopback.FindStringSubmatch(parameters.RedirectURI)
		if match == nil {
			return false
		}
		port, err := strconv.Atoi(match[1])
		if err != nil || port < 1024 || port > 65535 {
			return false
		}
	}
	digest, err := base64.RawURLEncoding.DecodeString(parameters.CodeChallenge)
	return err == nil && len(digest) == sha256.Size && base64.RawURLEncoding.EncodeToString(digest) == parameters.CodeChallenge && nativeOIDCState.MatchString(parameters.State)
}

// Redirect returns a callback URL only after revalidating the signed native parameters.
func (parameters NativeOIDCParameters) Redirect(field, value string) string {
	if !parameters.Valid() {
		return ""
	}
	query := url.Values{field: {value}, "state": {parameters.State}}
	return parameters.RedirectURI + "?" + query.Encode()
}

type nativeOIDCGrant struct {
	authorization domain.Authorization
	parameters    NativeOIDCParameters
	expiresAt     float64
}

// StartNative retains the ordinary confidential-client PKCE, nonce and signed state cookie.
func (oidc *Oidc) StartNative(ctx context.Context, parameters NativeOIDCParameters) (string, string, error) {
	if !parameters.Valid() {
		return "", "", OidcError("invalid_request")
	}
	return oidc.start(ctx, "/", nil, &parameters)
}

// FinishNative resolves the same identity as web login without creating a browser session.
func (oidc *Oidc) FinishNative(ctx context.Context, state OidcState, code string, audit domain.AuditEvent) (string, domain.User, error) {
	if state.Native == nil || !state.Native.Valid() || state.UserId != 0 {
		return "", domain.User{}, OidcError("state")
	}
	authorization, err := oidc.finishIdentity(ctx, state, code, nil, audit)
	if err != nil {
		return "", domain.User{}, err
	}
	var random [32]byte
	if _, err := rand.Read(random[:]); err != nil {
		return "", domain.User{}, domain.ErrEntropy
	}
	code = base64.RawURLEncoding.EncodeToString(random[:])
	now := oidc.service.now()
	oidc.nativeMutex.Lock()
	defer oidc.nativeMutex.Unlock()
	// ponytail: single-instance memory store; use a shared atomic store for multiple server instances; restarts drop pending codes.
	if oidc.nativeCodes == nil {
		oidc.nativeCodes = make(map[string]nativeOIDCGrant)
	}
	for hash, grant := range oidc.nativeCodes {
		if grant.expiresAt <= now {
			delete(oidc.nativeCodes, hash)
		}
	}
	if len(oidc.nativeCodes) >= 4096 {
		return "", domain.User{}, OidcError("provider")
	}
	oidc.nativeCodes[HashToken(code)] = nativeOIDCGrant{authorization: authorization, parameters: *state.Native, expiresAt: now + 120}
	return code, authorization.User, nil
}

// ExchangeNative consumes a grant once and fences device issuance against current auth generation.
func (oidc *Oidc) ExchangeNative(ctx context.Context, code, verifier, redirectURI, deviceName string, audit domain.AuditEvent) (domain.LoginSession, error) {
	hash := HashToken(code)
	oidc.nativeMutex.Lock()
	grant, exists := oidc.nativeCodes[hash]
	delete(oidc.nativeCodes, hash)
	oidc.nativeMutex.Unlock()
	digest := sha256.Sum256([]byte(verifier))
	if !exists || len(code) != 43 || grant.expiresAt <= oidc.service.now() || !nativeOIDCVerifier.MatchString(verifier) || !MatchOidcState(grant.parameters.CodeChallenge, base64.RawURLEncoding.EncodeToString(digest[:])) || redirectURI != grant.parameters.RedirectURI {
		return domain.LoginSession{}, OidcError("invalid_grant")
	}
	credentials, err := oidc.service.repository.CredentialsById(ctx, grant.authorization.User.Id)
	if err != nil {
		return domain.LoginSession{}, err
	}
	if credentials == nil || credentials.TokenGeneration != grant.authorization.TokenGeneration {
		return domain.LoginSession{}, OidcError("invalid_grant")
	}
	grant.authorization.User = credentials.User
	audit.Action = "oidc_native_exchange"
	session, err := oidc.service.issueDeviceToken(ctx, grant.authorization, deviceName, audit)
	if err == domain.ErrCredentials {
		return domain.LoginSession{}, OidcError("invalid_grant")
	}
	return session, err
}
