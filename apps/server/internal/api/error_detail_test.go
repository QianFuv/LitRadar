package api

import (
	"errors"
	"fmt"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
)

func TestPublicErrorDetailsPreserveContractsAndErrorIdentity(t *testing.T) {
	for _, scenario := range []struct {
		failure error
		detail  string
	}{
		{auth.ErrCredentials, "Invalid username or password"},
		{config.ErrNotFound, "Database not found"},
		{fmt.Errorf("authenticate: %w", auth.ErrCredentials), "authenticate: Invalid username or password"},
		{errors.Join(auth.ErrCredentials, config.ErrNotFound), "Invalid username or password\nDatabase not found"},
		{errors.New("invalid username or password"), "invalid username or password"},
	} {
		if actual := publicErrorDetail(scenario.failure); actual != scenario.detail {
			t.Fatalf("got %q, want %q", actual, scenario.detail)
		}
	}
}
