/**
 * Login loading, redirect, password visibility, and error feedback coverage.
 */

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import LoginClient from '@/app/login/login-client';
import { getAuthErrorMessage } from '@/lib/auth-error';
import { ApiError } from '@/lib/api/client';

type MockUser = {
  id: number;
  username: string;
  is_admin: boolean;
};

const loginPageMocks = vi.hoisted(() => ({
  auth: {
    loading: false,
    login: vi.fn(),
    logoutWarning: null as { occurredAt: number; requestId: string | null } | null,
    recoverLogout: vi.fn(),
    register: vi.fn(),
    user: null as MockUser | null,
  },
  getInviteRequirement: vi.fn(),
  logoutRecoveryParam: '',
  nextParam: '',
  replace: vi.fn(),
}));

vi.mock('next/navigation', () => ({
  useRouter: () => ({ replace: loginPageMocks.replace }),
  useSearchParams: () =>
    new URLSearchParams({
      logout_recovery: loginPageMocks.logoutRecoveryParam,
      next: loginPageMocks.nextParam,
    }),
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => loginPageMocks.auth,
}));

vi.mock('@/lib/api', () => ({
  getInviteRequirement: loginPageMocks.getInviteRequirement,
}));

/**
 * Restore the default anonymous login fixture.
 */
function resetLoginPageMocks(): void {
  loginPageMocks.auth.loading = false;
  loginPageMocks.auth.logoutWarning = null;
  loginPageMocks.auth.login.mockReset().mockResolvedValue(undefined);
  loginPageMocks.auth.recoverLogout.mockReset().mockResolvedValue(undefined);
  loginPageMocks.auth.register.mockReset().mockResolvedValue(undefined);
  loginPageMocks.auth.user = null;
  loginPageMocks.getInviteRequirement
    .mockReset()
    .mockResolvedValue({ required: true, bootstrap_required: false });
  loginPageMocks.nextParam = '';
  loginPageMocks.logoutRecoveryParam = '';
  loginPageMocks.replace.mockReset();
}

/**
 * Verify auth restoration and authenticated redirects never expose the editable form.
 */
async function hidesFormUntilAuthenticationSettles(): Promise<void> {
  loginPageMocks.auth.loading = true;
  loginPageMocks.nextParam = '/tracking';
  const view = render(<LoginClient />);

  expect(screen.getByRole('status')).toHaveTextContent('正在检查登录状态');
  expect(screen.queryByLabelText('用户名')).not.toBeInTheDocument();

  loginPageMocks.auth.loading = false;
  loginPageMocks.auth.user = { id: 7, username: 'signed_in', is_admin: false };
  view.rerender(<LoginClient />);

  await waitFor(() => expect(loginPageMocks.replace).toHaveBeenCalledWith('/tracking'));
  expect(screen.queryByLabelText('用户名')).not.toBeInTheDocument();
}

/**
 * Verify unsafe return paths fall back to the application home page.
 */
async function rejectsExternalReturnPaths(): Promise<void> {
  loginPageMocks.auth.user = { id: 8, username: 'signed_in', is_admin: false };
  loginPageMocks.nextParam = '//malicious.example';
  render(<LoginClient />);

  await waitFor(() => expect(loginPageMocks.replace).toHaveBeenCalledWith('/'));
}

/**
 * Verify form focus and password visibility preserve the current field value and autocomplete.
 */
async function focusesAndRevealsPasswordSafely(): Promise<void> {
  const user = userEvent.setup();
  render(<LoginClient />);

  const usernameInput = screen.getByLabelText('用户名');
  const passwordInput = screen.getByLabelText('密码');
  expect(usernameInput).toHaveFocus();
  expect(passwordInput).toHaveAttribute('autocomplete', 'current-password');

  await user.type(passwordInput, 'kept-password');
  await user.click(screen.getByRole('button', { name: '显示密码' }));

  expect(passwordInput).toHaveAttribute('type', 'text');
  expect(passwordInput).toHaveAttribute('autocomplete', 'current-password');
  expect(passwordInput).toHaveValue('kept-password');
  expect(screen.getByRole('button', { name: '隐藏密码' })).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '注册' }));
  expect(passwordInput).toHaveAttribute('autocomplete', 'new-password');
  expect(passwordInput).toHaveValue('kept-password');
}

