package auth

import (
	"context"
	"database/sql"
	"errors"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
)

// IssueInvite requires explicit rotation even when the previous unrevoked invite is expired or exhausted.
func (repository *Repository) IssueInvite(ctx context.Context, user identity.Id, code string, now, expires float64, maxUses int64, isRotation bool, audit *domain.AuditEvent) (domain.InviteRow, error) {
	if !domain.ValidInvitePolicy(now, expires, maxUses) {
		return domain.InviteRow{}, domain.ErrInvitePolicy
	}
	var invite domain.InviteRow
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var previous int64
		lookup := connection.QueryRowContext(ctx, "SELECT id FROM invite_codes WHERE created_by=? AND revoked_at IS NULL", user).Scan(&previous)
		if lookup != nil && !errors.Is(lookup, sql.ErrNoRows) {
			return lookup
		}
		if lookup == nil {
			if !isRotation {
				return domain.ErrActiveInvite
			}
			if _, err := connection.ExecContext(ctx, "UPDATE invite_codes SET revoked_at=MAX(?,created_at) WHERE created_by=? AND revoked_at IS NULL", now, user); err != nil {
				return err
			}
		}
		result, err := connection.ExecContext(ctx, "INSERT INTO invite_codes(code,created_by,created_at,expires_at,max_uses,use_count) VALUES(?,?,?,?,?,0)", code, user, now, expires, maxUses)
		if err != nil {
			return err
		}
		id, err := result.LastInsertId()
		if err != nil {
			return err
		}
		invite = domain.InviteRow{Id: id, Code: code, ExpiresAt: expires, MaxUses: maxUses, CreatedAt: now}
		if audit != nil {
			if lookup == nil && isRotation {
				copy := audit.WithTarget(previous)
				if err := InsertAudit(ctx, connection, &copy); err != nil {
					return err
				}
			}
			copy := audit.WithTarget(id)
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	if err != nil {
		return domain.InviteRow{}, err
	}
	return invite, nil
}

// RevokeInvite marks the creator's current invite irreversibly revoked and audits an actual transition.
func (repository *Repository) RevokeInvite(ctx context.Context, user identity.Id, now float64, audit *domain.AuditEvent) (bool, error) {
	var revoked bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var id int64
		err := connection.QueryRowContext(ctx, "UPDATE invite_codes SET revoked_at=MAX(?,created_at) WHERE created_by=? AND revoked_at IS NULL RETURNING id", now, user).Scan(&id)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return err
		}
		revoked = true
		if audit != nil {
			copy := audit.WithTarget(id)
			return InsertAudit(ctx, connection, &copy)
		}
		return nil
	})
	return revoked && err == nil, err
}

// Invite returns the latest issuance, including expired, exhausted or revoked history.
func (repository *Repository) Invite(ctx context.Context, user identity.Id) (*domain.InviteRow, error) {
	var invite domain.InviteRow
	err := repository.database.QueryRowContext(ctx, "SELECT id,code,used_by,used_at,expires_at,revoked_at,max_uses,use_count,created_at FROM invite_codes WHERE created_by=? ORDER BY created_at DESC,id DESC LIMIT 1", user).Scan(&invite.Id, &invite.Code, &invite.UsedBy, &invite.UsedAt, &invite.ExpiresAt, &invite.RevokedAt, &invite.MaxUses, &invite.UseCount, &invite.CreatedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &invite, nil
}
