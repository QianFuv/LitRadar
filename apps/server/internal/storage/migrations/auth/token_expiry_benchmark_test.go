package auth

import (
	"context"
	"path/filepath"
	"testing"
)

// BenchmarkTokenExpiry measures the same cleanup repeatedly without exhausting expired rows.
func BenchmarkTokenExpiry(benchmark *testing.B) {
	for _, scenario := range []struct {
		name    string
		rows    int
		expired int
	}{
		{"active_1000", 1000, 0},
		{"active_100000", 100000, 0},
		{"sparse_100000", 100000, 100},
		{"dense_100000", 100000, 100000},
	} {
		benchmark.Run(scenario.name, func(benchmark *testing.B) {
			ctx := context.Background()
			filename := filepath.Join(benchmark.TempDir(), "auth.sqlite")
			if _, err := Migrate(ctx, filename); err != nil {
				benchmark.Fatal(err)
			}
			database, err := openMigrationDatabase(filename)
			if err != nil {
				benchmark.Fatal(err)
			}
			defer func() {
				if err := database.Close(); err != nil {
					benchmark.Errorf("database.Close: %v", err)
				}
			}()
			if _, err := database.ExecContext(ctx, `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'synthetic','hash','salt',1,1);
WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM ids WHERE id<?)
INSERT INTO access_tokens(user_id,token_hash,name,expires_at,created_at)
SELECT 1,printf('synthetic-%d',id),'personal',CASE WHEN id<=? THEN 100 ELSE 200 END,1 FROM ids`, scenario.rows, scenario.expired); err != nil {
				benchmark.Fatal(err)
			}
			benchmark.ReportAllocs()
			benchmark.ResetTimer()
			for iteration := 0; iteration < benchmark.N; iteration++ {
				transaction, err := database.BeginTx(ctx, nil)
				if err != nil {
					benchmark.Fatal(err)
				}
				result, err := transaction.ExecContext(ctx, "DELETE FROM access_tokens WHERE expires_at<=?", 100)
				if err != nil {
					if err := transaction.Rollback(); err != nil {
						benchmark.Errorf("transaction.Rollback: %v", err)
					}
					benchmark.Fatal(err)
				}
				count, err := result.RowsAffected()
				if err != nil || count != int64(scenario.expired) {
					if err := transaction.Rollback(); err != nil {
						benchmark.Errorf("transaction.Rollback: %v", err)
					}
					benchmark.Fatalf("deleted %d rows, expected %d: %v", count, scenario.expired, err)
				}
				if err := transaction.Rollback(); err != nil {
					benchmark.Fatal(err)
				}
			}
		})
	}
}