/**
 * Verify typed API errors map to actionable messages while unknown details remain visible.
 */
function mapsAuthenticationErrors(): void {
  expect(
    getAuthErrorMessage(new ApiError('Invalid username or password', 401, null, null), 'login'),
  ).toBe('用户名或密码错误，请检查后重试。');
  expect(
    getAuthErrorMessage(new ApiError('Username already exists', 409, null, null), 'register'),
  ).toBe('该用户名已被注册，请更换用户名。');
  expect(
    getAuthErrorMessage(new ApiError('Invite code is required', 400, null, null), 'register'),
  ).toBe('请输入邀请码。');
  expect(
    getAuthErrorMessage(new ApiError('Invalid or used invite code', 400, null, null), 'register'),
  ).toBe('邀请码无效或已被使用，请确认后重试。');
  expect(
    getAuthErrorMessage(
      new ApiError('Administrator bootstrap is required', 400, null, null),
      'register',
    ),
  ).toBe('系统尚未完成管理员初始化，请联系管理员。');
  expect(
    getAuthErrorMessage(
      new ApiError('Username must be 3-32 alphanumeric or underscore characters', 400, null, null),
      'register',
    ),
  ).toBe('用户名需为 3–32 位字母、数字或下划线。');
  expect(
    getAuthErrorMessage(
      new ApiError('Password must be at least 12 characters', 400, null, null),
      'register',
    ),
  ).toBe('密码至少需要 12 个字符。');
  expect(
    getAuthErrorMessage(
      new ApiError('Too many authentication attempts; try again later', 429, null, null),
      'login',
    ),
  ).toBe('尝试次数过多，请稍后再试。');
  expect(
    getAuthErrorMessage(new ApiError('Specific server detail', 400, null, null), 'register'),
  ).toBe('Specific server detail');
  expect(getAuthErrorMessage({ unexpected: true }, 'login')).toBe('操作失败，请重试');
}

/**
 * Verify mapped login failures are announced and do not navigate.
 */
async function announcesLoginFailures(): Promise<void> {
  loginPageMocks.auth.login.mockRejectedValue(
    new ApiError('Invalid username or password', 401, null, null),
  );
  const user = userEvent.setup();
  render(<LoginClient />);

  await user.type(screen.getByLabelText('用户名'), 'reader');
  await user.type(screen.getByLabelText('密码'), 'wrong-password');
  await user.click(screen.getByRole('button', { name: '登录' }));

  const submitButton = screen.getByRole('button', { name: '登录' });
  expect(await screen.findByRole('alert')).toHaveTextContent('用户名或密码错误，请检查后重试。');
  expect(loginPageMocks.replace).not.toHaveBeenCalled();

  loginPageMocks.auth.login.mockRejectedValue({ unexpected: true });
  await user.click(submitButton);
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('操作失败，请重试'));
}

/**
 * Verify the public login route exposes a persisted logout warning and recovery entry.
 */
function exposesPersistedLogoutWarning(): void {
  loginPageMocks.auth.logoutWarning = {
    occurredAt: 1234,
    requestId: 'logout-request-3',
  };
  render(<LoginClient />);

  const warning = screen.getByRole('alert');
  expect(warning).toHaveTextContent('服务端会话撤销未确认');
  expect(warning).toHaveTextContent('请求 ID：logout-request-3');
  expect(screen.getByRole('link', { name: '重新认证并撤销全部会话' })).toHaveAttribute(
    'href',
    '/login?logout_recovery=1',
  );
}

/**
 * Verify a successful login trims identity input, blocks duplicate submission, and returns safely.
 */
