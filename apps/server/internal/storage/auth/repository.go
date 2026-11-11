// Package auth persists identities with transaction-local authorization and completion audits.
package auth

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"os"
	"path/filepath"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	platformsqlite "github.com/QianFuv/LitRadar/apps/server/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	sqlite3 "modernc.org/sqlite/lib"
)

var queries authdb.Queries

var ErrUserNotFound = errors.New("User not found")
var ErrLastAdministrator = errors.New("At least one administrator is required")

// AuditFailure preserves retry classification without exposing SQLite details in ordinary messages.
type AuditFailure struct{ cause error }

func (failure AuditFailure) Error() string        { return domain.ErrAudit.Error() }
func (failure AuditFailure) Unwrap() error        { return failure.cause }
func (failure AuditFailure) Is(target error) bool { return target == domain.ErrAudit }
func (failure AuditFailure) GoString() string     { return failure.Error() }

// Repository owns a pool; opening or reading it never runs schema migrations.
type Repository struct{ database *sql.DB }

// Open initializes ordinary connection policy after callers explicitly complete migrations.
func Open(filename string) (*Repository, error) {
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		return nil, err
	}
	database, err := platformsqlite.Open(platformsqlite.Config{Filename: filename, Mode: "rwc", MaxConnections: 8})
	if err != nil {
		return nil, err
	}
	return &Repository{database}, nil
}

// Close releases the repository's physical connections.
func (repository *Repository) Close() error { return repository.database.Close() }

// WithConnection runs an explicit non-transactional read using the auth pool's connection policy.
func (repository *Repository) WithConnection(ctx context.Context, read func(*sql.Conn) error) (resultError error) {
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := connection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	return read(connection)
}

// Immediate keeps the entire protected mutation and required audit on one connection.
// The bounded revocation policy is restored before returning a connection to the pool.
func (repository *Repository) Immediate(ctx context.Context, isRevocation bool, mutate func(*sql.Conn) error) (resultError error) {
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return err
	}
	defer func() {
		if closeError := connection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	if isRevocation {
		if _, err := connection.ExecContext(ctx, "PRAGMA busy_timeout=250"); err != nil {
			return err
		}
		defer func() {
			if _, err := connection.ExecContext(context.Background(), "PRAGMA busy_timeout=30000"); err != nil {
				slog.Error("storage.busy_timeout.restore_failed", "error", err)
			}
		}()
	}
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return err
	}
	hasTransaction := true
	defer func() {
		if hasTransaction {
			if _, rollbackError := connection.ExecContext(context.Background(), "ROLLBACK"); rollbackError != nil {
				slog.Error("storage.rollback.failed", "error", rollbackError)
			}
		}
	}()
	if err := mutate(connection); err != nil {
		return err
	}
	_, err = connection.ExecContext(ctx, "COMMIT")
	hasTransaction = err != nil
	return err
}

// InsertAudit fails closed and deliberately excludes SQL details from the public error.
func InsertAudit(ctx context.Context, connection *sql.Conn, event *domain.AuditEvent) error {
	err := insertAudit(ctx, connection, event)
	if err != nil {
		recordAuditError(ctx, err)
	}
	return err
}

func insertAudit(ctx context.Context, connection *sql.Conn, event *domain.AuditEvent) error {
	if event == nil {
		return nil
	}
	if !validAudit(event) {
		return AuditFailure{cause: ErrAuditEvent}
	}
	_, err := queries.InsertAudit(ctx, connection, authdb.InsertAuditParams{ActorID: event.ActorId, TargetID: event.TargetId, Action: event.Action, Outcome: event.Outcome, Reason: event.Reason, RequestID: event.RequestId, SourceClass: event.SourceClass, Bucket: event.Bucket, RejectedCount: event.RejectedCount, RetryAfterSeconds: event.RetryAfterSeconds, OccurredAt: event.OccurredAt})
	if err != nil {
		return AuditFailure{cause: err}
	}
	return nil
}

// validAudit checks symbols, scalar metadata and request bytes in their original order.
func validAudit(event *domain.AuditEvent) bool {
	return validAuditSymbols(event) && validAuditScalars(event) && validAuditRequestId(event.RequestId)
}

// validAuditSymbols keeps required and optional fixed metadata distinct.
func validAuditSymbols(event *domain.AuditEvent) bool {
	return validAuditSymbol(event.Action, false) && validAuditSymbol(event.Outcome, false) && validAuditSymbol(event.Reason, true) && validAuditSymbol(event.SourceClass, true) && validAuditSymbol(event.Bucket, true)
}

// validAuditSymbol admits lowercase ASCII symbols without trimming or enum restrictions.
func validAuditSymbol(value string, shouldAllowEmpty bool) bool {
	if (!shouldAllowEmpty && value == "") || len(value) > 64 {
		return false
	}
	for _, character := range []byte(value) {
		if !(character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || character == '_') {
			return false
		}
	}
	return true
}

