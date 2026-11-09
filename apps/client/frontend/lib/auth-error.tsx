/**
 * User-facing authentication error classification.
 */

import { ApiError } from '@/lib/api/client';
import { PlatformBridgeError } from '@/lib/platform/errors';

export type AuthFormMode = 'login' | 'register' | 'token';

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
  ['Email already exists', '该邮箱已被其他账号使用，请更换邮箱。'],
  [
    'Email must be at most 254 bytes with one @ and no whitespace or control characters',
    '邮箱格式无效，请检查后重试。',
  ],
  ['Email is managed by the identity provider', '该邮箱由身份提供方管理，无法在此修改。'],
  ['Password login is not available for single sign-on accounts', '单点登录账号不支持密码登录。'],
  [
    'Active access token limit of 50 reached; revoke a token before creating another',
    '访问令牌数量已达上限（50 个），请先在网页版设置中撤销不用的令牌。',
  ],
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
      return mode === 'token'
        ? '访问令牌无效、已过期或已被撤销。'
        : '用户名或密码错误，请检查后重试。';
    }
    if (
      error.status === 409 &&
      mode === 'register' &&
      !KNOWN_AUTH_ERROR_MESSAGES.has(error.message)
    ) {
      return '该用户名已被注册，请更换用户名。';
    }
  }
  return getAccountErrorMessage(error);
}

/**
 * Convert an account-management failure into a user-facing message without login-specific mapping.
 *
 * @param error - Unknown error raised by an account operation.
 * @returns Translated known server detail, the original safe detail, or a generic fallback.
 */
export function getAccountErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) {
    return KNOWN_AUTH_ERROR_MESSAGES.get(error.message) ?? error.message;
  }
  return FALLBACK_AUTH_ERROR;
}

const FALLBACK_OIDC_ERROR = '单点登录失败，请重试。';
const OIDC_ERROR_MESSAGES = new Map<string, string>([
  ['unavailable', '单点登录暂不可用，请使用账号密码登录或联系管理员。'],
  ['state', '单点登录请求已失效或校验失败，请重新发起。'],
  ['denied', '授权已取消，或身份提供方拒绝了本次请求。'],
  ['provider', '与身份提供方通信失败，请稍后重试。'],
  ['email_unverified', '身份提供方中的邮箱尚未验证，无法创建账号。'],
  [
    'email_conflict',
    '该邮箱已被现有 LitRadar 账号使用。请先用账号密码登录，再在“账号与安全”中绑定单点登录。',
  ],
  ['account_conflict', '该单点登录身份已绑定其他 LitRadar 账号，或当前账号已绑定其他身份。'],
  ['expired_session', '登录会话已过期或已变更，请重新登录后再试。'],
  ['interrupted', '单点登录被中断：应用在浏览器登录期间被关闭，请重新发起单点登录。'],
  ['browser', '无法打开系统浏览器，请检查默认浏览器设置后重试。'],
]);

const NATIVE_OIDC_TIMEOUT_ERROR = '单点登录超时：浏览器中的登录未在 5 分钟内完成，请重新发起。';

/**
 * Convert a bounded single sign-on failure code into a user-facing message.
 *
 * @param code - Value of the `oidc_error` or failed `oidc_link` query parameter.
 * @returns Readable message; unknown codes get a generic failure text.
 */
export function getOidcErrorMessage(code: string): string {
  return OIDC_ERROR_MESSAGES.get(code) ?? FALLBACK_OIDC_ERROR;
}

/**
 * Convert a failed native single sign-on into a user-facing message.
 *
 * @param error - Rejection from the native single sign-on flow.
 * @returns Readable message for the login page.
 */
export function getNativeOidcErrorMessage(error: unknown): string {
  if (error instanceof PlatformBridgeError) {
    if (error.code === 'oidc_timeout') {
      return NATIVE_OIDC_TIMEOUT_ERROR;
    }
    return getOidcErrorMessage(error.code === 'oidc_error' ? (error.reason ?? '') : '');
  }
  if (error instanceof ApiError) {
    if (error.status === 404) {
      return getOidcErrorMessage('unavailable');
    }
    if (error.status === 429) {
      return '尝试次数过多，请稍后再试。';
    }
    if (error.status === 400) {
      return getOidcErrorMessage('state');
    }
  }
  return getAccountErrorMessage(error);
}
