/**
 * Authentication, access-token, invite, password, and CNKI session API operations.
 */

import {
  parseAuthProviders,
  parseAuthUser,
  parseInviteRequirement,
  parseLoginResponse,
  parseOidcLinkStart,
  type AuthProviders,
  type AuthUser,
  type InviteRequirement,
  type LoginResponse,
  type OidcLinkStart,
} from '@/lib/api-contract';
import { ApiError, buildApiUrl, requestJson, requestJsonVia } from '@/lib/api/client';
import { CLIENT_PLATFORM } from '@/lib/platform';
import type {
  AccessToken,
  CnkiLoginPollResponse,
  CnkiLoginStartResponse,
  CnkiSessionStatus,
  InviteCode,
} from '@/lib/api/types';

/**
 * Get the current authenticated user.
 *
 * @returns Current user.
 */
export function getCurrentUser(): Promise<AuthUser> {
  return requestJson<AuthUser>(
    buildApiUrl('/api/auth/me'),
    undefined,
    '获取用户失败',
    parseAuthUser,
  );
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
export function loginUser(username: string, password: string): Promise<LoginResponse> {
  const nativeSession = CLIENT_PLATFORM.nativeSession;
  if (nativeSession) {
    return requestJsonVia<LoginResponse>(
      (_url, init) => nativeSession.login(username, password, init.signal ?? null),
      buildApiUrl('/api/auth/native/login'),
      { method: 'POST' },
      '登录失败',
      parseLoginResponse,
    );
  }
  return requestJson<LoginResponse>(
    buildApiUrl('/api/auth/login'),
    {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    },
    '登录失败',
    parseLoginResponse,
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
  await requestJson<unknown>(
    buildApiUrl('/api/auth/register'),
    {
      method: 'POST',
      body: JSON.stringify({
        username,
        password,
        invite_code: inviteCode,
        ...(email ? { email } : {}),
      }),
    },
    '注册失败',
  );
}

/**
 * Sign a native host in through the server's single sign-on in the system browser.
 *
 * The host opens the browser, waits for the callback, exchanges the one-time code, and keeps
 * the issued token; only the exchange response without the token reaches this function. The
 * shared request timeout does not apply because the browser step may take minutes.
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
  return requestJsonVia<LoginResponse>(
    () => Promise.resolve(response),
    buildApiUrl('/api/auth/oidc/native/exchange'),
    { method: 'POST', signal },
    '单点登录失败',
    parseLoginResponse,
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
    if (nativeSession) {
      await requestJsonVia<unknown>(
        (_url, init) => nativeSession.logout(init.signal ?? null),
        buildApiUrl('/api/auth/logout'),
        { method: 'POST' },
        '服务端会话撤销未确认',
      );
      return;
    }
    await requestJson<unknown>(
      buildApiUrl('/api/auth/logout'),
      { method: 'POST' },
      '服务端会话撤销未确认',
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
  await requestJson<unknown>(
    buildApiUrl('/api/auth/logout-all'),
    { method: 'POST' },
    '撤销全部会话失败',
  );
}

/**
 * Check whether registration currently requires an invite code.
 *
 * @returns Invite requirement.
 */
export function getInviteRequirement(): Promise<InviteRequirement> {
  return requestJson<InviteRequirement>(
    buildApiUrl('/api/auth/invite-required'),
    undefined,
    '获取邀请码状态失败',
    parseInviteRequirement,
  );
}
/**
 * Fetch the sign-in providers enabled on this server.
 *
 * @returns Single sign-on availability and display name.
 */
export function getAuthProviders(): Promise<AuthProviders> {
  return requestJson<AuthProviders>(
    buildApiUrl('/api/auth/providers'),
    undefined,
    '获取登录方式失败',
    parseAuthProviders,
  );
}

/**
 * Build the browser navigation target that starts single sign-on.
 *
 * @param nextPath - Same-site path to return to after sign-in.
 * @returns Absolute OIDC login start URL.
 */
export function buildOidcLoginUrl(nextPath: string): string {
  return buildApiUrl('/api/auth/oidc/login', new URLSearchParams({ next: nextPath }));
}

/**
 * Set or clear the active local account's email.
 *
 * @param email - New email, or null to clear it.
 * @returns Updated user.
 */
export function updateAccountEmail(email: string | null): Promise<AuthUser> {
  return requestJson<AuthUser>(
    buildApiUrl('/api/auth/email'),
    { method: 'PUT', body: JSON.stringify({ email }) },
    '更新邮箱失败',
    parseAuthUser,
  );
}

/**
 * Start linking the active local account to the single sign-on provider.
 *
 * @returns Provider authorization URL the browser must navigate to.
 */
export function startOidcLink(): Promise<OidcLinkStart> {
  return requestJson<OidcLinkStart>(
    buildApiUrl('/api/auth/oidc/link'),
    { method: 'POST' },
    '发起单点登录绑定失败',
    parseOidcLinkStart,
  );
}

/**
 * Remove the single sign-on identity linked to the active local account.
 *
 * @returns Updated user.
 */
export function unlinkOidc(): Promise<AuthUser> {
  return requestJson<AuthUser>(
    buildApiUrl('/api/auth/oidc/link'),
    { method: 'DELETE' },
    '解除单点登录绑定失败',
    parseAuthUser,
  );
}

/**
 * Fetch current user access tokens.
 *
 * @returns Access tokens.
 */
export function getAccessTokens(): Promise<AccessToken[]> {
  return requestJson<AccessToken[]>(buildApiUrl('/api/auth/tokens'), undefined, '获取访问令牌失败');
}

/**
 * Fetch current user's Zhejiang Library CNKI session status.
 *
 * @returns Safe CNKI session status.
 */
export function getCnkiSession(): Promise<CnkiSessionStatus> {
  return requestJson<CnkiSessionStatus>(
    buildApiUrl('/api/cnki/session'),
    undefined,
    '获取知网登录状态失败',
  );
}

/**
 * Start Zhejiang Library CNKI QR login for the current user.
 *
 * @returns QR login challenge.
 */
export function startCnkiLogin(): Promise<CnkiLoginStartResponse> {
  return requestJson<CnkiLoginStartResponse>(
    buildApiUrl('/api/cnki/login/start'),
    { method: 'POST' },
    '启动知网登录失败',
  );
}

/**
 * Poll Zhejiang Library CNKI QR login for the current user.
 *
 * @param timeoutSeconds - Maximum polling duration.
 * @param intervalSeconds - Polling interval.
 * @returns QR login poll result.
 */
export function pollCnkiLogin(
  timeoutSeconds: number,
  intervalSeconds: number,
): Promise<CnkiLoginPollResponse> {
  return requestJson<CnkiLoginPollResponse>(
    buildApiUrl('/api/cnki/login/poll'),
    {
      method: 'POST',
      body: JSON.stringify({
        timeout_seconds: timeoutSeconds,
        interval_seconds: intervalSeconds,
      }),
    },
    '确认知网登录失败',
  );
}

/**
 * Clear current user's Zhejiang Library CNKI session.
 *
 * @returns Safe empty CNKI session status.
 */
export function clearCnkiSession(): Promise<CnkiSessionStatus> {
  return requestJson<CnkiSessionStatus>(
    buildApiUrl('/api/cnki/session'),
    { method: 'DELETE' },
    '清除知网登录失败',
  );
}

/**
 * Create an access token.
 *
 * @param name - Token name.
 * @param ttl - Time to live in seconds.
 * @returns Created token response.
 */
export function createAccessToken(
  name: string,
  ttl: number,
): Promise<{ id: number; token: string; name: string; expires_at: number }> {
  return requestJson<{ id: number; token: string; name: string; expires_at: number }>(
    buildApiUrl('/api/auth/tokens'),
    {
      method: 'POST',
      body: JSON.stringify({ name, ttl }),
    },
    '创建访问令牌失败',
  );
}

/**
 * Revoke an access token.
 *
 * @param tokenId - Access token id.
 */
export async function revokeAccessToken(tokenId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/auth/tokens/${tokenId}`),
    { method: 'DELETE' },
    '撤销访问令牌失败',
  );
}

/**
 * Change the active user's password.
 *
 * @param oldPassword - Current password.
 * @param newPassword - New password.
 */
export async function changePassword(oldPassword: string, newPassword: string): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/auth/change-password'),
    {
      method: 'POST',
      body: JSON.stringify({ old_password: oldPassword, new_password: newPassword }),
    },
    '修改密码失败',
  );
}

/**
 * Fetch the current user's invite code.
 *
 * @returns Invite code or null.
 */
export function getInviteCode(): Promise<InviteCode | null> {
  return requestJson<InviteCode | null>(
    buildApiUrl('/api/auth/invite-code'),
    undefined,
    '获取邀请码失败',
  );
}

/**
 * Generate the current user's invite code.
 *
 * @returns Generated invite code.
 */
export function generateInviteCode(): Promise<InviteCode> {
  return requestJson<InviteCode>(
    buildApiUrl('/api/auth/invite-code'),
    { method: 'POST' },
    '生成邀请码失败',
  );
}

/**
 * Rotate the current user's invite code.
 *
 * @returns Newly issued replacement invite code.
 */
export function rotateInviteCode(): Promise<InviteCode> {
  return requestJson<InviteCode>(
    buildApiUrl('/api/auth/invite-code/rotate'),
    { method: 'POST' },
    '轮换邀请码失败',
  );
}

/**
 * Irreversibly revoke the current user's invite code.
 */
export async function revokeInviteCode(): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/auth/invite-code'),
    { method: 'DELETE' },
    '撤销邀请码失败',
  );
}
