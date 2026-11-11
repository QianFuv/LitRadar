package api

import (
	"context"
	"log/slog"
	"net/http"
	"net/netip"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/cryptography"
	"github.com/danielgtaylor/huma/v2"
)

type sessionOutput[Body any] struct {
	// Body contains the decoded operation payload.
	Body Body
	// SetCookie sets the Set-Cookie header.
	SetCookie []string `header:"Set-Cookie"`
}
type loginResponse struct {
	// User describes authenticated user.
	User userResponse `json:"user" nullable:"false"`
	// ExpiresAt describes token expiration timestamp.
	ExpiresAt float64 `json:"expires_at" nullable:"false" format:"double"`
}
type nativeLoginResponse struct {
	// Token describes personal access token for Authorization: Bearer.
	Token string `json:"token" nullable:"false"`
	// ExpiresAt describes token expiration timestamp.
	ExpiresAt float64 `json:"expires_at" nullable:"false" format:"double"`
	// User describes authenticated user.
	User userResponse `json:"user" nullable:"false"`
}
type logoutResponse struct {
	// OK reports whether the operation succeeded.
	OK bool `json:"ok" nullable:"false"`
	// UserID describes authenticated user identifier.
	UserID int64 `json:"user_id" nullable:"false" format:"int64"`
}
type inviteRequiredResponse struct {
	// Required reports whether registration requires an invite code.
	Required bool `json:"required" nullable:"false"`
	// BootstrapRequired reports whether a local administrator bootstrap must run before invites can be issued.
	BootstrapRequired bool `json:"bootstrap_required" nullable:"false"`
}

