// Package auth implements password, session, token and invite application policy.
package auth

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"strings"
	"time"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/platform/admission"
	"github.com/QianFuv/LitRadar/internal/platform/cryptography"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
)

const dummyPasswordHash = "$argon2id$v=19$m=19456,t=2,p=1$bGl0cmFkYXItZHVtbXkhIQ$ZYJd4/sBFG46tq7/498a19Zgzc/4MglD6AaOL5G1bdM"

// Service shares a bounded password work gate across requests using one repository.
type Service struct {
	repository   *storage.Repository
	passwordGate *admission.Gate
	now          func() float64
}

// New requires the caller to run migrations before exposing authentication endpoints.
func New(repository *storage.Repository, passwordConcurrency int) *Service {
	return &Service{repository: repository, passwordGate: admission.New(passwordConcurrency), now: func() float64 { return float64(time.Now().UnixNano()) / 1e9 }}
}

// ValidUsername preserves the existing ASCII-only account naming policy without trimming.
func ValidUsername(username string) bool {
	if len(username) < 3 || len(username) > 32 {
		return false
	}
	for _, character := range []byte(username) {
		if !isUsernameByte(character) {
			return false
		}
	}
	return true
}

// isUsernameByte admits only ASCII account-name characters without normalization.
func isUsernameByte(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || character == '_'
}

func validateCredentials(username, password string) error {
	if !ValidUsername(username) {
		return domain.ErrUsername
	}
	if !cryptography.ValidNewPassword(password) {
		return domain.ErrPasswordShort
	}
	return nil
}

// ValidateTokenRequest checks raw scalar length before trimming, reserved name and TTL.
func ValidateTokenRequest(name string, ttl int64) (string, error) {
	if !utf8.ValidString(name) || utf8.RuneCountInString(name) > 100 {
		return "", domain.ErrTokenNameLength
	}
	name = strings.TrimSpace(name)
	if name == domain.ReservedTokenName {
		return "", domain.ErrTokenReservedName
	}
	if ttl < 3600 || ttl > domain.MaximumTokenTtl {
		return "", domain.ErrTokenTtl
	}
	return name, nil
}

func (service *Service) hashPassword(ctx context.Context, password string) (string, error) {
	return admission.Run(ctx, service.passwordGate, func() (string, error) {
		hash, err := cryptography.HashPassword(password)
		if err != nil {
			return "", domain.ErrEntropy
		}
		return hash, nil
	})
}

func (service *Service) verifyPassword(ctx context.Context, password, salt, hash string) (cryptography.Verification, error) {
	return admission.Run(ctx, service.passwordGate, func() (cryptography.Verification, error) {
		return cryptography.VerifyPassword(password, salt, hash), nil
	})
}

func randomHex(size int) (string, error) {
	value := make([]byte, size)
	if _, err := rand.Read(value); err != nil {
		return "", domain.ErrEntropy
	}
	defer clear(value)
	return hex.EncodeToString(value), nil
}

// HashToken is the persisted lowercase SHA-256 bearer lookup key.
func HashToken(token string) string {
	digest := sha256.Sum256([]byte(token))
	return hex.EncodeToString(digest[:])
}

func (service *Service) event(action string, audit *domain.AuditEvent) domain.AuditEvent {
	if audit != nil {
		return *audit
	}
	return domain.AuditEvent{Action: action, Outcome: "completed", OccurredAt: service.now()}
}

// Bootstrap validates and hashes outside the single-winner administrator transaction.
func (service *Service) Bootstrap(ctx context.Context, username, password string, audit *domain.AuditEvent) (domain.User, error) {
	if err := validateCredentials(username, password); err != nil {
		return domain.User{}, err
	}
	hash, err := service.hashPassword(ctx, password)
	if err != nil {
		return domain.User{}, err
	}
	event := service.event("admin_bootstrap", audit)
	return service.repository.Bootstrap(ctx, username, hash, "", service.now(), &event)
}

// Register atomically consumes an invite after validating the new account's credentials.
func (service *Service) Register(ctx context.Context, username, password string, invite *string, audit *domain.AuditEvent) (domain.User, error) {
	if err := validateCredentials(username, password); err != nil {
		return domain.User{}, err
	}
	hash, err := service.hashPassword(ctx, password)
	if err != nil {
		return domain.User{}, err
	}
	event := service.event("register", audit)
	return service.repository.Register(ctx, username, hash, "", invite, service.now(), &event)
}

// VerifyPasswordAuthorization performs a real dummy KDF for absent users and upgrades valid legacy rows.
func (service *Service) VerifyPasswordAuthorization(ctx context.Context, username, password string) (*domain.Authorization, error) {
	credentials, err := service.repository.CredentialsByName(ctx, username)
	if err != nil {
		return nil, err
	}
	return service.verifyObservedAuthorization(ctx, credentials, password)
}

