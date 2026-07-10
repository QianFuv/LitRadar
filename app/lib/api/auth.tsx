/**
 * Authentication, access-token, invite, password, and CNKI session API operations.
 */

import {
  parseAuthUser,
  parseInviteRequirement,
  parseLoginResponse,
  type AuthUser,
  type InviteRequirement,
  type LoginResponse,
} from '@/lib/api-contract';
import { buildApiUrl, requestJson } from '@/lib/api/client';
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
 * @param token - Optional explicit bearer access token.
 * @returns Current user.
 */
export function getCurrentUser(token?: string | null): Promise<AuthUser> {
  return requestJson<AuthUser>(
    buildApiUrl('/api/auth/me'),
    token,
    undefined,
    '获取用户失败',
    parseAuthUser,
  );
}

/**
 * Authenticate a user with username and password.
 *
 * @param username - Username.
 * @param password - Password.
 * @returns Login response.
 */
export function loginUser(username: string, password: string): Promise<LoginResponse> {
  return requestJson<LoginResponse>(
    buildApiUrl('/api/auth/login'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    },
    '登录失败',
    parseLoginResponse,
  );
}

/**
 * Register a user with a required invite code.
 *
 * @param username - Username.
 * @param password - Password.
 * @param inviteCode - Invite code.
 * @returns Empty promise when registration succeeds.
 */
export async function registerUser(
  username: string,
  password: string,
  inviteCode: string,
): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/auth/register'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ username, password, invite_code: inviteCode }),
    },
    '注册失败',
  );
}

/**
 * Revoke the active login token.
 *
 * @param token - Optional explicit bearer access token.
 */
export async function logoutUser(token?: string | null): Promise<void> {
  await fetch(buildApiUrl('/api/auth/logout'), {
    method: 'POST',
    credentials: 'include',
    headers: token ? { Authorization: `Bearer ${token}` } : undefined,
  }).catch(() => undefined);
}

/**
 * Check whether registration currently requires an invite code.
 *
 * @returns Invite requirement.
 */
export function getInviteRequirement(): Promise<InviteRequirement> {
  return requestJson<InviteRequirement>(
    buildApiUrl('/api/auth/invite-required'),
    null,
    undefined,
    '获取邀请码状态失败',
    parseInviteRequirement,
  );
}
/**
 * Fetch current user access tokens.
 *
 * @returns Access tokens.
 */
export function getAccessTokens(): Promise<AccessToken[]> {
  return requestJson<AccessToken[]>(
    buildApiUrl('/api/auth/tokens'),
    null,
    undefined,
    '获取访问令牌失败',
  );
}

/**
 * Fetch current user's Zhejiang Library CNKI session status.
 *
 * @returns Safe CNKI session status.
 */
export function getCnkiSession(): Promise<CnkiSessionStatus> {
  return requestJson<CnkiSessionStatus>(
    buildApiUrl('/api/cnki/session'),
    null,
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
    null,
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
    null,
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
    null,
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
    null,
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
    null,
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
    null,
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
    null,
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
    null,
    { method: 'POST' },
    '生成邀请码失败',
  );
}
