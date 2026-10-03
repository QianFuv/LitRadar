package auth

import (
	"context"
	"database/sql"
	"errors"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
)

// IssueToken rechecks the observed generation and exact authorizing bearer before admitting a token.
// A login atomically replaces only the internal login row; personal tokens retain their quota.
func (repository *Repository) IssueToken(ctx context.Context, authorization domain.Authorization, hash, name string, expires, now float64, isLogin bool, audit *domain.AuditEvent) (domain.TokenInfo, error) {
	var token domain.TokenInfo
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var generation int64
		err := connection.QueryRowContext(ctx, "SELECT token_generation FROM users WHERE id=?", authorization.User.Id).Scan(&generation)
		if errors.Is(err, sql.ErrNoRows) || err == nil && generation != authorization.TokenGeneration {
			return domain.ErrStaleAuthorization
		}
		if err != nil {
			return err
		}
		if !isLogin && authorization.TokenHash != nil {
			var exists bool
			if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM access_tokens WHERE user_id=? AND token_hash=? AND expires_at>?)", authorization.User.Id, *authorization.TokenHash, now).Scan(&exists); err != nil {
				return err
			}
			if !exists {
				return domain.ErrStaleAuthorization
			}
		}
		if _, err := connection.ExecContext(ctx, "DELETE FROM access_tokens WHERE expires_at<=?", now); err != nil {
			return err
		}
		if isLogin {
			name = domain.ReservedTokenName
			if _, err := connection.ExecContext(ctx, "DELETE FROM access_tokens WHERE user_id=? AND name='login'", authorization.User.Id); err != nil {
				return err
			}
		} else {
			var count int64
			if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM access_tokens WHERE user_id=? AND expires_at>? AND name!='login'", authorization.User.Id, now).Scan(&count); err != nil {
				return err
			}
			if count >= domain.ActiveTokenLimit {
				return domain.ErrTokenLimit
			}
		}
		result, err := connection.ExecContext(ctx, "INSERT INTO access_tokens(user_id,token_hash,name,expires_at,created_at) VALUES(?,?,?,?,?)", authorization.User.Id, hash, name, expires, now)
		if err != nil {
			return err
		}
		id, err := result.LastInsertId()
		if err != nil {
			return err
		}
		token = domain.TokenInfo{Id: id, Name: name, ExpiresAt: expires, CreatedAt: now}
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
	var expires, created float64
	var isAdmin int64
	err := repository.database.QueryRowContext(ctx, `SELECT t.user_id,t.expires_at,u.username,u.is_admin,u.created_at,u.token_generation FROM access_tokens t JOIN users u ON t.user_id=u.id WHERE t.token_hash=?`, hash).Scan(&authorization.User.Id, &expires, &authorization.User.Username, &isAdmin, &created, &authorization.TokenGeneration)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if expires <= now {
		_, err := repository.database.ExecContext(ctx, "DELETE FROM access_tokens WHERE token_hash=?", hash)
		return nil, err
	}
	authorization.User.IsAdmin = isAdmin != 0
	authorization.TokenHash = &hash
	return &authorization, nil
}

// ListTokens purges expired rows before returning active personal-token metadata.
func (repository *Repository) ListTokens(ctx context.Context, user identity.Id, now float64) ([]domain.TokenInfo, error) {
	if _, err := repository.database.ExecContext(ctx, "DELETE FROM access_tokens WHERE expires_at<=?", now); err != nil {
		return nil, err
	}
	rows, err := repository.database.QueryContext(ctx, "SELECT id,name,expires_at,created_at FROM access_tokens WHERE user_id=? AND expires_at>? AND name!='login' ORDER BY created_at DESC", user, now)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []domain.TokenInfo{}
	for rows.Next() {
		var token domain.TokenInfo
		if err := rows.Scan(&token.Id, &token.Name, &token.ExpiresAt, &token.CreatedAt); err != nil {
			return nil, err
		}
		result = append(result, token)
	}
	return result, rows.Err()
}

// RevokeTokenId audits only an actually deleted row owned by the specified user.
func (repository *Repository) RevokeTokenId(ctx context.Context, user identity.Id, tokenId int64, audit *domain.AuditEvent) (bool, error) {
	var deleted bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		result, err := connection.ExecContext(ctx, "DELETE FROM access_tokens WHERE id=? AND user_id=?", tokenId, user)
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		deleted = count > 0
		if deleted && audit != nil {
			copy := audit.WithTarget(tokenId)
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
		var tokenId int64
		lookup := connection.QueryRowContext(ctx, "SELECT id FROM access_tokens WHERE token_hash=?", hash).Scan(&tokenId)
		if lookup != nil && !errors.Is(lookup, sql.ErrNoRows) {
			return lookup
		}
		result, err := connection.ExecContext(ctx, "DELETE FROM access_tokens WHERE token_hash=?", hash)
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
				copy = copy.WithTarget(tokenId)
			}
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	return deleted && err == nil, err
}

// RevokeAll increments the generation even if no token rows remain.
func (repository *Repository) RevokeAll(ctx context.Context, user identity.Id, audit domain.AuditEvent) (int64, error) {
	var deleted int64
	err := repository.Immediate(ctx, true, func(connection *sql.Conn) error {
		result, err := connection.ExecContext(ctx, "UPDATE users SET token_generation=token_generation+1 WHERE id=?", user)
		if err != nil {
			return err
		}
		if count, err := result.RowsAffected(); err != nil {
			return err
		} else if count != 1 {
			return domain.ErrCredentialInvariant
		}
		result, err = connection.ExecContext(ctx, "DELETE FROM access_tokens WHERE user_id=?", user)
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
	result, err := repository.database.ExecContext(ctx, "UPDATE users SET password_hash=?,salt='',updated_at=? WHERE id=? AND password_hash=? AND salt=?", replacement, now, observed.User.Id, observed.PasswordHash, observed.Salt)
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
	var changed bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		result, err := connection.ExecContext(ctx, "UPDATE users SET password_hash=?,salt=?,updated_at=?,token_generation=token_generation+1 WHERE id=? AND password_hash=? AND salt=?", replacement, salt, now, observed.User.Id, observed.PasswordHash, observed.Salt)
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
		if _, err := connection.ExecContext(ctx, "DELETE FROM access_tokens WHERE user_id=?", observed.User.Id); err != nil {
			return err
		}
		return InsertAudit(ctx, connection, &audit)
	})
	return changed && err == nil, err
}

// ResetPassword optionally fences an administrator actor inside the credential mutation transaction.
// A nil actor is reserved for trusted internal/CLI reset callers.
func (repository *Repository) ResetPassword(ctx context.Context, actor *identity.Id, user identity.Id, replacement, salt string, now float64, audit *domain.AuditEvent) (bool, error) {
	var changed bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if actor != nil {
			if err := RequireAdministrator(ctx, connection, *actor); err != nil {
				return err
			}
		}
		var exists bool
		if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users WHERE id=?)", user).Scan(&exists); err != nil {
			return err
		}
		if !exists {
			return nil
		}
		result, err := connection.ExecContext(ctx, "UPDATE users SET password_hash=?,salt=?,updated_at=?,token_generation=token_generation+1 WHERE id=?", replacement, salt, now, user)
		if err != nil {
			return err
		}
		if count, err := result.RowsAffected(); err != nil {
			return err
		} else if count != 1 {
			return domain.ErrCredentialInvariant
		}
		changed = true
		if _, err := connection.ExecContext(ctx, "DELETE FROM access_tokens WHERE user_id=?", user); err != nil {
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