func authOperation[Body any](handlers *authHandlers, request *http.Request, name string, tokenID int64, work func(context.Context, currentIdentity, *domain.AuditEvent) (Body, *apiError)) (*output[Body], error) {
	request = beginAuthAudit(request)
	event := domain.AuditEvent{Action: authOperationAction(name), Outcome: "completed", RequestID: requestID(request), OccurredAt: currentTimestamp()}
	current, failure := handlers.authenticator.requireUser(request)
	if failure != nil {
		return nil, handlers.authOperationFailure(request, event, "authentication_required", failure).problem()
	}
	applyAuthAuditIdentity(&event, current, name, tokenID)
	payload, failure := work(context.WithoutCancel(request.Context()), current, &event)
	if failure != nil {
		if name == "logout" || name == "logout_all" {
			rejection := event
			rejection.Outcome, rejection.Reason = "rejected", "operation_failed"
			if auditFailure := handlers.persistAudit(request, rejection); auditFailure != nil {
				slog.ErrorContext(request.Context(), "security.audit.persistence_failed", "component", "security", "action", event.Action)
			}
			logAuth(request, event, "operation_failed")
			return nil, failure.problem()
		}
		return nil, handlers.authOperationFailure(request, event, "operation_failed", failure).problem()
	}
	if event.Action != "" {
		logAuth(request, event, "")
	}
	return &output[Body]{Body: payload}, nil
}
func bodyAuthOperation[Body any](handlers *authHandlers, request *http.Request, action string, work func(context.Context, *http.Request, *domain.AuditEvent) (Body, *apiError)) (*output[Body], error) {
	request = beginAuthAudit(request)
	event := domain.AuditEvent{Action: action, Outcome: "completed", RequestID: requestID(request), OccurredAt: currentTimestamp()}
	payload, failure := work(context.WithoutCancel(request.Context()), request, &event)
	if failure != nil {
		return nil, failure.problem()
	}
	logAuth(request, event, "")
	return &output[Body]{Body: payload}, nil
}
func (handlers *authHandlers) admitPasswordAttempt(request *http.Request, event *domain.AuditEvent, kind authAttempt, username string) *apiError {
	peer, _ := netip.ParseAddrPort(request.RemoteAddr)
	if rejection := handlers.limiter.check(kind, resolveAuthClientSource(peer.Addr(), request.Header, handlers.trustedProxies), username); rejection != nil {
		return handlers.authRateLimit(request, event, rejection)
	}
	return nil
}
func (handlers *authHandlers) routes() []route {
	return []route{
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/me", ID: "get_me"}, func(ctx context.Context, input *GetMeInput) (*output[userResponse], error) {
			return authOperation(handlers, input.request, "get_me", 0, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) (userResponse, *apiError) {
				return publicUser(current.authorization.User), nil
			})
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/tokens", ID: "get_tokens"}, func(ctx context.Context, input *GetTokensInput) (*output[[]domain.TokenInfo], error) {
			return authOperation(handlers, input.request, "get_tokens", 0, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) ([]domain.TokenInfo, *apiError) {
				return runAuth(input.request, handlers.pool, func() ([]domain.TokenInfo, error) {
					return handlers.service.ListTokens(workContext, current.authorization.User.ID)
				})
			})
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/invite-code", ID: "get_invite_code"}, func(ctx context.Context, input *GetInviteCodeInput) (*output[nullableBody[domain.InviteResponse]], error) {
			value, err := authOperation(handlers, input.request, "get_invite_code", 0, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) (*domain.InviteResponse, *apiError) {
				return runAuth(input.request, handlers.pool, func() (*domain.InviteResponse, error) {
					return handlers.service.Invite(workContext, current.authorization.User.ID)
				})
			})
			if err != nil {
				return nil, err
			}
			return &output[nullableBody[domain.InviteResponse]]{Body: nullableBody[domain.InviteResponse]{value: value.Body}}, nil
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/invite-code", ID: "generate_invite_code"}, func(ctx context.Context, input *GenerateInviteCodeInput) (*output[domain.InviteResponse], error) {
			return authOperation(handlers, input.request, "generate_invite_code", 0, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) (domain.InviteResponse, *apiError) {
				value, failure := runAuth(input.request, handlers.pool, func() (domain.InviteResponse, error) {
					return handlers.service.IssueInvite(workContext, current.authorization.User.ID, false, event)
				})
				if failure == nil {
					event.TargetID = &value.ID
				}
				return value, failure
			})
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/invite-code/rotate", ID: "rotate_invite_code"}, func(ctx context.Context, input *RotateInviteCodeInput) (*output[domain.InviteResponse], error) {
			return authOperation(handlers, input.request, "rotate_invite_code", 0, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) (domain.InviteResponse, *apiError) {
				value, failure := runAuth(input.request, handlers.pool, func() (domain.InviteResponse, error) {
					return handlers.service.IssueInvite(workContext, current.authorization.User.ID, true, event)
				})
				if failure == nil {
					event.TargetID = &value.ID
				}
				return value, failure
			})
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/auth/tokens/{token_id}", ID: "delete_token"}, func(ctx context.Context, input *DeleteTokenInput) (*output[okResponse], error) {
			return authOperation(handlers, input.request, "delete_token", input.TokenID, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) (okResponse, *apiError) {
				changed, failure := runAuth(input.request, handlers.pool, func() (bool, error) {
					return handlers.service.RevokeTokenID(workContext, current.authorization.User.ID, input.TokenID, *event)
				})
				if failure != nil {
					return okResponse{}, failure
				}
				if !changed {
					return okResponse{}, &apiError{status: 404, detail: "Token not found"}
				}
				return okResponse{OK: true}, nil
			})
		}),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/auth/invite-code", ID: "revoke_invite_code"}, func(ctx context.Context, input *RevokeInviteCodeInput) (*output[okResponse], error) {
			return authOperation(handlers, input.request, "revoke_invite_code", 0, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) (okResponse, *apiError) {
				changed, failure := runAuth(input.request, handlers.pool, func() (bool, error) {
					return handlers.service.RevokeInvite(workContext, current.authorization.User.ID, event)
				})
				if failure != nil {
					return okResponse{}, failure
				}
				if !changed {
					return okResponse{}, &apiError{status: 404, detail: "Unrevoked invite code not found"}
				}
				return okResponse{OK: true}, nil
			})
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/invite-required", ID: "check_invite_required"}, func(ctx context.Context, input *CheckInviteRequiredInput) (*output[inviteRequiredResponse], error) {
			value, failure := runAuth(input.request, handlers.pool, func() (bool, error) { return handlers.service.BootstrapRequired(context.WithoutCancel(ctx)) })
			if failure != nil {
				return nil, failure.problem()
			}
			return &output[inviteRequiredResponse]{Body: inviteRequiredResponse{Required: handlers.service.InviteRequired(), BootstrapRequired: value}}, nil
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/logout", ID: "logout"}, func(ctx context.Context, input *LogoutInput) (*sessionOutput[logoutResponse], error) {
			return handlers.logout(input.request, "logout")
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/logout-all", ID: "logout_all"}, func(ctx context.Context, input *LogoutAllInput) (*sessionOutput[logoutResponse], error) {
			return handlers.logout(input.request, "logout_all")
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/register", ID: "register"}, func(ctx context.Context, input *RegisterInput) (*output[userResponse], error) {
			return bodyAuthOperation(handlers, input.request, "register", func(workContext context.Context, request *http.Request, event *domain.AuditEvent) (userResponse, *apiError) {
				username := strings.TrimSpace(input.Body.Username)
				if failure := handlers.admitPasswordAttempt(request, event, registerAttempt, username); failure != nil {
					return userResponse{}, failure
				}
				if !auth.ValidUsername(username) {
					return userResponse{}, handlers.reject(request, *event, "validation_failed", mapAuthError(domain.ErrUsername))
				}
				if !cryptography.ValidNewPassword(input.Body.Password) {
					return userResponse{}, handlers.reject(request, *event, "validation_failed", mapAuthError(domain.ErrPasswordShort))
				}
				invite := cleanText(&input.Body.InviteCode)
				user, failure := runAuth(request, handlers.kdfPool, func() (domain.User, error) {
					return handlers.service.Register(workContext, username, input.Body.Password, invite, event, input.Body.Email)
				})
				if failure != nil {
					return userResponse{}, handlers.reject(request, *event, "registration_failed", failure)
				}
				actor := int64(user.ID)
				event.ActorID = &actor
				handlers.limiter.clearUsername(registerAttempt, username)
				return publicUser(user), nil
			})
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/login", ID: "login"}, handlers.login),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/native/login", ID: "native_login"}, func(ctx context.Context, input *NativeLoginInput) (*output[nativeLoginResponse], error) {
			return bodyAuthOperation(handlers, input.request, "native_login", func(workContext context.Context, request *http.Request, event *domain.AuditEvent) (nativeLoginResponse, *apiError) {
				username := strings.TrimSpace(input.Body.Username)
				if failure := handlers.admitPasswordAttempt(request, event, loginAttempt, username); failure != nil {
					return nativeLoginResponse{}, failure
				}
				session, failure := runAuth(request, handlers.kdfPool, func() (domain.LoginSession, error) {
					return handlers.service.NativeLogin(workContext, username, input.Body.Password, input.Body.DeviceName, event)
				})
				if failure != nil {
					return nativeLoginResponse{}, handlers.reject(request, *event, "authentication_failed", failure)
				}
				actor := int64(session.User.ID)
				event.ActorID = &actor
				handlers.limiter.clearUsername(loginAttempt, username)
				return nativeLoginResponse{Token: session.Token, ExpiresAt: session.ExpiresAt, User: publicUser(session.User)}, nil
			})
		}),
		typedRoute(openapi.Operation{Method: "PUT", Path: "/api/auth/email", ID: "update_email"}, func(ctx context.Context, input *UpdateEmailInput) (*output[userResponse], error) {
			return bodyAuthOperation(handlers, input.request, "email_change", func(workContext context.Context, request *http.Request, event *domain.AuditEvent) (userResponse, *apiError) {
				current, failure := handlers.authenticator.requireUser(request)
				if failure != nil {
					var zero userResponse
					return zero, handlers.reject(request, *event, "authentication_required", failure)
				}
				actor := int64(current.authorization.User.ID)
				event.ActorID = &actor
				value, failure := func() (userResponse, *apiError) {
					user, failure := runAuth(request, handlers.pool, func() (domain.User, error) {
						return handlers.service.UpdateEmail(workContext, current.authorization.User.ID, input.Body.Email, event)
					})
					return publicUser(user), failure
				}()
				if failure != nil {
					return value, handlers.reject(request, *event, "operation_failed", failure)
				}
				return value, nil
			})
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/tokens", ID: "create_token"}, func(ctx context.Context, input *CreateTokenInput) (*output[domain.IssuedToken], error) {
			return bodyAuthOperation(handlers, input.request, "token_create", func(workContext context.Context, request *http.Request, event *domain.AuditEvent) (domain.IssuedToken, *apiError) {
				current, failure := handlers.authenticator.requireUser(request)
				if failure != nil {
					var zero domain.IssuedToken
					return zero, handlers.reject(request, *event, "authentication_required", failure)
				}
				actor := int64(current.authorization.User.ID)
				event.ActorID = &actor
				value, failure := func() (domain.IssuedToken, *apiError) {
					token, failure := runAuth(request, handlers.pool, func() (domain.IssuedToken, error) {
						return handlers.service.CreateToken(workContext, current.authorization, input.Body.Name, input.Body.TTL, event)
					})
					if failure == nil {
						event.TargetID = &token.ID
					}
					return token, failure
				}()
				if failure != nil {
					return value, handlers.reject(request, *event, "operation_failed", failure)
				}
				return value, nil
			})
		}),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/change-password", ID: "change_password"}, func(ctx context.Context, input *ChangePasswordInput) (*output[okResponse], error) {
			return bodyAuthOperation(handlers, input.request, "password_change", func(workContext context.Context, request *http.Request, event *domain.AuditEvent) (okResponse, *apiError) {
				if !cryptography.ValidNewPassword(input.Body.NewPassword) {
					return okResponse{}, handlers.reject(request, *event, "validation_failed", mapAuthError(domain.ErrPasswordShort))
				}
				current, failure := handlers.authenticator.requireUser(request)
				if failure != nil {
					var zero okResponse
					return zero, handlers.reject(request, *event, "authentication_required", failure)
				}
				actor := int64(current.authorization.User.ID)
				event.ActorID = &actor
				value, failure := func() (okResponse, *apiError) {
					changed, failure := runAuth(request, handlers.kdfPool, func() (bool, error) {
						return handlers.service.ChangePassword(workContext, current.authorization.User.ID, input.Body.OldPassword, input.Body.NewPassword, event)
					})
					if failure == nil && !changed {
						failure = &apiError{status: 400, detail: "Old password is incorrect"}
					}
					return okResponse{OK: true}, failure
				}()
				if failure != nil {
					return value, handlers.reject(request, *event, "operation_failed", failure)
				}
				return value, nil
			})
		}),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/providers", ID: "get_auth_providers"}, handlers.authProviders),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/oidc/login", ID: "oidc_login"}, handlers.oidcLogin),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/oidc/callback", ID: "oidc_callback"}, handlers.oidcCallback),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/oidc/link", ID: "oidc_link"}, handlers.oidcLink),
		typedRoute(openapi.Operation{Method: "DELETE", Path: "/api/auth/oidc/link", ID: "oidc_unlink"}, handlers.oidcUnlink),
		typedRoute(openapi.Operation{Method: "GET", Path: "/api/auth/oidc/native/login", ID: "oidc_native_login"}, handlers.oidcNativeLogin),
		typedRoute(openapi.Operation{Method: "POST", Path: "/api/auth/oidc/native/exchange", ID: "oidc_native_exchange"}, handlers.nativeOIDCExchange),
	}
}