// verifyObservedAuthorization performs dummy or observed password work before authorizing identity.
func (service *Service) verifyObservedAuthorization(ctx context.Context, credentials *domain.Credentials, password string) (*domain.Authorization, error) {
	if credentials == nil {
		_, err := service.verifyPassword(ctx, password, "", dummyPasswordHash)
		return nil, err
	}
	verification, err := service.verifyPassword(ctx, password, credentials.Salt, credentials.PasswordHash)
	if err != nil {
		return nil, err
	}
	if verification == cryptography.Invalid {
		return nil, nil
	}
	if verification == cryptography.ValidLegacy {
		return service.upgradeObservedLegacyAuthorization(ctx, credentials, password)
	}
	return &domain.Authorization{User: credentials.User, TokenGeneration: credentials.TokenGeneration}, nil
}

// upgradeObservedLegacyAuthorization rechecks a winning credential row while retaining captured identity.
func (service *Service) upgradeObservedLegacyAuthorization(ctx context.Context, credentials *domain.Credentials, password string) (*domain.Authorization, error) {
	generation := credentials.TokenGeneration
	replacement, err := service.hashPassword(ctx, password)
	if err != nil {
		return nil, err
	}
	changed, err := service.repository.UpgradeLegacy(ctx, *credentials, replacement, service.now())
	if err != nil {
		return nil, err
	}
	if !changed {
		current, err := service.repository.CredentialsById(ctx, credentials.User.Id)
		if err != nil || current == nil {
			return nil, err
		}
		verification, err := service.verifyPassword(ctx, password, current.Salt, current.PasswordHash)
		if err != nil {
			return nil, err
		}
		if verification == cryptography.Invalid {
			return nil, nil
		}
		generation = current.TokenGeneration
	}
	return &domain.Authorization{User: credentials.User, TokenGeneration: generation}, nil
}

// Login replaces the browser session only while the verified credential generation remains current.
func (service *Service) Login(ctx context.Context, username, password string, audit *domain.AuditEvent) (domain.LoginSession, error) {
	authorization, err := service.VerifyPasswordAuthorization(ctx, username, password)
	if err != nil {
		return domain.LoginSession{}, err
	}
	if authorization == nil {
		return domain.LoginSession{}, domain.ErrCredentials
	}
	return service.createLogin(ctx, *authorization, service.event("login", audit))
}

func (service *Service) createLogin(ctx context.Context, authorization domain.Authorization, audit domain.AuditEvent) (domain.LoginSession, error) {
	token, err := randomHex(32)
	if err != nil {
		return domain.LoginSession{}, err
	}
	now := service.now()
	actor := int64(authorization.User.Id)
	audit.ActorId = &actor
	row, err := service.repository.IssueToken(ctx, authorization, HashToken(token), domain.ReservedTokenName, now+float64(domain.DefaultTokenTtl), now, true, &audit)
	if errors.Is(err, domain.ErrStaleAuthorization) {
		return domain.LoginSession{}, domain.ErrCredentials
	}
	if err != nil {
		return domain.LoginSession{}, err
	}
	return domain.LoginSession{User: authorization.User, Token: token, ExpiresAt: row.ExpiresAt}, nil
}

// CreateToken uses the request's captured authorization, including its exact bearer hash.
func (service *Service) CreateToken(ctx context.Context, authorization domain.Authorization, name string, ttl int64, audit *domain.AuditEvent) (domain.IssuedToken, error) {
	if authorization.TokenHash == nil {
		return domain.IssuedToken{}, domain.ErrStaleAuthorization
	}
	return service.createToken(ctx, authorization, name, ttl, audit)
}

func (service *Service) createToken(ctx context.Context, authorization domain.Authorization, name string, ttl int64, audit *domain.AuditEvent) (domain.IssuedToken, error) {
	name, err := ValidateTokenRequest(name, ttl)
	if err != nil {
		return domain.IssuedToken{}, err
	}
	token, err := randomHex(32)
	if err != nil {
		return domain.IssuedToken{}, err
	}
	now := service.now()
	event := service.event("token_create", audit)
	if audit == nil {
		actor := int64(authorization.User.Id)
		event.ActorId = &actor
	}
	row, err := service.repository.IssueToken(ctx, authorization, HashToken(token), name, now+float64(ttl), now, false, &event)
	if err != nil {
		return domain.IssuedToken{}, err
	}
	return domain.IssuedToken{Id: row.Id, Token: token, Name: row.Name, ExpiresAt: row.ExpiresAt}, nil
}

// CreateTrustedToken is reserved for internal callers already holding user authority.
func (service *Service) CreateTrustedToken(ctx context.Context, user identity.Id, name string, ttl int64, audit *domain.AuditEvent) (domain.IssuedToken, error) {
	if _, err := ValidateTokenRequest(name, ttl); err != nil {
		return domain.IssuedToken{}, err
	}
	credentials, err := service.repository.CredentialsById(ctx, user)
	if err != nil {
		return domain.IssuedToken{}, err
	}
	if credentials == nil {
		return domain.IssuedToken{}, domain.ErrStaleAuthorization
	}
	return service.createToken(ctx, domain.Authorization{User: credentials.User, TokenGeneration: credentials.TokenGeneration}, name, ttl, audit)
}

