package auth

import (
	"context"
	"crypto/rand"
	"database/sql"
	"errors"
	"fmt"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
)

func requireObservedSession(ctx context.Context, connection *sql.Conn, authorization domain.Authorization, now float64) error {
	if authorization.TokenHash == nil {
		return domain.ErrExpiredSession
	}
	var exists bool
	err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users u JOIN access_tokens t ON t.user_id=u.id WHERE u.id=? AND u.token_generation=? AND u.auth_kind='local' AND t.token_hash=? AND t.expires_at>?)", authorization.User.Id, authorization.TokenGeneration, *authorization.TokenHash, now).Scan(&exists)
	if err != nil {
		return err
	}
	if !exists {
		return domain.ErrExpiredSession
	}
	return nil
}

// CompleteOidc resolves an issuer/subject without matching accounts by profile attributes.
// Linking and provisioning, profile synchronization and audits share one transaction.
// A provisioned account uses the validated preferred username when free, otherwise a random name.
func (repository *Repository) CompleteOidc(ctx context.Context, issuer, subject, email, preferredUsername string, link *domain.Authorization, now float64, audit domain.AuditEvent) (domain.Authorization, error) {
	var result domain.Authorization
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var owner identity.Id
		lookup := connection.QueryRowContext(ctx, "SELECT user_id FROM user_identities WHERE issuer=? AND subject=?", issuer, subject).Scan(&owner)
		if lookup != nil && !errors.Is(lookup, sql.ErrNoRows) {
			return lookup
		}
		if link != nil {
			if err := requireObservedSession(ctx, connection, *link, now); err != nil {
				return err
			}
			var hasIdentity bool
			if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM user_identities WHERE user_id=?)", link.User.Id).Scan(&hasIdentity); err != nil {
				return err
			}
			if lookup == nil || hasIdentity {
				return domain.ErrAccountConflict
			}
			owner = link.User.Id
			if _, err := connection.ExecContext(ctx, "INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(?,?,?,?,?)", owner, issuer, subject, now, now); err != nil {
				return err
			}
			audit.Action = "oidc_link"
		} else if errors.Is(lookup, sql.ErrNoRows) {
			var exists bool
			if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users WHERE email=?)", email).Scan(&exists); err != nil {
				return err
			}
			if exists {
				return domain.ErrEmailExists
			}
			var user domain.User
			var err error
			for attempt := 0; attempt < 8; attempt++ {
				username := preferredUsername
				if attempt > 0 || username == "" {
					username, err = identityUsername()
					if err != nil {
						return err
					}
				}
				user, err = insertUser(ctx, connection, username, domain.UnusablePasswordHash, "", false, now)
				if !errors.Is(err, domain.ErrUsernameExists) {
					break
				}
			}
			if err != nil {
				return err
			}
			owner = user.Id
			if _, err := connection.ExecContext(ctx, "UPDATE users SET email=?,auth_kind='oidc' WHERE id=?", email, owner); err != nil {
				return err
			}
			if _, err := connection.ExecContext(ctx, "INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(?,?,?,?,?)", owner, issuer, subject, now, now); err != nil {
				return err
			}
			if err := defaultFolder(ctx, connection, owner, now); err != nil {
				return err
			}
			provision := audit.WithIdentity(int64(owner), int64(owner))
			provision.Action = "oidc_provision"
			if err := InsertAudit(ctx, connection, &provision); err != nil {
				return err
			}
		}
		credentials, err := scanCredentials(connection.QueryRowContext(ctx, credentialSelect+" WHERE id=?", owner))
		if err != nil {
			return err
		}
		if credentials == nil {
			return ErrUserNotFound
		}
		if link == nil && credentials.User.AuthKind == "oidc" {
			var conflict bool
			if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM users WHERE email=? AND id!=?)", email, owner).Scan(&conflict); err != nil {
				return err
			}
			if !conflict {
				if err := updateEmail(ctx, connection, owner, &email, now); err != nil {
					return err
				}
				credentials.User.Email = &email
			}
		}
		if _, err := connection.ExecContext(ctx, "UPDATE user_identities SET last_login_at=? WHERE issuer=? AND subject=?", now, issuer, subject); err != nil {
			return err
		}
		result = domain.Authorization{User: credentials.User, TokenGeneration: credentials.TokenGeneration}
		if link != nil {
			completed := audit.WithIdentity(int64(owner), int64(owner))
			return InsertAudit(ctx, connection, &completed)
		}
		return nil
	})
	return result, err
}

// UnlinkOidc keeps the local password credential and refuses OIDC-only accounts.
func (repository *Repository) UnlinkOidc(ctx context.Context, authorization domain.Authorization, now float64, audit domain.AuditEvent) (domain.User, error) {
	var user domain.User
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := requireObservedSession(ctx, connection, authorization, now); err != nil {
			return err
		}
		result, err := connection.ExecContext(ctx, "DELETE FROM user_identities WHERE user_id=?", authorization.User.Id)
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		if count == 0 {
			return domain.ErrAccountConflict
		}
		credentials, err := scanCredentials(connection.QueryRowContext(ctx, credentialSelect+" WHERE id=?", authorization.User.Id))
		if err != nil {
			return err
		}
		if credentials == nil {
			return ErrUserNotFound
		}
		user = credentials.User
		event := audit.WithIdentity(int64(user.Id), int64(user.Id))
		return InsertAudit(ctx, connection, &event)
	})
	return user, err
}

func identityUsername() (string, error) {
	var value [12]byte
	if _, err := rand.Read(value[:]); err != nil {
		return "", domain.ErrEntropy
	}
	return fmt.Sprintf("oidc_%x", value[:]), nil
}
