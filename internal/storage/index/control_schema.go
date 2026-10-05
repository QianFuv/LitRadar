package index

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"io"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// InitControl migrates supported disposable versions atomically, preserving future versions untouched.
func InitControl(ctx context.Context, connection *sql.Conn) error {
	var version sqlite.Integer
	if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
		return err
	}
	if version > ControlSchemaVersion {
		return &ControlError{Kind: "unsupported_version", FoundVersion: int64(version)}
	}
	if _, err := connection.ExecContext(ctx, "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;"); err != nil {
		return err
	}
	if version == ControlSchemaVersion {
		return nil
	}
	return immediate(ctx, connection, func() error {
		if _, err := connection.ExecContext(ctx, controlLeaseSchema); err != nil {
			return err
		}
		var legacy sqlite.Integer
		if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='provider_checkpoints')").Scan(&legacy); err != nil {
			return err
		}
		if version <= 1 {
			if err := rewriteLegacyProviders(ctx, connection, legacy != 0); err != nil {
				return err
			}
		}
		if _, err := connection.ExecContext(ctx, controlStateSchema); err != nil {
			return err
		}
		for _, field := range []struct{ table, column string }{{"provider_sync_anchors", "completed_batch_id"}, {"provider_run_checkpoints", "batch_id"}} {
			var exists sqlite.Integer
			if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)", field.table, field.column).Scan(&exists); err != nil {
				return err
			}
			if exists == 0 {
				if _, err := connection.ExecContext(ctx, "ALTER TABLE "+field.table+" ADD COLUMN "+field.column+" TEXT CHECK ("+field.column+" IS NULL OR length("+field.column+")>0)"); err != nil {
					return err
				}
			}
		}
		if legacy != 0 {
			if err := migrateLegacyAnchors(ctx, connection); err != nil {
				return err
			}
			if _, err := connection.ExecContext(ctx, "DROP TABLE provider_checkpoints"); err != nil {
				return err
			}
		}
		_, err := connection.ExecContext(ctx, "DELETE FROM provider_leases WHERE provider_name='cnki_oversea'; DELETE FROM provider_sync_anchors WHERE provider_name='cnki_oversea'; DELETE FROM provider_run_checkpoints WHERE provider_name='cnki_oversea'; PRAGMA user_version=5;")
		return err
	})
}

func rewriteLegacyProviders(ctx context.Context, connection *sql.Conn, hasLegacy bool) error {
	for _, names := range [][2]string{{"cnki", "cnki_oversea"}, {"zjlib_cnki", "zjlib"}} {
		if hasLegacy {
			if _, err := connection.ExecContext(ctx, `UPDATE provider_checkpoints SET provider_name=?1 WHERE provider_name=?2 AND NOT EXISTS(SELECT 1 FROM provider_checkpoints AS existing WHERE existing.catalog_name=provider_checkpoints.catalog_name AND existing.provider_name=?1 AND existing.scope_kind=provider_checkpoints.scope_kind AND existing.scope_key=provider_checkpoints.scope_key)`, names[1], names[0]); err != nil {
				return err
			}
			if _, err := connection.ExecContext(ctx, "DELETE FROM provider_checkpoints WHERE provider_name=?1", names[0]); err != nil {
				return err
			}
		}
		if _, err := connection.ExecContext(ctx, `UPDATE provider_leases SET provider_name=?1 WHERE provider_name=?2 AND NOT EXISTS(SELECT 1 FROM provider_leases AS existing WHERE existing.catalog_name=provider_leases.catalog_name AND existing.provider_name=?1)`, names[1], names[0]); err != nil {
			return err
		}
		if _, err := connection.ExecContext(ctx, "DELETE FROM provider_leases WHERE provider_name=?1", names[0]); err != nil {
			return err
		}
	}
	return nil
}

func migrateLegacyAnchors(ctx context.Context, connection *sql.Conn) error {
	rows, err := connection.QueryContext(ctx, "SELECT catalog_name,provider_name,scope_key,checkpoint,updated_at FROM provider_checkpoints WHERE scope_kind='journal'")
	if err != nil {
		return err
	}
	values := [][5]sqlite.Text{}
	for rows.Next() {
		var value [5]sqlite.Text
		if err := rows.Scan(&value[0], &value[1], &value[2], &value[3], &value[4]); err != nil {
			rows.Close()
			return err
		}
		values = append(values, value)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return err
	}
	for _, value := range values {
		if value[0] == "" || value[1] == "" || value[2] == "" || value[4] == "" || !isLegacyComplete(string(value[3])) {
			continue
		}
		if _, err := connection.ExecContext(ctx, `INSERT INTO provider_sync_anchors(catalog_name,provider_name,catalog_id,committed_anchor,completed_at) VALUES(?1,?2,?3,NULL,?4) ON CONFLICT(catalog_name,provider_name,catalog_id) DO UPDATE SET committed_anchor=NULL,completed_at=excluded.completed_at`, value[0], value[1], value[2], value[4]); err != nil {
			return err
		}
	}
	return nil
}

func isLegacyComplete(value string) bool {
	if !jsonvalue.ValidJson(value) {
		return false
	}
	decoder := json.NewDecoder(bytes.NewBufferString(value))
	opening, err := decoder.Token()
	if err != nil || opening != json.Delim('{') {
		return false
	}
	hasState := false
	for decoder.More() {
		key, err := decoder.Token()
		if err != nil {
			return false
		}
		if key == "state" {
			var state string
			if hasState || decoder.Decode(&state) != nil || state != "complete" {
				return false
			}
			hasState = true
		} else {
			var ignored json.RawMessage
			if decoder.Decode(&ignored) != nil {
				return false
			}
		}
	}
	if !hasState {
		return false
	}
	if _, err := decoder.Token(); err != nil {
		return false
	}
	return decoder.Decode(new(any)) == io.EOF
}
