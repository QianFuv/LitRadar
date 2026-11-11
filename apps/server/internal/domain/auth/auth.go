// Package auth defines identity contracts independent of transport and persistence.
package auth

import (
	"errors"
	"fmt"
	"log/slog"
	"math"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
)

const SessionCookieName = "litradar_session"
const DefaultTokenTTL int64 = 604800
const MaximumTokenTTL int64 = 31536000
const ActiveTokenLimit = 50
const ReservedTokenName = "login"

// UnusablePasswordHash cannot be interpreted as a valid password credential.
const UnusablePasswordHash = "!oidc"

var (
	ErrEmail               = errors.New("email must be at most 254 bytes with one @ and no whitespace or control characters")
	ErrEmailExists         = errors.New("email already exists")
	ErrManagedEmail        = errors.New("email is managed by the identity provider")
	ErrOidcPassword        = errors.New("password login is not available for single sign-on accounts")
	ErrAccountConflict     = errors.New("account_conflict")
	ErrExpiredSession      = errors.New("expired_session")
	ErrCredentials         = errors.New("invalid username or password")
	ErrUsername            = errors.New("username must be 3-32 alphanumeric or underscore characters")
	ErrPasswordShort       = errors.New("password must be at least 12 characters")
	ErrTokenNameLength     = errors.New("access token name must be at most 100 Unicode code points")
	ErrTokenReservedName   = errors.New("access token name \"login\" is reserved")
	ErrTokenTTL            = errors.New("access token TTL must be between 3600 and 31536000 seconds")
	ErrTokenLimit          = errors.New("active access token limit of 50 reached; revoke a token before creating another")
	ErrInviteRequired      = errors.New("invite code is required")
	ErrBootstrapRequired   = errors.New("administrator bootstrap is required")
	ErrInvite              = errors.New("invalid, expired, revoked, or exhausted invite code")
	ErrActiveInvite        = errors.New("an unrevoked invite code already exists; rotate it instead")
	ErrInvitePolicy        = errors.New("invite code policy is outside the allowed range")
	ErrUsernameExists      = errors.New("username already exists")
	ErrBootstrapComplete   = errors.New("administrator bootstrap is already complete")
	ErrEntropy             = errors.New("operating-system cryptographic randomness is unavailable")
	ErrCredentialInvariant = errors.New("credential update affected an unexpected number of users")
	ErrStaleAuthorization  = errors.New("token authorization changed before issuance")
	ErrAdminForbidden      = errors.New("admin access required")
	ErrAudit               = errors.New("security audit persistence failed")
)

// ErrDeviceName rejects labels that cannot form a bounded device token name.
var ErrDeviceName = errors.New("device name must contain 1-93 Unicode code points, no control characters, and non-whitespace text")

// User is the public identity projection. Its formatting hides the account name.
type User struct {
	Email        *string     `json:"email"`
	AuthKind     string      `json:"auth_kind"`
	IsOidcLinked bool        `json:"is_oidc_linked"`
	ID           identity.ID `json:"id"`
	Username     string      `json:"username"`
	IsAdmin      bool        `json:"is_admin"`
}

func (user User) String() string {
	return fmt.Sprintf("User{Id:%d Username:[REDACTED] IsAdmin:%t}", user.ID, user.IsAdmin)
}
func (user User) GoString() string { return user.String() }

// Credentials is a private database projection, never a public response.
type Credentials struct {
	User            User    `json:"-"`
	PasswordHash    string  `json:"-"`
	Salt            string  `json:"-"`
	CreatedAt       float64 `json:"-"`
	TokenGeneration int64   `json:"-"`
}

func (credentials Credentials) String() string   { return "Credentials([REDACTED])" }
func (credentials Credentials) GoString() string { return credentials.String() }

// Authorization carries the generation and optional exact token observed by authentication.
type Authorization struct {
	User            User    `json:"-"`
	TokenGeneration int64   `json:"-"`
	TokenHash       *string `json:"-"`
}

func (authorization Authorization) String() string   { return "Authorization([REDACTED])" }
func (authorization Authorization) GoString() string { return authorization.String() }

// TokenInfo exposes metadata without a credential hash or raw bearer value.
type TokenInfo struct {
	ID        int64   `json:"id" nullable:"false" format:"int64"`
	Name      string  `json:"name" nullable:"false"`
	ExpiresAt float64 `json:"expires_at" nullable:"false" format:"double"`
	CreatedAt float64 `json:"created_at" nullable:"false" format:"double"`
}

// IssuedToken is the explicitly authorized one-time bearer disclosure.
type IssuedToken struct {
	ID        int64   `json:"id" nullable:"false" format:"int64"`
	Token     string  `json:"token" nullable:"false"`
	Name      string  `json:"name" nullable:"false"`
	ExpiresAt float64 `json:"expires_at" nullable:"false" format:"double"`
}

