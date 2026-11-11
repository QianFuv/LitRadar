package auth

import (
	"context"
	"crypto/rand"
	"database/sql"
	"errors"
	"fmt"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
)

func requireObservedSession(ctx context.Context, connection *sql.Conn, authorization domain.Authorization, now float64) error {
	if authorization.TokenHash == nil {
		return domain.ErrExpiredSession
	}
	var exists bool
	err := func() error {
		queryResult, err := queries.ObservedSession(ctx, connection, authdb.ObservedSessionParams{ID: int64(authorization.User.ID), TokenGeneration: authorization.TokenGeneration, TokenHash: *authorization.TokenHash, ExpiresAt: now})
		if err != nil {
			return err
		}
		exists = queryResult
		return nil
	}()
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
		var owner identity.ID
		lookup := func() error {
			queryResult, err := queries.IdentityOwner(ctx, connection, authdb.IdentityOwnerParams{Issuer: issuer, Subject: subject})
			if err != nil {
				return err
			}
			owner = identity.ID(queryResult)
			return nil
		}()
		if lookup != nil && !errors.Is(lookup, sql.ErrNoRows) {
			return lookup
		}
		if link != nil {
			if err := requireObservedSession(ctx, connection, *link, now); err != nil {
				return err
			}
			var hasIdentity bool
			if err := func() error {
				queryResult, err := queries.HasIdentity(ctx, connection, authdb.HasIdentityParams{UserID: int64(link.User.ID)})
				if err != nil {
					return err
				}
				hasIdentity = queryResult
				return nil
			}(); err != nil {
				return err
			}
			if lookup == nil || hasIdentity {
				return domain.ErrAccountConflict
			}
			owner = link.User.ID
			if _, err := queries.CreateIdentity(ctx, connection, authdb.CreateIdentityParams{UserID: int64(owner), Issuer: issuer, Subject: subject, CreatedAt: now, LastLoginAt: now}); err != nil {
				return err
			}
			audit.Action = "oidc_link"
		} else if errors.Is(lookup, sql.ErrNoRows) {
			var exists bool
			if err := func() error {
				queryResult, err := queries.HasEmail(ctx, connection, authdb.HasEmailParams{Email: new(email)})
				if err != nil {
					return err
				}
				exists = queryResult
				return nil
			}(); err != nil {
				return err
			}
			if exists {
				return domain.ErrEmailExists
			}
			var user domain.User
			var err error
			for attempt := range 8 {
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
			owner = user.ID
			if _, err := queries.SetOIDCEmail(ctx, connection, authdb.SetOIDCEmailParams{Email: new(email), ID: int64(owner)}); err != nil {
				return err
			}
			if _, err := queries.CreateIdentity(ctx, connection, authdb.CreateIdentityParams{UserID: int64(owner), Issuer: issuer, Subject: subject, CreatedAt: now, LastLoginAt: now}); err != nil {
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
		credentials, err := decodeCredentials(queries.CredentialsByID(ctx, connection, authdb.CredentialsByIDParams{ID: int64(owner)}))
		if err != nil {
			return err
		}
		if credentials == nil {
			return ErrUserNotFound
		}
		if link == nil && credentials.User.AuthKind == "oidc" {
			var conflict bool
			if err := func() error {
				queryResult, err := queries.HasOtherEmail(ctx, connection, authdb.HasOtherEmailParams{Email: new(email), ID: int64(owner)})
				if err != nil {
					return err
				}
				conflict = queryResult
				return nil
			}(); err != nil {
				return err
			}
			if !conflict {
				if err := updateEmail(ctx, connection, owner, &email, now); err != nil {
					return err
				}
				credentials.User.Email = &email
			}
		}
		if _, err := queries.RecordIdentityLogin(ctx, connection, authdb.RecordIdentityLoginParams{LastLoginAt: now, Issuer: issuer, Subject: subject}); err != nil {
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
		result, err := queries.DeleteIdentity(ctx, connection, authdb.DeleteIdentityParams{UserID: int64(authorization.User.ID)})
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
		credentials, err := decodeCredentials(queries.CredentialsByID(ctx, connection, authdb.CredentialsByIDParams{ID: int64(authorization.User.ID)}))
		if err != nil {
			return err
		}
		if credentials == nil {
			return ErrUserNotFound
		}
		user = credentials.User
		event := audit.WithIdentity(int64(user.ID), int64(user.ID))
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
