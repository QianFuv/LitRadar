package auth

import (
	"context"
	"database/sql"
	"errors"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
	sqlite3 "modernc.org/sqlite/lib"
)

func updateEmail(ctx context.Context, connection *sql.Conn, user identity.Id, email *string, now float64) error {
	_, err := queries.UpdateEmail(ctx, connection, authdb.UpdateEmailParams{Email: email, UpdatedAt: now, ID: int64(user)})
	var failure interface{ Code() int }
	if errors.As(err, &failure) && failure.Code() == sqlite3.SQLITE_CONSTRAINT_UNIQUE {
		return domain.ErrEmailExists
	}
	return err
}

// UpdateEmail changes only local accounts and commits the audit with the mutation.
func (repository *Repository) UpdateEmail(ctx context.Context, user identity.Id, email *string, now float64, audit *domain.AuditEvent) (domain.User, error) {
	var credentials *domain.Credentials
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var err error
		credentials, err = decodeCredentials(queries.CredentialsByID(ctx, connection, authdb.CredentialsByIDParams{ID: int64(user)}))
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