func (token IssuedToken) String() string   { return "IssuedToken([REDACTED])" }
func (token IssuedToken) GoString() string { return token.String() }

// LoginSession keeps the browser credential out of ordinary JSON responses.
type LoginSession struct {
	User      User    `json:"user"`
	ExpiresAt float64 `json:"expires_at"`
	Token     string  `json:"-"`
}

func (session LoginSession) String() string   { return "LoginSession([REDACTED])" }
func (session LoginSession) GoString() string { return session.String() }

// InviteRow holds the persisted first redemption and bounded lifecycle.
type InviteRow struct {
	ID        int64        `json:"-"`
	Code      string       `json:"-"`
	UsedBy    *identity.ID `json:"-"`
	UsedAt    *float64     `json:"-"`
	ExpiresAt float64      `json:"-"`
	RevokedAt *float64     `json:"-"`
	MaxUses   int64        `json:"-"`
	UseCount  int64        `json:"-"`
	CreatedAt float64      `json:"-"`
}

func (invite InviteRow) String() string   { return "InviteRow([REDACTED])" }
func (invite InviteRow) GoString() string { return invite.String() }

// InviteResponse is the owner-visible issuance and lifecycle projection.
type InviteResponse struct {
	ID        int64    `json:"id" nullable:"false" format:"int64"`
	Code      string   `json:"code" nullable:"false"`
	Used      bool     `json:"used" nullable:"false"`
	Status    string   `json:"status" nullable:"false" enum:"active,expired,revoked,exhausted"`
	ExpiresAt float64  `json:"expires_at" nullable:"false" format:"double"`
	RevokedAt *float64 `json:"revoked_at" required:"false" nullable:"true" format:"double"`
	MaxUses   int64    `json:"max_uses" nullable:"false" format:"int64"`
	UseCount  int64    `json:"use_count" nullable:"false" format:"int64"`
	CreatedAt float64  `json:"created_at" nullable:"false" format:"double"`
}

func (invite InviteResponse) String() string   { return "InviteResponse([REDACTED])" }
func (invite InviteResponse) GoString() string { return invite.String() }

// Response applies the original revocation, exhaustion and expiry precedence.
func (invite InviteRow) Response(now float64) InviteResponse {
	status := "active"
	switch {
	case invite.RevokedAt != nil:
		status = "revoked"
	case invite.UseCount >= invite.MaxUses:
		status = "exhausted"
	case invite.ExpiresAt <= now:
		status = "expired"
	}
	return InviteResponse{invite.ID, invite.Code, invite.UseCount > 0, status, invite.ExpiresAt, invite.RevokedAt, invite.MaxUses, invite.UseCount, invite.CreatedAt}
}

// ValidInvitePolicy bounds both timestamps and the maximum redemption count.
func ValidInvitePolicy(now, expires float64, maxUses int64) bool {
	return !math.IsNaN(now) && !math.IsInf(now, 0) && !math.IsNaN(expires) && !math.IsInf(expires, 0) && expires > now && expires-now <= float64(MaximumTokenTTL) && maxUses >= 1 && maxUses <= 1000
}

// AuditEvent contains only bounded structured security metadata.
type AuditEvent struct {
	ActorID           *int64
	TargetID          *int64
	Action            string
	Outcome           string
	Reason            string
	RequestID         string
	SourceClass       string
	Bucket            string
	RejectedCount     int64
	RetryAfterSeconds int64
	OccurredAt        float64
}

// WithIdentity copies an event so a mutation cannot change its caller's audit template.
func (event AuditEvent) WithIdentity(actor, target int64) AuditEvent {
	event.ActorID = &actor
	event.TargetID = &target
	return event
}

// WithTarget copies an event and attaches the committed resource identifier.
func (event AuditEvent) WithTarget(target int64) AuditEvent { event.TargetID = &target; return event }

// LogValue keeps private account data out of both structured and text log handlers.
func (user User) LogValue() slog.Value               { return slog.StringValue(user.String()) }
func (credentials Credentials) LogValue() slog.Value { return slog.StringValue(credentials.String()) }
func (authorization Authorization) LogValue() slog.Value {
	return slog.StringValue(authorization.String())
}
func (token IssuedToken) LogValue() slog.Value     { return slog.StringValue(token.String()) }
func (session LoginSession) LogValue() slog.Value  { return slog.StringValue(session.String()) }
func (invite InviteRow) LogValue() slog.Value      { return slog.StringValue(invite.String()) }
func (invite InviteResponse) LogValue() slog.Value { return slog.StringValue(invite.String()) }
