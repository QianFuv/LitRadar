package auth

import (
	"context"
	"errors"
	"fmt"
	"path/filepath"
	"testing"

	migration "github.com/QianFuv/LitRadar/apps/server/internal/storage/migrations/auth"
)

// TestBootstrapRequiredUsesAnyUser checks the public bootstrap signal independently of administrator status.
func TestBootstrapRequiredUsesAnyUser(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	isRequired, err := repository.BootstrapRequired(ctx)
	if err != nil || !isRequired {
		t.Fatalf("empty database: required=%t, err=%v", isRequired, err)
	}
	runSql(t, repository, "INSERT INTO users(id,username,password_hash,salt,is_admin,created_at,updated_at) VALUES(1,'member','hash','salt',0,1,1)")
	isRequired, err = repository.BootstrapRequired(ctx)
	if err != nil || isRequired {
		t.Fatalf("non-administrator present: required=%t, err=%v", isRequired, err)
	}
	runSql(t, repository, "UPDATE users SET is_admin=1 WHERE id=1")
	isRequired, err = repository.BootstrapRequired(ctx)
	if err != nil || isRequired {
		t.Fatalf("administrator present: required=%t, err=%v", isRequired, err)
	}
	runSql(t, repository, "DELETE FROM users")
	isRequired, err = repository.BootstrapRequired(ctx)
	if err != nil || !isRequired {
		t.Fatalf("last user deleted: required=%t, err=%v", isRequired, err)
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if _, err := repository.BootstrapRequired(cancelled); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancelled read: %v", err)
	}
	if err := repository.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.BootstrapRequired(ctx); err == nil {
		t.Fatal("closed database accepted")
	}
}

// BenchmarkBootstrapRequired measures the public status read on empty, ordinary and large user inventories.
func BenchmarkBootstrapRequired(benchmark *testing.B) {
	for _, users := range []int{0, 1, 1000, 100000} {
		benchmark.Run(fmt.Sprintf("users_%d", users), func(benchmark *testing.B) {
			ctx := context.Background()
			filename := filepath.Join(benchmark.TempDir(), "auth.sqlite")
			if _, err := migration.Migrate(ctx, filename); err != nil {
				benchmark.Fatal(err)
			}
			repository, err := Open(filename)
			if err != nil {
				benchmark.Fatal(err)
			}
			benchmark.Cleanup(func() { repository.Close() })
			if users > 0 {
				if _, err := repository.database.Exec(`WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM ids WHERE id<?)
INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) SELECT id,printf('synthetic-%d',id),'hash','salt',1,1 FROM ids`, users); err != nil {
					benchmark.Fatal(err)
				}
			}
			benchmark.ReportAllocs()
			benchmark.ResetTimer()
			for benchmark.Loop() {
				isRequired, err := repository.BootstrapRequired(ctx)
				if err != nil || isRequired != (users == 0) {
					benchmark.Fatalf("required=%t, err=%v", isRequired, err)
				}
			}
		})
	}
}
