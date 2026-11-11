package auth

import (
	"context"
	"database/sql"
	"errors"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
)

// IssueToken rechecks the observed generation and exact authorizing bearer before admitting a token.
// A login atomically replaces only the internal login row; personal tokens retain their quota.
func (repository *Repository) IssueToken(ctx context.Context, authorization domain.Authorization, hash, name string, expires, now float64, isLogin bool, audit *domain.AuditEvent) (domain.TokenInfo, error) {
	var token domain.TokenInfo
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var generation int64
		err := func() error {
			queryResult, err := queries.TokenGeneration(ctx, connection, authdb.TokenGenerationParams{ID: int64(authorization.User.ID)})
			if err != nil {
				return err
			}
			generation = queryResult
			return nil
		}()
		if errors.Is(err, sql.ErrNoRows) || err == nil && generation != authorization.TokenGeneration {
			return domain.ErrStaleAuthorization
		}
		if err != nil {
			return err
		}
		if !isLogin && authorization.TokenHash != nil {
			var exists bool
			if err := func() error {
				queryResult, err := queries.HasActiveToken(ctx, connection, authdb.HasActiveTokenParams{UserID: int64(authorization.User.ID), TokenHash: *authorization.TokenHash, ExpiresAt: now})
				if err != nil {
					return err
				}
				exists = queryResult
				return nil
			}(); err != nil {
				return err
			}
			if !exists {
				return domain.ErrStaleAuthorization
			}
		}
		if _, err := queries.PruneTokens(ctx, connection, authdb.PruneTokensParams{ExpiresAt: now}); err != nil {
			return err
		}
		if isLogin {
			name = domain.ReservedTokenName
			if _, err := queries.DeleteLoginToken(ctx, connection, authdb.DeleteLoginTokenParams{UserID: int64(authorization.User.ID)}); err != nil {
				return err
			}
		} else {
			var count int64
			if err := func() error {
				queryResult, err := queries.TokenCount(ctx, connection, authdb.TokenCountParams{UserID: int64(authorization.User.ID), ExpiresAt: now})
				if err != nil {
					return err
				}
				count = queryResult
				return nil
			}(); err != nil {
				return err
			}
			if count >= domain.ActiveTokenLimit {
				return domain.ErrTokenLimit
			}
		}
		result, err := queries.CreateToken(ctx, connection, authdb.CreateTokenParams{UserID: int64(authorization.User.ID), TokenHash: hash, Name: name, ExpiresAt: expires, CreatedAt: now})
		if err != nil {
			return err
		}
		id, err := result.LastInsertId()
		if err != nil {
			return err
		}
		token = domain.TokenInfo{ID: id, Name: name, ExpiresAt: expires, CreatedAt: now}
		if audit != nil {
			copy := audit.WithTarget(id)
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	if err != nil {
		return domain.TokenInfo{}, err
	}
	return token, nil
}

// VerifyToken returns current identity/generation and removes a token expiring exactly at now.
func (repository *Repository) VerifyToken(ctx context.Context, hash string, now float64) (*domain.Authorization, error) {
	var authorization domain.Authorization
	var expires float64
	var isAdmin int64
	err := func() error {
		queryResult, err := queries.TokenAuthorization(ctx, repository.database, authdb.TokenAuthorizationParams{TokenHash: hash})
		if err != nil {
			return err
		}
		authorization.User.ID = identity.ID(queryResult.UserID)
		expires = queryResult.ExpiresAt
		authorization.User.Username = queryResult.Username
		isAdmin = queryResult.IsAdmin
		authorization.TokenGeneration = queryResult.TokenGeneration
		authorization.User.Email = queryResult.Email
		authorization.User.AuthKind = queryResult.AuthKind
		authorization.User.IsOidcLinked = queryResult.Exists
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if expires <= now {
		_, err := queries.DeleteTokenHash(ctx, repository.database, authdb.DeleteTokenHashParams{TokenHash: hash})
		return nil, err
	}
	authorization.User.IsAdmin = isAdmin != 0
	authorization.TokenHash = &hash
	return &authorization, nil
}

// ListTokens purges expired rows before returning active personal-token metadata.
func (repository *Repository) ListTokens(ctx context.Context, user identity.ID, now float64) ([]domain.TokenInfo, error) {
	if _, err := queries.PruneTokens(ctx, repository.database, authdb.PruneTokensParams{ExpiresAt: now}); err != nil {
		return nil, err
	}
	rows, err := queries.Tokens(ctx, repository.database, authdb.TokensParams{UserID: int64(user), ExpiresAt: now})
	if err != nil {
		return nil, err
	}
	result := make([]domain.TokenInfo, 0, len(rows))
	for _, row := range rows {
		result = append(result, domain.TokenInfo{ID: row.ID, Name: row.Name, ExpiresAt: row.ExpiresAt, CreatedAt: row.CreatedAt})
	}
	return result, nil
}

// RevokeTokenID audits only an actually deleted row owned by the specified user.
func (repository *Repository) RevokeTokenID(ctx context.Context, user identity.ID, tokenID int64, audit *domain.AuditEvent) (bool, error) {
	var deleted bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		result, err := queries.DeleteTokenID(ctx, connection, authdb.DeleteTokenIDParams{ID: tokenID, UserID: int64(user)})
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		deleted = count > 0
		if deleted && audit != nil {
			copy := audit.WithTarget(tokenID)
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	return deleted && err == nil, err
}

// RevokeTokenHash durably audits even an already absent bearer, using bounded writer contention.
func (repository *Repository) RevokeTokenHash(ctx context.Context, hash string, audit *domain.AuditEvent) (bool, error) {
	var deleted bool
	err := repository.Immediate(ctx, true, func(connection *sql.Conn) error {
		var tokenID int64
		lookup := func() error {
			queryResult, err := queries.TokenID(ctx, connection, authdb.TokenIDParams{TokenHash: hash})
			if err != nil {
				return err
			}
			tokenID = queryResult
			return nil
		}()
		if lookup != nil && !errors.Is(lookup, sql.ErrNoRows) {
			return lookup
		}
		result, err := queries.DeleteTokenHash(ctx, connection, authdb.DeleteTokenHashParams{TokenHash: hash})
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		deleted = count > 0
		if audit != nil {
			copy := *audit
			if lookup == nil {
				copy = copy.WithTarget(tokenID)
			}
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	return deleted && err == nil, err
}

// RevokeAll increments the generation even if no token rows remain.
func (repository *Repository) RevokeAll(ctx context.Context, user identity.ID, audit domain.AuditEvent) (int64, error) {
	var deleted int64
	err := repository.Immediate(ctx, true, func(connection *sql.Conn) error {
		result, err := queries.AdvanceTokenGeneration(ctx, connection, authdb.AdvanceTokenGenerationParams{ID: int64(user)})
		if err != nil {
			return err
		}
		if count, err := result.RowsAffected(); err != nil {
			return err
		} else if count != 1 {
			return domain.ErrCredentialInvariant
		}
		result, err = queries.DeleteUserTokens(ctx, connection, authdb.DeleteUserTokensParams{UserID: int64(user)})
		if err != nil {
			return err
		}
		deleted, err = result.RowsAffected()
		if err != nil {
			return err
		}
		return InsertAudit(ctx, connection, &audit)
	})
	if err != nil {
		return 0, err
	}
	return deleted, nil
}

// UpgradeLegacy performs one opaque hash-and-salt CAS without revoking sessions or increasing generation.
func (repository *Repository) UpgradeLegacy(ctx context.Context, observed domain.Credentials, replacement string, now float64) (bool, error) {
	result, err := queries.UpgradePassword(ctx, repository.database, authdb.UpgradePasswordParams{PasswordHash: replacement, UpdatedAt: now, ID: int64(observed.User.ID), PasswordHash_2: observed.PasswordHash, Salt: observed.Salt})
	if err != nil {
		return false, err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return false, err
	}
	if count > 1 {
		return false, domain.ErrCredentialInvariant
	}
	return count == 1, nil
}

// ChangePassword admits one winner for the exact credential row observed before verification.
func (repository *Repository) ChangePassword(ctx context.Context, observed domain.Credentials, replacement, salt string, now float64, audit domain.AuditEvent) (bool, error) {
	if observed.User.AuthKind == "oidc" {
		return false, domain.ErrOidcPassword
	}
	var changed bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		result, err := queries.ChangePassword(ctx, connection, authdb.ChangePasswordParams{PasswordHash: replacement, Salt: salt, UpdatedAt: now, ID: int64(observed.User.ID), PasswordHash_2: observed.PasswordHash, Salt_2: observed.Salt})
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		if count > 1 {
			return domain.ErrCredentialInvariant
		}
		if count == 0 {
			return nil
		}
		changed = true
		if _, err := queries.DeleteUserTokens(ctx, connection, authdb.DeleteUserTokensParams{UserID: int64(observed.User.ID)}); err != nil {
			return err
		}
		return InsertAudit(ctx, connection, &audit)
	})
	return changed && err == nil, err
}

// ResetPassword optionally fences an administrator actor inside the credential mutation transaction.
// A nil actor is reserved for trusted internal/CLI reset callers.
func (repository *Repository) ResetPassword(ctx context.Context, actor *identity.ID, user identity.ID, replacement, salt string, now float64, audit *domain.AuditEvent) (bool, error) {
	var changed bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if actor != nil {
			if err := RequireAdministrator(ctx, connection, *actor); err != nil {
				return err
			}
		}
		var authKind string
		lookup := func() error {
			queryResult, err := queries.AuthKind(ctx, connection, authdb.AuthKindParams{ID: int64(user)})
			if err != nil {
				return err
			}
			authKind = queryResult
			return nil
		}()
		if lookup != nil && !errors.Is(lookup, sql.ErrNoRows) {
			return lookup
		}
		if authKind == "oidc" {
			return domain.ErrOidcPassword
		}
		if errors.Is(lookup, sql.ErrNoRows) {
			return nil
		}
		result, err := queries.ResetPassword(ctx, connection, authdb.ResetPasswordParams{PasswordHash: replacement, Salt: salt, UpdatedAt: now, ID: int64(user)})
		if err != nil {
			return err
		}
		if count, err := result.RowsAffected(); err != nil {
			return err
		} else if count != 1 {
			return domain.ErrCredentialInvariant
		}
		changed = true
		if _, err := queries.DeleteUserTokens(ctx, connection, authdb.DeleteUserTokensParams{UserID: int64(user)}); err != nil {
			return err
		}
		if audit != nil {
			copy := *audit
			if actor != nil {
				copy = copy.WithIdentity(int64(*actor), int64(user))
			}
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	return changed && err == nil, err
}
