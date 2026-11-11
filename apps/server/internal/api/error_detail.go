package api

import (
	"errors"
	"strings"

	authdomain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	authstorage "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/query"
)

// publicErrorDetails keeps the public problem vocabulary independent of Go diagnostics.
var publicErrorDetails = []struct {
	failure error
	detail  string
}{
	{authdomain.ErrEmail, "Email must be at most 254 bytes with one @ and no whitespace or control characters"},
	{authdomain.ErrEmailExists, "Email already exists"},
	{authdomain.ErrManagedEmail, "Email is managed by the identity provider"},
	{authdomain.ErrOidcPassword, "Password login is not available for single sign-on accounts"},
	{authdomain.ErrCredentials, "Invalid username or password"},
	{authdomain.ErrUsername, "Username must be 3-32 alphanumeric or underscore characters"},
	{authdomain.ErrPasswordShort, "Password must be at least 12 characters"},
	{authdomain.ErrTokenNameLength, "Access token name must be at most 100 Unicode code points"},
	{authdomain.ErrTokenReservedName, "Access token name \"login\" is reserved"},
	{authdomain.ErrTokenTTL, "Access token TTL must be between 3600 and 31536000 seconds"},
	{authdomain.ErrTokenLimit, "Active access token limit of 50 reached; revoke a token before creating another"},
	{authdomain.ErrInviteRequired, "Invite code is required"},
	{authdomain.ErrBootstrapRequired, "Administrator bootstrap is required"},
	{authdomain.ErrInvite, "Invalid, expired, revoked, or exhausted invite code"},
	{authdomain.ErrActiveInvite, "An unrevoked invite code already exists; rotate it instead"},
	{authdomain.ErrInvitePolicy, "Invite code policy is outside the allowed range"},
	{authdomain.ErrUsernameExists, "Username already exists"},
	{authdomain.ErrBootstrapComplete, "Administrator bootstrap is already complete"},
	{authdomain.ErrEntropy, "Operating-system cryptographic randomness is unavailable"},
	{authdomain.ErrCredentialInvariant, "Credential update affected an unexpected number of users"},
	{authdomain.ErrStaleAuthorization, "Token authorization changed before issuance"},
	{authdomain.ErrAdminForbidden, "Admin access required"},
	{authdomain.ErrAudit, "Security audit persistence failed"},
	{authdomain.ErrDeviceName, "Device name must contain 1-93 Unicode code points, no control characters, and non-whitespace text"},
	{authstorage.ErrAuditEvent, "Security audit event is invalid"},
	{authstorage.ErrAuditRetention, "Security audit retention days are invalid"},
	{authstorage.ErrUserNotFound, "User not found"},
	{authstorage.ErrLastAdministrator, "At least one administrator is required"},
	{config.ErrNoDatabases, "No SQLite databases found"},
	{config.ErrNotFound, "Database not found"},
	{config.ErrMultipleDatabases, "Multiple databases found, specify ?db=<name>"},
	{config.ErrInvalidName, "Database name is not a safe catalog stem"},
	{favorites.ErrSourceFolderNotFound, "Source folder not found"},
	{favorites.ErrTargetFolderNotFound, "Target folder not found"},
	{favorites.ErrSameFolders, "Source and target folders must be different"},
	{favorites.ErrCursor, "Invalid favorite pagination cursor"},
	{query.ErrInvalidCursor, "Invalid cursor"},
	{query.ErrUnsupportedArticleSort, "Articles only support sort=date:desc or date:asc"},
	{query.ErrInvalidSearchExpression, "Invalid search expression"},
	{query.ErrLegacyWeeklyLimit, "Weekly updates exceed 2000 articles; use /api/weekly-updates/summary and /api/weekly-updates/articles"},
}

// publicErrorDetail preserves established problem details, including wrapped and joined failures.
func publicErrorDetail(err error) string {
	detail := err.Error()
	for _, public := range publicErrorDetails {
		if errors.Is(err, public.failure) {
			detail = strings.Replace(detail, public.failure.Error(), public.detail, 1)
		}
	}
	return detail
}