// VerifyToken captures fresh identity and generation for later transaction-local rechecks.
func (service *Service) VerifyToken(ctx context.Context, token string) (*domain.Authorization, error) {
	return service.repository.VerifyToken(ctx, HashToken(token), service.now())
}

// ListTokens returns active personal credentials' public metadata.
func (service *Service) ListTokens(ctx context.Context, user identity.Id) ([]domain.TokenInfo, error) {
	return service.repository.ListTokens(ctx, user, service.now())
}

// RevokeTokenId revokes one owned credential and commits its completion event atomically.
func (service *Service) RevokeTokenId(ctx context.Context, user identity.Id, tokenId int64, audit domain.AuditEvent) (bool, error) {
	return service.repository.RevokeTokenId(ctx, user, tokenId, &audit)
}

// RevokeToken revokes an exact raw bearer without writing the raw value to storage or audit.
func (service *Service) RevokeToken(ctx context.Context, token string, audit domain.AuditEvent) (bool, error) {
	return service.repository.RevokeTokenHash(ctx, HashToken(token), &audit)
}

// RevokeAll invalidates both existing and concurrently prepared credentials.
func (service *Service) RevokeAll(ctx context.Context, user identity.Id, audit domain.AuditEvent) (int64, error) {
	return service.repository.RevokeAll(ctx, user, audit)
}

// ChangePassword validates the replacement first and commits only for the verified old hash and salt.
func (service *Service) ChangePassword(ctx context.Context, user identity.Id, oldPassword, newPassword string, audit *domain.AuditEvent) (bool, error) {
	if !cryptography.ValidNewPassword(newPassword) {
		return false, domain.ErrPasswordShort
	}
	credentials, err := service.repository.CredentialsById(ctx, user)
	if err != nil || credentials == nil {
		return false, err
	}
	verification, err := service.verifyPassword(ctx, oldPassword, credentials.Salt, credentials.PasswordHash)
	if err != nil {
		return false, err
	}
	if verification == cryptography.Invalid {
		return false, nil
	}
	hash, err := service.hashPassword(ctx, newPassword)
	if err != nil {
		return false, err
	}
	event := service.event("password_change", audit)
	if audit == nil {
		event = event.WithIdentity(int64(user), int64(user))
	}
	return service.repository.ChangePassword(ctx, *credentials, hash, "", service.now(), event)
}

// ResetPassword supports trusted CLI and actor-fenced administrator reset paths.
func (service *Service) ResetPassword(ctx context.Context, actor *identity.Id, user identity.Id, password string, audit *domain.AuditEvent) (bool, error) {
	if !cryptography.ValidNewPassword(password) {
		return false, domain.ErrPasswordShort
	}
	hash, err := service.hashPassword(ctx, password)
	if err != nil {
		return false, err
	}
	event := service.event("user_password_reset", audit)
	if audit == nil {
		event = event.WithTarget(int64(user))
	}
	return service.repository.ResetPassword(ctx, actor, user, hash, "", service.now(), &event)
}

// IssueInvite creates or explicitly rotates the standard seven-day, single-use owner invite.
func (service *Service) IssueInvite(ctx context.Context, user identity.Id, isRotation bool, audit *domain.AuditEvent) (domain.InviteResponse, error) {
	code, err := randomHex(8)
	if err != nil {
		return domain.InviteResponse{}, err
	}
	now := service.now()
	action := "invite_create"
	if isRotation {
		action = "invite_rotate"
	}
	event := service.event(action, audit)
	if audit == nil {
		actor := int64(user)
		event.ActorId = &actor
	}
	row, err := service.repository.IssueInvite(ctx, user, code, now, now+604800, 1, isRotation, &event)
	if err != nil {
		return domain.InviteResponse{}, err
	}
	return row.Response(now), nil
}

// RevokeInvite irreversibly revokes the latest unrevoked owner issuance.
func (service *Service) RevokeInvite(ctx context.Context, user identity.Id, audit *domain.AuditEvent) (bool, error) {
	event := service.event("invite_revoke", audit)
	if audit == nil {
		actor := int64(user)
		event.ActorId = &actor
	}
	return service.repository.RevokeInvite(ctx, user, service.now(), &event)
}

// Invite projects the latest issuance, retaining explicit null revocation metadata.
func (service *Service) Invite(ctx context.Context, user identity.Id) (*domain.InviteResponse, error) {
	row, err := service.repository.Invite(ctx, user)
	if err != nil || row == nil {
		return nil, err
	}
	response := row.Response(service.now())
	return &response, nil
}

// BootstrapRequired reports whether local administrator bootstrap is still needed.
func (service *Service) BootstrapRequired(ctx context.Context) (bool, error) {
	return service.repository.BootstrapRequired(ctx)
}

// InviteRequired is always true for public registration, including an empty installation.
func (service *Service) InviteRequired() bool { return true }