// validAuditScalars retains optional positive identities and finite unrestricted timestamps.
func validAuditScalars(event *domain.AuditEvent) bool {
	return validAuditIdentity(event.ActorId) && validAuditIdentity(event.TargetId) && !math.IsNaN(event.OccurredAt) && !math.IsInf(event.OccurredAt, 0) && len(event.RequestId) <= 128
}

// validAuditIdentity allows an omitted identity but requires a positive supplied value.
func validAuditIdentity(value *int64) bool {
	return value == nil || *value > 0
}

// validAuditRequestId permits only the original request correlation alphabet.
func validAuditRequestId(value string) bool {
	for _, character := range []byte(value) {
		if !isAuditRequestIdByte(character) {
			return false
		}
	}
	return true
}

// isAuditRequestIdByte recognizes case-sensitive ASCII request metadata without normalization.
func isAuditRequestIdByte(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || character == '-' || character == '_' || character == '.' || character == ':'
}

// RequireAdministrator rechecks live authority while the caller owns the write transaction.
func RequireAdministrator(ctx context.Context, connection *sql.Conn, actor identity.Id) error {
	var isAdmin int64
	err := func() error {
		row, err := queries.Administrator(ctx, connection, authdb.AdministratorParams{ID: int64(actor)})
		if err != nil {
			return err
		}
		isAdmin = row
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) || err == nil && isAdmin == 0 {
		return domain.ErrAdminForbidden
	}
	return err
}