async function submitsLoginOnceAndReturnsToProtectedPath(): Promise<void> {
  let resolveLogin: (() => void) | undefined;
  loginPageMocks.nextParam = '/favorites?folder=4';
  loginPageMocks.auth.login.mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        resolveLogin = resolve;
      }),
  );
  const user = userEvent.setup();
  render(<LoginClient />);

  await user.type(screen.getByLabelText('用户名'), '  reader  ');
  await user.type(screen.getByLabelText('密码'), 'correct-password');
  await user.click(screen.getByRole('button', { name: '登录' }));

  const pendingButton = screen.getByRole('button', { name: '请稍候…' });
  expect(pendingButton).toBeDisabled();
  await user.click(pendingButton);
  expect(loginPageMocks.auth.login).toHaveBeenCalledOnce();
  expect(loginPageMocks.auth.login).toHaveBeenCalledWith('reader', 'correct-password');

  resolveLogin?.();
  await waitFor(() => expect(loginPageMocks.replace).toHaveBeenCalledWith('/favorites?folder=4'));
}

/**
 * Verify invited registration submits normalized identity fields and enters the requested route.
 */
async function registersInvitedUserAndReturnsToRequestedPath(): Promise<void> {
  loginPageMocks.nextParam = '/tracking';
  const user = userEvent.setup();
  render(<LoginClient />);

  await user.click(screen.getByRole('button', { name: '注册' }));
  await user.type(screen.getByLabelText('用户名'), '  invited_reader  ');
  await user.type(screen.getByLabelText('密码'), 'registration-password');
  await user.type(screen.getByLabelText('邀请码'), '  invite-code  ');
  await user.click(screen.getByRole('button', { name: '注册' }));

  await waitFor(() =>
    expect(loginPageMocks.auth.register).toHaveBeenCalledWith(
      'invited_reader',
      'registration-password',
      'invite-code',
    ),
  );
  expect(loginPageMocks.replace).toHaveBeenCalledWith('/tracking');
}

/**
 * Verify logout recovery reauthenticates, revokes every session, and stays logged out.
 */
async function reauthenticatesForLogoutRecovery(): Promise<void> {
  loginPageMocks.logoutRecoveryParam = '1';
  const user = userEvent.setup();
  render(<LoginClient />);

  expect(screen.getByRole('heading', { name: '撤销全部会话' })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '注册' })).not.toBeInTheDocument();
  await user.type(screen.getByLabelText('用户名'), '  recovery_user  ');
  await user.type(screen.getByLabelText('密码'), 'recovery-password');
  await user.click(screen.getByRole('button', { name: '重新认证并撤销全部会话' }));

  await waitFor(() =>
    expect(loginPageMocks.auth.recoverLogout).toHaveBeenCalledWith(
      'recovery_user',
      'recovery-password',
    ),
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '全部会话和个人访问令牌已撤销。现在可以重新登录。',
  );
  expect(screen.getByRole('link', { name: '返回登录' })).toHaveAttribute('href', '/login');
  expect(loginPageMocks.replace).not.toHaveBeenCalled();
}

beforeEach(resetLoginPageMocks);

describe('login page', () => {
  test('hides the form until authentication settles', hidesFormUntilAuthenticationSettles);
  test('rejects external return paths', rejectsExternalReturnPaths);
  test('focuses username and toggles password visibility safely', focusesAndRevealsPasswordSafely);
  test('maps known authentication errors and preserves unknown details', mapsAuthenticationErrors);
  test('announces mapped login failures', announcesLoginFailures);
  test('exposes a persisted logout warning', exposesPersistedLogoutWarning);
  test(
    'submits a successful login once and returns to the protected path',
    submitsLoginOnceAndReturnsToProtectedPath,
  );
  test(
    'registers an invited user and returns to the requested path',
    registersInvitedUserAndReturnsToRequestedPath,
  );
  test('reauthenticates for logout recovery', reauthenticatesForLogoutRecovery);
});
