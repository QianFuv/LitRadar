package auth

import (
	"context"
	"database/sql"
	"errors"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	sqlite3 "github.com/mattn/go-sqlite3"
)

func updateEmail(ctx context.Context, connection *sql.Conn, user identity.Id, email *string, now float64) error {
	_, err := connection.ExecContext(ctx, "UPDATE users SET email=?,updated_at=? WHERE id=?", email, now, user)
	var failure sqlite3.Error
	if errors.As(err, &failure) && failure.ExtendedCode == sqlite3.ErrConstraintUnique {
		return domain.ErrEmailExists
	}
	return err
}

// UpdateEmail changes only local accounts and commits the audit with the mutation.
func (repository *Repository) UpdateEmail(ctx context.Context, user identity.Id, email *string, now float64, audit *domain.AuditEvent) (domain.User, error) {
	var credentials *domain.Credentials
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var err error
		credentials, err = scanCredentials(connection.QueryRowContext(ctx, credentialSelect+" WHERE id=?", user))
		if err != nil {
			return err
		}
		if credentials == nil {
			return ErrUserNotFound
		}
		if credentials.User.AuthKind == "oidc" {
			return domain.ErrManagedEmail
		}
		if err := updateEmail(ctx, connection, user, email, now); err != nil {
			return err
		}
		credentials.User.Email = email
		return InsertAudit(ctx, connection, audit)
	})
	if err != nil {
		return domain.User{}, err
	}
	return credentials.User, nil
}

const credentialSelect = "SELECT id,username,password_hash,salt,is_admin,created_at,token_generation,email,auth_kind,EXISTS(SELECT 1 FROM user_identities WHERE user_id=users.id) FROM users"
