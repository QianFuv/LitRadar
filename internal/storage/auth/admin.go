package auth

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"errors"
	"log/slog"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
)

var ErrAdminInvitePolicy = errors.New("Invite code expires_at must be in the next 365 days and max_uses must be between 1 and 1000")

// AdminUserInfo supplies account counts without credential material.
type AdminUserInfo struct {
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
	rows, err := repository.database.QueryContext(ctx, `SELECT u.id,u.username,u.is_admin,u.created_at,u.updated_at,(SELECT COUNT(*) FROM folders f WHERE f.user_id=u.id),(SELECT COUNT(*) FROM favorites fv WHERE fv.user_id=u.id),(SELECT COUNT(*) FROM notification_settings ns WHERE ns.user_id=u.id AND ns.enabled=1) FROM users u ORDER BY u.id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []AdminUserInfo{}
	for rows.Next() {
		var item AdminUserInfo
		var isAdmin, notifyEnabled int64
		if err := rows.Scan(&item.Id, &item.Username, &isAdmin, &item.CreatedAt, &item.UpdatedAt, &item.FolderCount, &item.FavoriteCount, &notifyEnabled); err != nil {
			return nil, err
		}
		item.IsAdmin = isAdmin != 0
		item.NotifyEnabled = notifyEnabled != 0
		result = append(result, item)
	}
	return result, rows.Err()
}

// ListInvites retains all administrator-visible lifecycle states, including deleted creators.
func (repository *Repository) ListInvites(ctx context.Context, now float64) ([]AdminInviteInfo, error) {
	rows, err := repository.database.QueryContext(ctx, `SELECT ic.id,ic.code,ic.created_by,ic.used_by,ic.used_at,ic.created_at,uc.username,uu.username,ic.expires_at,ic.revoked_at,ic.max_uses,ic.use_count FROM invite_codes ic LEFT JOIN users uc ON ic.created_by=uc.id LEFT JOIN users uu ON ic.used_by=uu.id ORDER BY ic.created_at DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []AdminInviteInfo{}
	for rows.Next() {
		var item AdminInviteInfo
		if err := rows.Scan(&item.Id, &item.Code, &item.CreatedBy, &item.UsedBy, &item.UsedAt, &item.CreatedAt, &item.CreatedByName, &item.UsedByName, &item.ExpiresAt, &item.RevokedAt, &item.MaxUses, &item.UseCount); err != nil {
			return nil, err
		}
		item.Status = (domain.InviteRow{ExpiresAt: item.ExpiresAt, RevokedAt: item.RevokedAt, MaxUses: item.MaxUses, UseCount: item.UseCount}).Response(now).Status
		result = append(result, item)
	}
	return result, rows.Err()
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
		result, err := connection.ExecContext(ctx, "INSERT INTO invite_codes(code,created_by,created_at,expires_at,max_uses,use_count) VALUES(?,NULL,?,?,?,0)", code, now, expires, uses)
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
		result, err := connection.ExecContext(ctx, "UPDATE invite_codes SET revoked_at=MAX(?,created_at) WHERE id=? AND revoked_at IS NULL", float64(time.Now().UnixNano())/1e9, inviteId)
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
