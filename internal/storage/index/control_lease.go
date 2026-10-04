package index

import (
	"context"
	"database/sql"
	"errors"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// AcquireLease claims an absent, expired or same-owner provider lease for five minutes.
func AcquireLease(ctx context.Context, connection *sql.Conn, catalog, provider, run string, now int64) error {
	return immediate(ctx, connection, func() error {
		var owner sqlite.Text
		var expires sqlite.Integer
		err := connection.QueryRowContext(ctx, "SELECT run_id,expires_at FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2", catalog, provider).Scan(&owner, &expires)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if err == nil && string(owner) != run && int64(expires) > now {
			return &ControlError{Kind: "active_lease", RunId: string(owner), ExpiresAt: int64(expires)}
		}
		_, err = connection.ExecContext(ctx, `INSERT INTO provider_leases(catalog_name,provider_name,run_id,heartbeat_at,expires_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(catalog_name,provider_name) DO UPDATE SET run_id=excluded.run_id,heartbeat_at=excluded.heartbeat_at,expires_at=excluded.expires_at`, catalog, provider, run, now, now+300)
		return err
	})
}

// HeartbeatLease renews only the exact owner while its lease has not expired.
func HeartbeatLease(ctx context.Context, connection *sql.Conn, catalog, provider, run string, now int64) error {
	result, err := connection.ExecContext(ctx, `UPDATE provider_leases SET heartbeat_at=?4,expires_at=?5 WHERE catalog_name=?1 AND provider_name=?2 AND run_id=?3 AND expires_at>?4`, catalog, provider, run, now, now+300)
	return requireLeaseChange(result, err, run)
}

// ReleaseLease removes the exact owner's lease even after its expiry.
func ReleaseLease(ctx context.Context, connection *sql.Conn, catalog, provider, run string) error {
	result, err := connection.ExecContext(ctx, "DELETE FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2 AND run_id=?3", catalog, provider, run)
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
		return &ControlError{Kind: "ownership_lost", RunId: run}
	}
	return nil
}
