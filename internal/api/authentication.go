package api

import (
	"context"
	"errors"
	"math"
	"net/http"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	"github.com/QianFuv/LitRadar/internal/auth"
	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/platform/mcpcompat"
)

// Authenticator verifies credentials for each request using the shared storage pool.
// Routes invoke it after their own extractors to preserve observable error ordering.
type Authenticator struct {
	service *auth.Service
	pool    *executor.Pool
}

// NewAuthenticator shares an already initialized service and its API storage executor.
func NewAuthenticator(service *auth.Service, pool *executor.Pool) *Authenticator {
	return &Authenticator{service, pool}
}

type currentIdentity struct {
	authorization domain.Authorization
	token         string
}

func (authenticator *Authenticator) requireUser(request *http.Request) (currentIdentity, *apiError) {
	token, failure := resolveAuthToken(request.Header)
	if failure != nil {
		return currentIdentity{}, failure
	}
	if token == "" {
		return currentIdentity{}, &apiError{status: 401, detail: "Authentication required"}
	}
	type outcome struct {
		authorization *domain.Authorization
		err           error
	}
	result, err := executor.Run(request.Context(), authenticator.pool, func() (outcome, error) {
		authorization, err := authenticator.service.VerifyToken(context.Background(), token)
		return outcome{authorization, err}, nil
	})
	if err != nil {
		return currentIdentity{}, mapExecutorError(err)
	}
	if result.err != nil {
		return currentIdentity{}, mapAuthError(result.err)
	}
	if result.authorization == nil {
		return currentIdentity{}, &apiError{status: 401, detail: "Invalid or expired token"}
	}
	return currentIdentity{*result.authorization, token}, nil
}

func (authenticator *Authenticator) requireAdmin(request *http.Request) (currentIdentity, *apiError) {
	identity, err := authenticator.requireUser(request)
	if err != nil {
		return currentIdentity{}, err
	}
	if !identity.authorization.User.IsAdmin {
		return currentIdentity{}, &apiError{status: 403, detail: "Admin access required"}
	}
	return identity, nil
}

// McpAuthorize supplies only the identity freshly validated for this HTTP request.
func (authenticator *Authenticator) McpAuthorize(writer http.ResponseWriter, request *http.Request) (mcpcompat.Principal, bool) {
	identity, err := authenticator.requireUser(request)
	if err != nil {
		err.write(writer)
		return mcpcompat.Principal{}, false
	}
	return mcpcompat.Principal{UserId: identity.authorization.User.Id}, true
}

func resolveAuthToken(headers http.Header) (string, *apiError) {
	if values := headers.Values("Authorization"); len(values) != 0 {
		value := values[0]
		scheme, token, hasSpace := strings.Cut(value, " ")
		if !validHeaderText(value) || !hasSpace || asciiLower(scheme) != "bearer" {
			return "", &apiError{status: 401, detail: "Invalid authorization format"}
		}
		if token = strings.TrimSpace(token); token != "" {
			return token, nil
		}
	}
	token, _ := sessionCookie(headers)
	return token, nil
}

func validHeaderText(value string) bool {
	for _, character := range []byte(value) {
		if (character < 32 && character != '\t') || character >= 127 {
			return false
		}
	}
	return true
}

func sessionCookie(headers http.Header) (string, bool) {
	values := headers.Values("Cookie")
	if len(values) == 0 || !validHeaderText(values[0]) {
		return "", false
	}
	for _, cookie := range strings.Split(values[0], ";") {
		name, value, hasEquals := strings.Cut(strings.TrimSpace(cookie), "=")
		if hasEquals && name == domain.SessionCookieName {
			return strings.TrimSpace(value), true
		}
	}
	return "", false
}

func sessionCookieHeader(token string, expires, now float64, isSecure bool) string {
	maxAge := math.Floor(math.Max(expires-now, 0))
	seconds := int64(0)
	if maxAge >= float64(math.MaxInt64) {
		seconds = math.MaxInt64
	} else if !math.IsNaN(maxAge) {
		seconds = int64(maxAge)
	}
	value := domain.SessionCookieName + "=" + token + "; Max-Age=" + strconv.FormatInt(seconds, 10) + "; Path=/; SameSite=lax; HttpOnly"
	if isSecure {
		value += "; Secure"
	}
	return value
}

func mapAuthError(err error) *apiError {
	switch {
	case errors.Is(err, domain.ErrCredentials):
		return &apiError{status: 401, detail: domain.ErrCredentials.Error()}
	case errors.Is(err, domain.ErrUsername), errors.Is(err, domain.ErrPasswordShort), errors.Is(err, domain.ErrTokenNameLength), errors.Is(err, domain.ErrTokenReservedName), errors.Is(err, domain.ErrTokenTtl), errors.Is(err, domain.ErrInviteRequired), errors.Is(err, domain.ErrBootstrapRequired), errors.Is(err, domain.ErrInvite), errors.Is(err, domain.ErrActiveInvite), errors.Is(err, domain.ErrInvitePolicy):
		return badRequest(err.Error())
	case errors.Is(err, domain.ErrUsernameExists), errors.Is(err, domain.ErrTokenLimit):
		return &apiError{status: 409, detail: err.Error()}
	case errors.Is(err, domain.ErrStaleAuthorization):
		return &apiError{status: 401, detail: "Authentication state changed; authenticate again"}
	case errors.Is(err, domain.ErrAdminForbidden):
		return &apiError{status: 403, detail: "Admin access required"}
	case errors.Is(err, domain.ErrAudit):
		return serviceUnavailable()
	default:
		return internalError()
	}
}