func (handlers *authHandlers) login(ctx context.Context, input *LoginInput) (*sessionOutput[loginResponse], error) {
	var cookie string
	value, err := bodyAuthOperation(handlers, input.request, "login", func(workContext context.Context, request *http.Request, event *domain.AuditEvent) (loginResponse, *apiError) {
		username := strings.TrimSpace(input.Body.Username)
		if failure := handlers.admitPasswordAttempt(request, event, loginAttempt, username); failure != nil {
			return loginResponse{}, failure
		}
		session, failure := runAuth(request, handlers.kdfPool, func() (domain.LoginSession, error) {
			return handlers.service.Login(workContext, username, input.Body.Password, event)
		})
		if failure != nil {
			return loginResponse{}, handlers.reject(request, *event, "authentication_failed", failure)
		}
		actor := int64(session.User.ID)
		event.ActorID = &actor
		cookie = sessionCookieHeader(session.Token, session.ExpiresAt, currentTimestamp(), handlers.isCookieSecure)
		handlers.limiter.clearUsername(loginAttempt, username)
		return loginResponse{User: publicUser(session.User), ExpiresAt: session.ExpiresAt}, nil
	})
	if err != nil {
		return nil, err
	}
	return &sessionOutput[loginResponse]{Body: value.Body, SetCookie: []string{cookie}}, nil
}
func (handlers *authHandlers) logout(request *http.Request, name string) (*sessionOutput[logoutResponse], error) {
	cookies := []string{}
	if _, hasCookie := sessionCookie(request.Header); hasCookie {
		cookies = append(cookies, sessionCookieHeader("", 0, 0, handlers.isCookieSecure))
	}
	value, err := authOperation(handlers, request, name, 0, func(workContext context.Context, current currentIdentity, event *domain.AuditEvent) (logoutResponse, *apiError) {
		actor := int64(current.authorization.User.ID)
		_, failure := runAuth(request, handlers.pool, func() (bool, error) {
			return retryRevocation(func() (bool, error) {
				if name == "logout" {
					return handlers.service.RevokeToken(workContext, current.token, *event)
				}
				_, err := handlers.service.RevokeAll(workContext, current.authorization.User.ID, *event)
				return err == nil, err
			})
		})
		if failure != nil {
			slog.WarnContext(request.Context(), "security.auth.revocation_unconfirmed", "event", "security.auth.revocation_unconfirmed", "component", "security", "action", event.Action, "actor_id", actor, "request_id", event.RequestID)
			return logoutResponse{}, &apiError{status: 503, detail: "Session revocation could not be confirmed", details: []error{&huma.ErrorDetail{Location: "code", Message: "session_revocation_unconfirmed"}, &huma.ErrorDetail{Location: "request_id", Message: event.RequestID}}}
		}
		return logoutResponse{OK: true, UserID: actor}, nil
	})
	if err != nil {
		return nil, huma.ErrorWithHeaders(err, http.Header{"Set-Cookie": cookies})
	}
	return &sessionOutput[logoutResponse]{Body: value.Body, SetCookie: cookies}, nil
}