// Bootstrap creates exactly one first administrator, default folder and optional completion audit.
func (repository *Repository) Bootstrap(ctx context.Context, username, passwordHash, salt string, now float64, audit *domain.AuditEvent) (domain.User, error) {
	var user domain.User
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var count int64
		if err := func() error {
			row, err := queries.UserCount(ctx, connection)
			if err != nil {
				return err
			}
			count = row
			return nil
		}(); err != nil {
			return err
		}
		if count != 0 {
			return domain.ErrBootstrapComplete
		}
		var err error
		user, err = insertUser(ctx, connection, username, passwordHash, salt, true, now)
		if err != nil {
			return err
		}
		if err := defaultFolder(ctx, connection, user.Id, now); err != nil {
			return err
		}
		if audit != nil {
			copy := audit.WithIdentity(int64(user.Id), int64(user.Id))
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	if err != nil {
		return domain.User{}, err
	}
	return user, nil
}

// Register preserves bootstrap, missing invite, duplicate username, redemption and duplicate email error priority.
func (repository *Repository) Register(ctx context.Context, username, passwordHash, salt string, invite *string, now float64, audit *domain.AuditEvent, email *string) (domain.User, error) {
	var user domain.User
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var count int64
		if err := func() error {
			row, err := queries.UserCount(ctx, connection)
			if err != nil {
				return err
			}
			count = row
			return nil
		}(); err != nil {
			return err
		}
		if count == 0 {
			return domain.ErrBootstrapRequired
		}
		if invite == nil {
			return domain.ErrInviteRequired
		}
		var err error
		user, err = insertUser(ctx, connection, username, passwordHash, salt, false, now)
		if err != nil {
			return err
		}
		var inviteId int64
		err = func() error {
			row, err := queries.RedeemInvite(ctx, connection, authdb.RedeemInviteParams{UsedBy: new(int64(user.Id)), UsedAt: new(now), Code: *invite})
			if err != nil {
				return err
			}
			inviteId = row
			return nil
		}()
		if errors.Is(err, sql.ErrNoRows) {
			return domain.ErrInvite
		}
		if err != nil {
			return err
		}
		if _, err := queries.RecordInviteUse(ctx, connection, authdb.RecordInviteUseParams{InviteCodeID: inviteId, UserID: new(int64(user.Id)), UsedAt: now}); err != nil {
			return err
		}
		if email != nil {
			if err := updateEmail(ctx, connection, user.Id, email, now); err != nil {
				return err
			}
			user.Email = email
		}
		redemption := (domain.AuditEvent{Action: "invite_redeem", Outcome: "completed", OccurredAt: now}).WithIdentity(int64(user.Id), inviteId)
		if audit != nil {
			redemption.RequestId = audit.RequestId
		}
		if err := InsertAudit(ctx, connection, &redemption); err != nil {
			return err
		}
		if err := defaultFolder(ctx, connection, user.Id, now); err != nil {
			return err
		}
		if audit != nil {
			copy := audit.WithIdentity(int64(user.Id), int64(user.Id))
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	if err != nil {
		return domain.User{}, err
	}
	return user, nil
}

func insertUser(ctx context.Context, connection *sql.Conn, username, passwordHash, salt string, isAdmin bool, now float64) (domain.User, error) {
	result, err := queries.CreateUser(ctx, connection, authdb.CreateUserParams{Username: username, PasswordHash: passwordHash, Salt: salt, IsAdmin: dbtypes.Boolean(isAdmin), CreatedAt: now, UpdatedAt: now})
	if err != nil {
		var sqliteError interface{ Code() int }
		if errors.As(err, &sqliteError) && sqliteError.Code()&255 == sqlite3.SQLITE_CONSTRAINT {
			return domain.User{}, domain.ErrUsernameExists
		}
		return domain.User{}, err
	}
	id, err := result.LastInsertId()
	return domain.User{Id: identity.Id(id), Username: username, IsAdmin: isAdmin, AuthKind: "local"}, err
}

func defaultFolder(ctx context.Context, connection *sql.Conn, user identity.Id, now float64) error {
	_, err := queries.CreateDefaultFolder(ctx, connection, authdb.CreateDefaultFolderParams{UserID: int64(user), Name: "默认收藏", CreatedAt: now, UpdatedAt: now})
	return err
}

func decodeCredentials(row authdb.CredentialsByIDRow, err error) (*domain.Credentials, error) {
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &domain.Credentials{User: domain.User{Id: identity.Id(row.ID), Username: row.Username, IsAdmin: row.IsAdmin != 0, Email: row.Email, AuthKind: row.AuthKind, IsOidcLinked: row.Exists}, PasswordHash: row.PasswordHash, Salt: row.Salt, CreatedAt: row.CreatedAt, TokenGeneration: row.TokenGeneration}, nil
}

// CredentialsByName preserves SQLite's existing NOCASE username comparison.
func (repository *Repository) CredentialsByName(ctx context.Context, username string) (*domain.Credentials, error) {
	if strings.Contains(username, "@") {
		row, err := queries.CredentialsByEmail(ctx, repository.database, authdb.CredentialsByEmailParams{Email: new(strings.ToLower(strings.TrimSpace(username)))})
		return decodeCredentials(authdb.CredentialsByIDRow(row), err)
	}
	row, err := queries.CredentialsByName(ctx, repository.database, authdb.CredentialsByNameParams{Username: username})
	return decodeCredentials(authdb.CredentialsByIDRow(row), err)
}

// CredentialsById loads the current credential generation for post-work race checks.
func (repository *Repository) CredentialsById(ctx context.Context, user identity.Id) (*domain.Credentials, error) {
	return decodeCredentials(queries.CredentialsByID(ctx, repository.database, authdb.CredentialsByIDParams{ID: int64(user)}))
}

// BootstrapRequired stops at the first user, including non-administrator users.
func (repository *Repository) BootstrapRequired(ctx context.Context) (bool, error) {
	var hasUsers bool
	err := func() error {
		row, err := queries.HasUsers(ctx, repository.database)
		if err != nil {
			return err
		}
		hasUsers = row
		return nil
	}()
	return !hasUsers, err
}

// SetAdministrator preserves at least one administrator without revoking existing credentials.
func (repository *Repository) SetAdministrator(ctx context.Context, actor, target identity.Id, isAdmin bool, audit *domain.AuditEvent) error {
	return repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := checkAdminTarget(ctx, connection, actor, target, !isAdmin); err != nil {
			return err
		}
		result, err := queries.SetAdministrator(ctx, connection, authdb.SetAdministratorParams{IsAdmin: dbtypes.Boolean(isAdmin), UpdatedAt: float64(time.Now().UnixNano()) / 1e9, ID: int64(target)})
		if err != nil {
			return err
		}
		if count, err := result.RowsAffected(); err != nil {
			return err
		} else if count != 1 {
			return ErrUserNotFound
		}
		if audit != nil {
			copy := audit.WithIdentity(int64(actor), int64(target))
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
}

// DeleteUser first revokes issued invites, then atomically deletes the account and its owned rows.
func (repository *Repository) DeleteUser(ctx context.Context, actor, target identity.Id, audit *domain.AuditEvent) error {
	return repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := checkAdminTarget(ctx, connection, actor, target, true); err != nil {
			return err
		}
		if _, err := queries.RevokeOwnedInvites(ctx, connection, authdb.RevokeOwnedInvitesParams{MAX: float64(time.Now().UnixNano()) / 1e9, CreatedBy: new(int64(target))}); err != nil {
			return err
		}
		result, err := queries.DeleteUser(ctx, connection, authdb.DeleteUserParams{ID: int64(target)})
		if err != nil {
			return err
		}
		if count, err := result.RowsAffected(); err != nil {
			return err
		} else if count != 1 {
			return ErrUserNotFound
		}
		if audit != nil {
			copy := audit.WithIdentity(int64(actor), int64(target))
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
}

func checkAdminTarget(ctx context.Context, connection *sql.Conn, actor, target identity.Id, canRemoveAdmin bool) error {
	if err := RequireAdministrator(ctx, connection, actor); err != nil {
		return err
	}
	var wasAdmin int64
	err := func() error {
		row, err := queries.Administrator(ctx, connection, authdb.AdministratorParams{ID: int64(target)})
		if err != nil {
			return err
		}
		wasAdmin = row
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) {
		return ErrUserNotFound
	}
	if err != nil {
		return err
	}
	if canRemoveAdmin && wasAdmin != 0 {
		var count int64
		if err := func() error {
			row, err := queries.AdministratorCount(ctx, connection)
			if err != nil {
				return err
			}
			count = row
			return nil
		}(); err != nil {
			return err
		}
		if count <= 1 {
			return ErrLastAdministrator
		}
	}
	return nil
}
