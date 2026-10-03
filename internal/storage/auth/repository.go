// Package auth persists identities with transaction-local authorization and completion audits.
package auth

import (
	"context"
	"database/sql"
	"errors"
	"math"
	"os"
	"path/filepath"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	platformsqlite "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	sqlite3 "github.com/mattn/go-sqlite3"
)

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
func (repository *Repository) WithConnection(ctx context.Context, read func(*sql.Conn) error) error {
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	return read(connection)
}

// Immediate keeps the entire protected mutation and required audit on one connection.
// The bounded revocation policy is restored before returning a connection to the pool.
func (repository *Repository) Immediate(ctx context.Context, isRevocation bool, mutate func(*sql.Conn) error) error {
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	if isRevocation {
		if _, err := connection.ExecContext(ctx, "PRAGMA busy_timeout=250"); err != nil {
			return err
		}
		defer connection.ExecContext(context.Background(), "PRAGMA busy_timeout=30000")
	}
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	if err := mutate(connection); err != nil {
		return err
	}
	_, err = connection.ExecContext(ctx, "COMMIT")
	return err
}

// InsertAudit fails closed and deliberately excludes SQL details from the public error.
func InsertAudit(ctx context.Context, connection *sql.Conn, event *domain.AuditEvent) error {
	err := insertAudit(ctx, connection, event)
	if err != nil {
		recordAuditError(err)
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
	_, err := connection.ExecContext(ctx, `INSERT INTO security_audit_events(actor_id,target_id,action,outcome,reason,request_id,source_class,bucket,rejected_count,retry_after_seconds,occurred_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)`, event.ActorId, event.TargetId, event.Action, event.Outcome, event.Reason, event.RequestId, event.SourceClass, event.Bucket, event.RejectedCount, event.RetryAfterSeconds, event.OccurredAt)
	if err != nil {
		return AuditFailure{cause: err}
	}
	return nil
}

func validAudit(event *domain.AuditEvent) bool {
	validSymbol := func(value string, allowEmpty bool) bool {
		if (!allowEmpty && value == "") || len(value) > 64 {
			return false
		}
		for _, character := range []byte(value) {
			if !(character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || character == '_') {
				return false
			}
		}
		return true
	}
	if !validSymbol(event.Action, false) || !validSymbol(event.Outcome, false) || !validSymbol(event.Reason, true) || !validSymbol(event.SourceClass, true) || !validSymbol(event.Bucket, true) || event.ActorId != nil && *event.ActorId <= 0 || event.TargetId != nil && *event.TargetId <= 0 || math.IsNaN(event.OccurredAt) || math.IsInf(event.OccurredAt, 0) || len(event.RequestId) > 128 {
		return false
	}
	for _, character := range []byte(event.RequestId) {
		if !(character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || character == '-' || character == '_' || character == '.' || character == ':') {
			return false
		}
	}
	return true
}

// RequireAdministrator rechecks live authority while the caller owns the write transaction.
func RequireAdministrator(ctx context.Context, connection *sql.Conn, actor identity.Id) error {
	var isAdmin int64
	err := connection.QueryRowContext(ctx, "SELECT is_admin FROM users WHERE id=?", actor).Scan(&isAdmin)
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
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM users").Scan(&count); err != nil {
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

// Register preserves bootstrap, missing invite, duplicate username and redemption error priority.
func (repository *Repository) Register(ctx context.Context, username, passwordHash, salt string, invite *string, now float64, audit *domain.AuditEvent) (domain.User, error) {
	var user domain.User
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var count int64
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM users").Scan(&count); err != nil {
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
		err = connection.QueryRowContext(ctx, `UPDATE invite_codes SET used_by=CASE WHEN use_count=0 THEN ?1 ELSE used_by END,used_at=CASE WHEN use_count=0 THEN ?2 ELSE used_at END,use_count=use_count+1 WHERE code=?3 AND revoked_at IS NULL AND expires_at>?2 AND use_count<max_uses RETURNING id`, user.Id, now, *invite).Scan(&inviteId)
		if errors.Is(err, sql.ErrNoRows) {
			return domain.ErrInvite
		}
		if err != nil {
			return err
		}
		if _, err := connection.ExecContext(ctx, "INSERT INTO invite_code_uses(invite_code_id,user_id,used_at) VALUES(?,?,?)", inviteId, user.Id, now); err != nil {
			return err
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
	result, err := connection.ExecContext(ctx, "INSERT INTO users(username,password_hash,salt,is_admin,created_at,updated_at) VALUES(?,?,?,?,?,?)", username, passwordHash, salt, isAdmin, now, now)
	if err != nil {
		var sqliteError sqlite3.Error
		if errors.As(err, &sqliteError) && sqliteError.Code == sqlite3.ErrConstraint {
			return domain.User{}, domain.ErrUsernameExists
		}
		return domain.User{}, err
	}
	id, err := result.LastInsertId()
	return domain.User{Id: identity.Id(id), Username: username, IsAdmin: isAdmin}, err
}

func defaultFolder(ctx context.Context, connection *sql.Conn, user identity.Id, now float64) error {
	_, err := connection.ExecContext(ctx, "INSERT INTO folders(user_id,name,is_tracking,created_at,updated_at) VALUES(?,?,1,?,?)", user, "默认收藏", now, now)
	return err
}

func scanCredentials(row *sql.Row) (*domain.Credentials, error) {
	var credentials domain.Credentials
	var isAdmin int64
	err := row.Scan(&credentials.User.Id, &credentials.User.Username, &credentials.PasswordHash, &credentials.Salt, &isAdmin, &credentials.CreatedAt, &credentials.TokenGeneration)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	credentials.User.IsAdmin = isAdmin != 0
	return &credentials, nil
}

// CredentialsByName preserves SQLite's existing NOCASE username comparison.
func (repository *Repository) CredentialsByName(ctx context.Context, username string) (*domain.Credentials, error) {
	return scanCredentials(repository.database.QueryRowContext(ctx, "SELECT id,username,password_hash,salt,is_admin,created_at,token_generation FROM users WHERE username=?", username))
}

// CredentialsById loads the current credential generation for post-work race checks.
func (repository *Repository) CredentialsById(ctx context.Context, user identity.Id) (*domain.Credentials, error) {
	return scanCredentials(repository.database.QueryRowContext(ctx, "SELECT id,username,password_hash,salt,is_admin,created_at,token_generation FROM users WHERE id=?", user))
}

// BootstrapRequired means there are no users, including non-administrator users.
func (repository *Repository) BootstrapRequired(ctx context.Context) (bool, error) {
	var count int64
	err := repository.database.QueryRowContext(ctx, "SELECT count(*) FROM users").Scan(&count)
	return count == 0, err
}

// SetAdministrator preserves at least one administrator without revoking existing credentials.
func (repository *Repository) SetAdministrator(ctx context.Context, actor, target identity.Id, isAdmin bool, audit *domain.AuditEvent) error {
	return repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := checkAdminTarget(ctx, connection, actor, target, !isAdmin); err != nil {
			return err
		}
		result, err := connection.ExecContext(ctx, "UPDATE users SET is_admin=?,updated_at=? WHERE id=?", isAdmin, float64(time.Now().UnixNano())/1e9, target)
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
		if _, err := connection.ExecContext(ctx, "UPDATE invite_codes SET revoked_at=MAX(?,created_at) WHERE created_by=? AND revoked_at IS NULL", float64(time.Now().UnixNano())/1e9, target); err != nil {
			return err
		}
		result, err := connection.ExecContext(ctx, "DELETE FROM users WHERE id=?", target)
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
	err := connection.QueryRowContext(ctx, "SELECT is_admin FROM users WHERE id=?", target).Scan(&wasAdmin)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrUserNotFound
	}
	if err != nil {
		return err
	}
	if canRemoveAdmin && wasAdmin != 0 {
		var count int64
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM users WHERE is_admin=1").Scan(&count); err != nil {
			return err
		}
		if count <= 1 {
			return ErrLastAdministrator
		}
	}
	return nil
}
