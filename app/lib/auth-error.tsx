/**
 * User-facing authentication error classification.
 */

import { ApiError } from '@/lib/api/client';

export type AuthFormMode = 'login' | 'register';

const FALLBACK_AUTH_ERROR = '操作失败，请重试';
const KNOWN_AUTH_ERROR_MESSAGES = new Map<string, string>([
  ['Invalid username or password', '用户名或密码错误，请检查后重试。'],
  ['Username already exists', '该用户名已被注册，请更换用户名。'],
  ['Invite code is required', '请输入邀请码。'],
  ['Invalid or used invite code', '邀请码无效或已被使用，请确认后重试。'],
  ['Administrator bootstrap is required', '系统尚未完成管理员初始化，请联系管理员。'],
  [
    'Username must be 3-32 alphanumeric or underscore characters',
    '用户名需为 3–32 位字母、数字或下划线。',
  ],
  ['Password must be at least 12 characters', '密码至少需要 12 个字符。'],
  ['Too many authentication attempts; try again later', '尝试次数过多，请稍后再试。'],
]);

/**
 * Convert an authentication failure into an actionable user-facing message.
 *
 * @param error - Unknown error raised by the authentication operation.
 * @param mode - Active authentication form mode.
 * @returns Classified message or the original safe error detail.
 */
export function getAuthErrorMessage(error: unknown, mode: AuthFormMode): string {
  if (error instanceof ApiError) {
    if (error.status === 429) {
      return '尝试次数过多，请稍后再试。';
    }
    if (error.status === 401) {
      return '用户名或密码错误，请检查后重试。';
    }
    if (error.status === 409 && mode === 'register') {
      return '该用户名已被注册，请更换用户名。';
    }
    return (
      KNOWN_AUTH_ERROR_MESSAGES.get(error.message) ?? (error.message.trim() || FALLBACK_AUTH_ERROR)
    );
  }
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  return FALLBACK_AUTH_ERROR;
}
