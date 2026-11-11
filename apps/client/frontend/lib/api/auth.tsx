/**
 * Authentication, access-token, invite, password, and CNKI session API operations.
 */

import {
  parseAuthProviders,
  parseAuthUser,
  parseInviteRequirement,
  parseLoginResponse,
  parseOidcLinkStart,
  type AuthUser,
  type LoginResponse,
  type OidcLinkStart,
} from '@/lib/api-contract';
import {
  ApiError,
  apiClient,
  createServerFetch,
  nativeSessionClient,
  readData,
  resolveApiBase,
  $api,
} from '@/lib/api/client';
import { apiQuery, validatedApiQuery } from '@/lib/api/query';
import { CLIENT_PLATFORM } from '@/lib/platform';
import type {
  CnkiLoginPollResponse,
  CnkiLoginStartResponse,
  CnkiSessionStatus,
  CreatedAccessToken,
  InviteCode,
} from '@/lib/api/types';

/**
 * Get the current authenticated user.
 *
 * @returns Current user.
 */
export async function getCurrentUser(): Promise<AuthUser> {
  return parseAuthUser(readData(await apiClient.GET('/api/auth/me')));
}

/**
 * Authenticate a user with a username or email and a password.
 *
 * Native hosts sign in through their own endpoint and keep the issued token; the browser
 * receives the session cookie.
 *
 * @param username - Username or account email.
 * @param password - Password.
 * @returns Login response.
 */
export async function loginUser(username: string, password: string): Promise<LoginResponse> {
  const nativeSession = CLIENT_PLATFORM.nativeSession;
  if (nativeSession) {
    return parseLoginResponse(
      readData(
        await nativeSessionClient.POST('/api/auth/native/login', {
          fetch: createServerFetch((_url, init) =>
            nativeSession.login(username, password, init.signal ?? null),
          ),
        }),
      ),
    );
  }
  return parseLoginResponse(
    readData(await apiClient.POST('/api/auth/login', { body: { username, password } })),
  );
}

/**
 * Register a user with a required invite code and an optional email.
 *
 * @param username - Username.
 * @param password - Password.
 * @param inviteCode - Invite code.
 * @param email - Optional account email; omitted from the request when empty.
 * @returns Empty promise when registration succeeds.
 */
export async function registerUser(
  username: string,
  password: string,
  inviteCode: string,
  email = '',
): Promise<void> {
  await apiClient.POST('/api/auth/register', {
    body: { username, password, invite_code: inviteCode, ...(email ? { email } : {}) },
  });
}

/**
 * Sign a native host in through the server's single sign-on in the system browser.
 *
 * The host opens the browser, waits for the callback, exchanges the one-time code, and keeps
 * the issued token; only the exchange response without the token reaches this function. The
 * shared request timeout does not apply to the browser step, which may take minutes.
 *
 * @param signal - Cancellation signal that stops waiting for the browser.
 * @returns Login response.
 */
export async function loginWithNativeOidc(signal: AbortSignal): Promise<LoginResponse> {
  const nativeSession = CLIENT_PLATFORM.nativeSession;
  if (!nativeSession) {
    throw new Error('客户端单点登录仅适用于桌面和移动客户端');
  }
  const response = await nativeSession.loginWithOidc(signal);
  return parseLoginResponse(
    readData(
      await nativeSessionClient.POST('/api/auth/oidc/native/exchange', {
        fetch: createServerFetch(() => Promise.resolve(response)),
        signal,
      }),
    ),
  );
}

/**
 * Revoke the active login token.
 *
 * A native host deletes its stored token even when the server cannot confirm revocation.
 */
export async function logoutUser(): Promise<void> {
  const nativeSession = CLIENT_PLATFORM.nativeSession;
  try {
    await apiClient.POST(
      '/api/auth/logout',
      nativeSession
        ? {
            fetch: createServerFetch((_url, init) => nativeSession.logout(init.signal ?? null)),
          }
        : {},
    );
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      return;
    }
    throw error;
  }
}

/**
 * Revoke every login session and personal access token for the authenticated user.
 *
 * @returns Empty promise after durable revocation is confirmed.
 */
export async function logoutAllSessions(): Promise<void> {
  await apiClient.POST('/api/auth/logout-all');
}

/**
 * Build the browser navigation target that starts single sign-on.
 *
 * @param nextPath - Same-site path to return to after sign-in.
 * @returns Absolute OIDC login start URL.
 */
export function buildOidcLoginUrl(nextPath: string): string {
  const url = new URL('/api/auth/oidc/login', resolveApiBase());
  url.searchParams.set('next', nextPath);
  return url.toString();
}

/**
 * Set or clear the active local account's email.
 *
 * @param email - New email, or null to clear it.
 * @returns Updated user.
 */
