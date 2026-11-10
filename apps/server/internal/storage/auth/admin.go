package auth

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"errors"
	"log/slog"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
)

var ErrAdminInvitePolicy = errors.New("Invite code expires_at must be in the next 365 days and max_uses must be between 1 and 1000")

// AdminUserInfo supplies account counts without credential material.
type AdminUserInfo struct {
	Email         *string     `json:"email"`
	AuthKind      string      `json:"auth_kind"`
	IsOidcLinked  bool        `json:"is_oidc_linked"`
	Id            identity.Id `json:"id"`
	Username      string      `json:"username"`
	IsAdmin       bool        `json:"is_admin"`
	CreatedAt     float64     `json:"created_at"`
	UpdatedAt     float64     `json:"updated_at"`
	FolderCount   int64       `json:"folder_count"`
	FavoriteCount int64       `json:"favorite_count"`
	NotifyEnabled bool        `json:"notify_enabled"`
}

// AdminInviteInfo exposes authorized administrator lifecycle and first-redemption metadata.
type AdminInviteInfo struct {
	Id            int64        `json:"id"`
	Code          string       `json:"code"`
	CreatedBy     *identity.Id `json:"created_by"`
	CreatedByName *string      `json:"created_by_name"`
	UsedBy        *identity.Id `json:"used_by"`
	UsedByName    *string      `json:"used_by_name"`
	UsedAt        *float64     `json:"used_at"`
	Status        string       `json:"status"`
	ExpiresAt     float64      `json:"expires_at"`
	RevokedAt     *float64     `json:"revoked_at"`
	MaxUses       int64        `json:"max_uses"`
	UseCount      int64        `json:"use_count"`
	CreatedAt     float64      `json:"created_at"`
}

func (invite AdminInviteInfo) String() string       { return "AdminInviteInfo([REDACTED])" }
func (invite AdminInviteInfo) GoString() string     { return invite.String() }
func (invite AdminInviteInfo) LogValue() slog.Value { return slog.StringValue(invite.String()) }

// ListUsers returns administrator dashboard rows in stable identifier order.
func (repository *Repository) ListUsers(ctx context.Context) ([]AdminUserInfo, error) {
	rows, err := queries.Users(ctx, repository.database)
	if err != nil {
		return nil, err
	}
	result := make([]AdminUserInfo, 0, len(rows))
	for _, row := range rows {
		result = append(result, AdminUserInfo{Id: identity.Id(row.ID), Username: row.Username, Email: row.Email, AuthKind: row.AuthKind, IsOidcLinked: row.Exists, IsAdmin: row.IsAdmin != 0, CreatedAt: row.CreatedAt, UpdatedAt: row.UpdatedAt, FolderCount: row.Count, FavoriteCount: row.Count_2, NotifyEnabled: row.Count_3 != 0})
	}
	return result, nil
}

// ListInvites retains all administrator-visible lifecycle states, including deleted creators.
func (repository *Repository) ListInvites(ctx context.Context, now float64) ([]AdminInviteInfo, error) {
	rows, err := queries.Invites(ctx, repository.database)
	if err != nil {
		return nil, err
	}
	result := make([]AdminInviteInfo, 0, len(rows))
	for _, row := range rows {
		item := AdminInviteInfo{Id: row.ID, Code: row.Code, CreatedBy: (*identity.Id)(row.CreatedBy), UsedBy: (*identity.Id)(row.UsedBy), UsedAt: row.UsedAt, CreatedAt: row.CreatedAt, CreatedByName: row.Username, UsedByName: row.Username_2, ExpiresAt: row.ExpiresAt, RevokedAt: row.RevokedAt, MaxUses: row.MaxUses, UseCount: row.UseCount}
		item.Status = (domain.InviteRow{ExpiresAt: item.ExpiresAt, RevokedAt: item.RevokedAt, MaxUses: item.MaxUses, UseCount: item.UseCount}).Response(now).Status
		result = append(result, item)
	}
	return result, nil
}

// CreateAdministratorInvite issues an ownerless invite after checking the actor under the write lock.
// A nil actor retains the trusted local administration entry point.
func (repository *Repository) CreateAdministratorInvite(ctx context.Context, actor *identity.Id, expiresAt *float64, maxUses *int64, audit *domain.AuditEvent) (AdminInviteInfo, error) {
	var random [8]byte
	if _, err := rand.Read(random[:]); err != nil {
		return AdminInviteInfo{}, domain.ErrEntropy
	}
	code := hex.EncodeToString(random[:])
	clear(random[:])
	var invite AdminInviteInfo
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if actor != nil {
			if err := RequireAdministrator(ctx, connection, *actor); err != nil {
				return err
			}
		}
		now := float64(time.Now().UnixNano()) / 1e9
		expires := now + 604800
		uses := int64(1)
		if expiresAt != nil {
			expires = *expiresAt
		}
		if maxUses != nil {
			uses = *maxUses
		}
		if !domain.ValidInvitePolicy(now, expires, uses) {
			return ErrAdminInvitePolicy
		}
		result, err := queries.CreateAdministratorInvite(ctx, connection, authdb.CreateAdministratorInviteParams{Code: code, CreatedAt: now, ExpiresAt: expires, MaxUses: uses})
		if err != nil {
			return err
		}
		id, err := result.LastInsertId()
		if err != nil {
			return err
		}
		invite = AdminInviteInfo{Id: id, Code: code, Status: "active", ExpiresAt: expires, MaxUses: uses, CreatedAt: now}
		if audit != nil {
			copy := audit.WithTarget(id)
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	if err != nil {
		return AdminInviteInfo{}, err
	}
	return invite, nil
}

// RevokeAdministratorInvite revokes an arbitrary invite only while the optional actor remains authorized.
func (repository *Repository) RevokeAdministratorInvite(ctx context.Context, actor *identity.Id, inviteId int64, audit *domain.AuditEvent) (bool, error) {
	var revoked bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if actor != nil {
			if err := RequireAdministrator(ctx, connection, *actor); err != nil {
				return err
			}
		}
		result, err := queries.RevokeAdministratorInvite(ctx, connection, authdb.RevokeAdministratorInviteParams{MAX: float64(time.Now().UnixNano()) / 1e9, ID: inviteId})
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		revoked = count > 0
		if revoked && audit != nil {
			copy := audit.WithTarget(inviteId)
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	return revoked && err == nil, err
}
