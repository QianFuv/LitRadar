package auth

import (
	"context"
	"strings"
	"unicode"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
)

// NormalizeEmail validates and canonicalizes an optional account email; blank input means no email.
func NormalizeEmail(email *string) (*string, error) {
	if email == nil || strings.TrimSpace(*email) == "" {
		return nil, nil
	}
	value := strings.ToLower(strings.TrimSpace(*email))
	if len(value) > 254 || !utf8.ValidString(value) || strings.Count(value, "@") != 1 {
		return nil, domain.ErrEmail
	}
	local, host, _ := strings.Cut(value, "@")
	if local == "" || host == "" {
		return nil, domain.ErrEmail
	}
	for _, character := range value {
		if unicode.IsSpace(character) || unicode.IsControl(character) {
			return nil, domain.ErrEmail
		}
	}
	return &value, nil
}

// UpdateEmail changes a local account email with its required completion audit.
func (service *Service) UpdateEmail(ctx context.Context, user identity.Id, email *string, audit *domain.AuditEvent) (domain.User, error) {
	address, err := NormalizeEmail(email)
	if err != nil {
		return domain.User{}, err
	}
	event := service.event("email_change", audit).WithIdentity(int64(user), int64(user))
	return service.repository.UpdateEmail(ctx, user, address, service.now(), &event)
}