export async function updateAccountEmail(email: string | null): Promise<AuthUser> {
  return parseAuthUser(readData(await apiClient.PUT('/api/auth/email', { body: { email } })));
}

/**
 * Start linking the active local account to the single sign-on provider.
 *
 * @returns Provider authorization URL the browser must navigate to.
 */
export async function startOidcLink(): Promise<OidcLinkStart> {
  return parseOidcLinkStart(readData(await apiClient.POST('/api/auth/oidc/link')));
}

/**
 * Remove the single sign-on identity linked to the active local account.
 *
 * @returns Updated user.
 */
export async function unlinkOidc(): Promise<AuthUser> {
  return parseAuthUser(readData(await apiClient.DELETE('/api/auth/oidc/link')));
}

/**
 * Start Zhejiang Library CNKI QR login for the current user.
 *
 * @returns QR login challenge.
 */
export async function startCnkiLogin(): Promise<CnkiLoginStartResponse> {
  return readData(await apiClient.POST('/api/cnki/login/start'));
}

/**
 * Poll Zhejiang Library CNKI QR login for the current user.
 *
 * @param timeoutSeconds - Maximum polling duration.
 * @param intervalSeconds - Polling interval.
 * @returns QR login poll result.
 */
export async function pollCnkiLogin(
  timeoutSeconds: number,
  intervalSeconds: number,
): Promise<CnkiLoginPollResponse> {
  return readData(
    await apiClient.POST('/api/cnki/login/poll', {
      body: { timeout_seconds: timeoutSeconds, interval_seconds: intervalSeconds },
    }),
  );
}

/**
 * Clear current user's Zhejiang Library CNKI session.
 *
 * @returns Safe empty CNKI session status.
 */
export async function clearCnkiSession(): Promise<CnkiSessionStatus> {
  return readData(await apiClient.DELETE('/api/cnki/session'));
}

/**
 * Create an access token.
 *
 * @param name - Token name.
 * @param ttl - Time to live in seconds.
 * @returns Created token response.
 */
export async function createAccessToken(name: string, ttl: number): Promise<CreatedAccessToken> {
  return readData(await apiClient.POST('/api/auth/tokens', { body: { name, ttl } }));
}

/**
 * Revoke an access token.
 *
 * @param tokenId - Access token id.
 */
export async function revokeAccessToken(tokenId: number): Promise<void> {
  await apiClient.DELETE('/api/auth/tokens/{token_id}', {
    params: { path: { token_id: tokenId } },
  });
}

/**
 * Change the active user's password.
 *
 * @param oldPassword - Current password.
 * @param newPassword - New password.
 */
export async function changePassword(oldPassword: string, newPassword: string): Promise<void> {
  await apiClient.POST('/api/auth/change-password', {
    body: { old_password: oldPassword, new_password: newPassword },
  });
}

/**
 * Generate the current user's invite code.
 *
 * @returns Generated invite code.
 */
export async function generateInviteCode(): Promise<InviteCode> {
  return readData(await apiClient.POST('/api/auth/invite-code'));
}

/**
 * Rotate the current user's invite code.
 *
 * @returns Newly issued replacement invite code.
 */
export async function rotateInviteCode(): Promise<InviteCode> {
  return readData(await apiClient.POST('/api/auth/invite-code/rotate'));
}

/**
 * Irreversibly revoke the current user's invite code.
 */
export async function revokeInviteCode(): Promise<void> {
  await apiClient.DELETE('/api/auth/invite-code');
}

/**
 * Query of the signed-in user, contract-validated.
 *
 * @returns Query options.
 */
export function currentUserQuery() {
  return validatedApiQuery($api.queryOptions('get', '/api/auth/me'), parseAuthUser);
}

/**
 * Query of the sign-in providers enabled on this server, contract-validated.
 *
 * @returns Query options.
 */
export function authProvidersQuery() {
  return validatedApiQuery($api.queryOptions('get', '/api/auth/providers'), parseAuthProviders);
}

/**
 * Query of whether registration requires an invite code, contract-validated.
 *
 * @returns Query options.
 */
export function inviteRequirementQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/auth/invite-required'),
    parseInviteRequirement,
  );
}

/**
 * Query of the current user's access tokens.
 *
 * @returns Query options.
 */
export function accessTokensQuery() {
  return apiQuery($api.queryOptions('get', '/api/auth/tokens'));
}

/**
 * Query of the current user's Zhejiang Library CNKI session status.
 *
 * @returns Query options.
 */
export function cnkiSessionQuery() {
  return apiQuery($api.queryOptions('get', '/api/cnki/session'));
}

/**
 * Query of the current user's invite code.
 *
 * @returns Query options.
 */
export function inviteCodeQuery() {
  return apiQuery($api.queryOptions('get', '/api/auth/invite-code'));
}
