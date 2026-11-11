package index

import (
	"context"
	"database/sql"
	"errors"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/controldb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// AcquireLease claims an absent, expired or same-owner provider lease for five minutes.
func AcquireLease(ctx context.Context, connection *sql.Conn, catalog, provider, run string, now int64) error {
	return immediate(ctx, connection, func() error {
		var owner sqlite.Text
		var expires sqlite.Integer
		err := func() error {
			queryResult, err := controlQueries.ProviderLease(ctx, connection, controldb.ProviderLeaseParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider)})
			if err != nil {
				return err
			}
			owner = queryResult.RunID
			expires = queryResult.ExpiresAt
			return nil
		}()
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if err == nil && string(owner) != run && int64(expires) > now {
			return &ControlError{Kind: "active_lease", RunID: string(owner), ExpiresAt: int64(expires)}
		}
		_, err = controlQueries.AcquireLease(ctx, connection, controldb.AcquireLeaseParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider), RunID: sqlite.Text(run), HeartbeatAt: sqlite.Integer(now), ExpiresAt: sqlite.Integer(now + 300)})
		return err
	})
}

// HeartbeatLease renews only the exact owner while its lease has not expired.
func HeartbeatLease(ctx context.Context, connection *sql.Conn, catalog, provider, run string, now int64) error {
	result, err := controlQueries.HeartbeatLease(ctx, connection, controldb.HeartbeatLeaseParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider), RunID: sqlite.Text(run), HeartbeatAt: sqlite.Integer(now), ExpiresAt: sqlite.Integer(now + 300)})
	return requireLeaseChange(result, err, run)
}

// ReleaseLease removes the exact owner's lease even after its expiry.
func ReleaseLease(ctx context.Context, connection *sql.Conn, catalog, provider, run string) error {
	result, err := controlQueries.ReleaseLease(ctx, connection, controldb.ReleaseLeaseParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider), RunID: sqlite.Text(run)})
	return requireLeaseChange(result, err, run)
}

func requireLeaseChange(result sql.Result, err error, run string) error {
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if count == 0 {
		return &ControlError{Kind: "ownership_lost", RunID: run}
	}
	return nil
}
