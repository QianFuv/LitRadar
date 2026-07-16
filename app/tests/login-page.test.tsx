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
    register: vi.fn(),
    user: null as MockUser | null,
  },
  getInviteRequirement: vi.fn(),
  nextParam: '',
  replace: vi.fn(),
}));

vi.mock('next/navigation', () => ({
  useRouter: () => ({ replace: loginPageMocks.replace }),
  useSearchParams: () => new URLSearchParams({ next: loginPageMocks.nextParam }),
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
  loginPageMocks.auth.login.mockReset().mockResolvedValue(undefined);
  loginPageMocks.auth.register.mockReset().mockResolvedValue(undefined);
  loginPageMocks.auth.user = null;
  loginPageMocks.getInviteRequirement
    .mockReset()
    .mockResolvedValue({ required: true, bootstrap_required: false });
  loginPageMocks.nextParam = '';
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

beforeEach(resetLoginPageMocks);

describe('login page', () => {
  test('hides the form until authentication settles', hidesFormUntilAuthenticationSettles);
  test('rejects external return paths', rejectsExternalReturnPaths);
  test('focuses username and toggles password visibility safely', focusesAndRevealsPasswordSafely);
  test('maps known authentication errors and preserves unknown details', mapsAuthenticationErrors);
  test('announces mapped login failures', announcesLoginFailures);
});
