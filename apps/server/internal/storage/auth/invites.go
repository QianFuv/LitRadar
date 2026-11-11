package auth

import (
	"context"
	"database/sql"
	"errors"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
)

// IssueInvite requires explicit rotation even when the previous unrevoked invite is expired or exhausted.
func (repository *Repository) IssueInvite(ctx context.Context, user identity.ID, code string, now, expires float64, maxUses int64, isRotation bool, audit *domain.AuditEvent) (domain.InviteRow, error) {
	if !domain.ValidInvitePolicy(now, expires, maxUses) {
		return domain.InviteRow{}, domain.ErrInvitePolicy
	}
	var invite domain.InviteRow
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var previous int64
		lookup := func() error {
			queryResult, err := queries.ActiveInviteID(ctx, connection, authdb.ActiveInviteIDParams{CreatedBy: new(int64(user))})
			if err != nil {
				return err
			}
			previous = queryResult
			return nil
		}()
		if lookup != nil && !errors.Is(lookup, sql.ErrNoRows) {
			return lookup
		}
		if lookup == nil {
			if !isRotation {
				return domain.ErrActiveInvite
			}
			if _, err := queries.RevokeOwnedInvites(ctx, connection, authdb.RevokeOwnedInvitesParams{MAX: now, CreatedBy: new(int64(user))}); err != nil {
				return err
			}
		}
		result, err := queries.CreateInvite(ctx, connection, authdb.CreateInviteParams{Code: code, CreatedBy: new(int64(user)), CreatedAt: now, ExpiresAt: expires, MaxUses: maxUses})
		if err != nil {
			return err
		}
		id, err := result.LastInsertId()
		if err != nil {
			return err
		}
		invite = domain.InviteRow{ID: id, Code: code, ExpiresAt: expires, MaxUses: maxUses, CreatedAt: now}
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
func (repository *Repository) RevokeInvite(ctx context.Context, user identity.ID, now float64, audit *domain.AuditEvent) (bool, error) {
	var revoked bool
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var id int64
		err := func() error {
			queryResult, err := queries.RevokeInvite(ctx, connection, authdb.RevokeInviteParams{MAX: now, CreatedBy: new(int64(user))})
			if err != nil {
				return err
			}
			id = queryResult
			return nil
		}()
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
func (repository *Repository) Invite(ctx context.Context, user identity.ID) (*domain.InviteRow, error) {
	var invite domain.InviteRow
	err := func() error {
		queryResult, err := queries.LatestInvite(ctx, repository.database, authdb.LatestInviteParams{CreatedBy: new(int64(user))})
		if err != nil {
			return err
		}
		invite.ID = queryResult.ID
		invite.Code = queryResult.Code
		invite.UsedBy = (*identity.ID)(queryResult.UsedBy)
		invite.UsedAt = queryResult.UsedAt
		invite.ExpiresAt = queryResult.ExpiresAt
		invite.RevokedAt = queryResult.RevokedAt
		invite.MaxUses = queryResult.MaxUses
		invite.UseCount = queryResult.UseCount
		invite.CreatedAt = queryResult.CreatedAt
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &invite, nil
}
