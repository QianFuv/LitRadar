/**
 * Login loading, redirect, password visibility, and error feedback coverage.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import LoginClient from '@/src/routes/login-client';
import { MotionProvider } from '@/components/ui/motion';
import { getAuthErrorMessage } from '@/lib/auth-error';
import { ApiError } from '@/lib/api/client';
import { renderWithQuery, type QueryRenderResult } from '@/tests/render';

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
  getAuthProviders: vi.fn(),
  getInviteRequirement: vi.fn(),
  logoutRecoveryParam: '',
  nextParam: '',
  oidcErrorParam: null as string | null,
  navigate: vi.fn(),
}));

/**
 * Render the authentication page at a login URL built from the current search fixtures.
 *
 * @returns Testing Library render controls for the wrapped page.
 */
function renderLoginClient(): Promise<QueryRenderResult> {
  const search = new URLSearchParams({
    logout_recovery: loginPageMocks.logoutRecoveryParam,
    next: loginPageMocks.nextParam,
    ...(loginPageMocks.oidcErrorParam === null
      ? {}
      : { oidc_error: loginPageMocks.oidcErrorParam }),
  });
  return renderWithQuery(
    <MotionProvider>
      <LoginClient />
    </MotionProvider>,
    { url: `/login?${search.toString()}` },
  );
}

vi.mock('@tanstack/react-router', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tanstack/react-router')>()),
  useNavigate: () => loginPageMocks.navigate,
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => loginPageMocks.auth,
}));

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getAuthProviders: loginPageMocks.getAuthProviders,
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
  loginPageMocks.getAuthProviders
    .mockReset()
    .mockResolvedValue({ oidc_enabled: false, oidc_display_name: 'authentik' });
  loginPageMocks.nextParam = '';
  loginPageMocks.oidcErrorParam = null;
  loginPageMocks.logoutRecoveryParam = '';
  loginPageMocks.navigate.mockReset();
}

/**
 * Verify auth restoration and authenticated redirects never expose the editable form.
 */
async function hidesFormUntilAuthenticationSettles(): Promise<void> {
  loginPageMocks.auth.loading = true;
  loginPageMocks.nextParam = '/tracking';
  const view = await renderLoginClient();

  expect(screen.getByRole('status')).toHaveTextContent('正在检查登录状态');
  expect(screen.queryByLabelText(/用户名/)).not.toBeInTheDocument();

  loginPageMocks.auth.loading = false;
  loginPageMocks.auth.user = { id: 7, username: 'signed_in', is_admin: false };
  view.rerender(
    <MotionProvider>
      <LoginClient />
    </MotionProvider>,
  );

  await waitFor(() =>
    expect(loginPageMocks.navigate).toHaveBeenCalledWith({ href: '/tracking', replace: true }),
  );
  expect(screen.queryByLabelText(/用户名/)).not.toBeInTheDocument();
}

/**
 * Verify unsafe return paths fall back to the application home page.
 */
async function rejectsExternalReturnPaths(): Promise<void> {
  loginPageMocks.auth.user = { id: 8, username: 'signed_in', is_admin: false };
  loginPageMocks.nextParam = '//malicious.example';
  await renderLoginClient();

  await waitFor(() =>
    expect(loginPageMocks.navigate).toHaveBeenCalledWith({ href: '/', replace: true }),
  );
}

/**
 * Verify browser-normalized backslash return paths cannot become external navigation.
 */
async function rejectsBackslashReturnPaths(): Promise<void> {
  loginPageMocks.auth.user = { id: 9, username: 'signed_in', is_admin: false };
  for (const candidate of [
    '/\\malicious.example',
    '/safe\\..\\malicious.example',
    '/safe\tpath',
    '/safe\npath',
  ]) {
    loginPageMocks.nextParam = candidate;
    const view = await renderLoginClient();

    await waitFor(() =>
      expect(loginPageMocks.navigate).toHaveBeenCalledWith({ href: '/', replace: true }),
    );
    view.unmount();
    loginPageMocks.navigate.mockReset();
  }
}

/**
 * Verify safe internal return paths retain canonical query and fragment state.
 */
async function normalizesSafeInternalReturnPaths(): Promise<void> {
  loginPageMocks.auth.user = { id: 10, username: 'signed_in', is_admin: false };
  loginPageMocks.nextParam = '/favorites/../?view=favorites&folder=4#saved';
  await renderLoginClient();

  await waitFor(() =>
    expect(loginPageMocks.navigate).toHaveBeenCalledWith({
      href: '/?view=favorites&folder=4#saved',
      replace: true,
    }),
  );
}

/**
 * Verify form focus and password visibility preserve the current field value and autocomplete.
 */
async function focusesAndRevealsPasswordSafely(): Promise<void> {
  const user = userEvent.setup();
  await renderLoginClient();

  const usernameInput = screen.getByLabelText('用户名或邮箱');
  const passwordInput = screen.getByLabelText('密码');
  const authForm = screen.getByRole('form', { name: '身份验证表单' });
  const modeToggle = screen.getByRole('button', { name: '注册' });
  expect(screen.getByLabelText('LitRadar')).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '登录' })).toBeInTheDocument();
  expect(document.querySelector('[data-auth-header-mode="login"]')).toBeInTheDocument();
  expect(usernameInput).toHaveFocus();
  expect(passwordInput).toHaveAttribute('autocomplete', 'current-password');

  await user.type(passwordInput, 'kept-password');
  await user.click(screen.getByRole('button', { name: '显示密码' }));

  expect(passwordInput).toHaveAttribute('type', 'text');
  expect(passwordInput).toHaveAttribute('autocomplete', 'current-password');
  expect(passwordInput).toHaveValue('kept-password');
  expect(screen.getByRole('button', { name: '隐藏密码' })).toBeInTheDocument();

  await user.click(modeToggle);
  expect(screen.getByRole('form', { name: '身份验证表单' })).toBe(authForm);
  expect(screen.getByLabelText('用户名')).toBe(usernameInput);
  expect(screen.getByLabelText('密码')).toBe(passwordInput);
  expect(screen.getByRole('button', { name: '登录' })).toBe(modeToggle);
  expect(modeToggle).toHaveFocus();
  expect(screen.getByRole('heading', { name: '注册' })).toBeInTheDocument();
  await waitFor(() =>
    expect(document.querySelector('[data-auth-header-mode="register"]')).toBeInTheDocument(),
  );
  expect(document.querySelector('[data-auth-conditional="invite-code"]')).toBeInTheDocument();
  expect(passwordInput).toHaveAttribute('autocomplete', 'new-password');
  expect(passwordInput).toHaveValue('kept-password');
}

/**
 * Verify typed API errors map to actionable messages while unknown details remain visible.
 */
function mapsAuthenticationErrors(): void {
  expect(getAuthErrorMessage(new ApiError('Invalid username or password', 401), 'login')).toBe(
    '用户名或密码错误，请检查后重试。',
  );
  expect(getAuthErrorMessage(new ApiError('Username already exists', 409), 'register')).toBe(
    '该用户名已被注册，请更换用户名。',
  );
  expect(getAuthErrorMessage(new ApiError('Invite code is required', 400), 'register')).toBe(
    '请输入邀请码。',
  );
  expect(getAuthErrorMessage(new ApiError('Invalid or used invite code', 400), 'register')).toBe(
    '邀请码无效或已被使用，请确认后重试。',
  );
  expect(
    getAuthErrorMessage(new ApiError('Administrator bootstrap is required', 400), 'register'),
  ).toBe('系统尚未完成管理员初始化，请联系管理员。');
  expect(
    getAuthErrorMessage(
      new ApiError('Username must be 3-32 alphanumeric or underscore characters', 400),
      'register',
    ),
  ).toBe('用户名需为 3–32 位字母、数字或下划线。');
  expect(
    getAuthErrorMessage(new ApiError('Password must be at least 12 characters', 400), 'register'),
  ).toBe('密码至少需要 12 个字符。');
  expect(
    getAuthErrorMessage(
      new ApiError('Too many authentication attempts; try again later', 429),
      'login',
    ),
  ).toBe('尝试次数过多，请稍后再试。');
  expect(getAuthErrorMessage(new ApiError('Specific server detail', 400), 'register')).toBe(
    'Specific server detail',
  );
  expect(getAuthErrorMessage({ unexpected: true }, 'login')).toBe('操作失败，请重试');
}

/**
 * Verify mapped login failures are announced and do not navigate.
 */
async function announcesLoginFailures(): Promise<void> {
  loginPageMocks.auth.login.mockRejectedValue(new ApiError('Invalid username or password', 401));
  const user = userEvent.setup();
  await renderLoginClient();

  await user.type(screen.getByLabelText('用户名或邮箱'), 'reader');
  await user.type(screen.getByLabelText('密码'), 'wrong-password');
  await user.click(screen.getByRole('button', { name: '登录' }));

  const submitButton = screen.getByRole('button', { name: '登录' });
  const errorFeedback = await screen.findByRole('alert');
  expect(errorFeedback).toHaveTextContent('用户名或密码错误，请检查后重试。');
  expect(errorFeedback).toHaveAttribute('data-auth-feedback', 'error');
  expect(loginPageMocks.navigate).not.toHaveBeenCalled();

  loginPageMocks.auth.login.mockRejectedValue({ unexpected: true });
  await user.click(submitButton);
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('操作失败，请重试'));
}

/**
 * Verify the public login route exposes a persisted logout warning and recovery entry.
 */
async function exposesPersistedLogoutWarning(): Promise<void> {
  loginPageMocks.auth.logoutWarning = {
    occurredAt: 1234,
    requestId: 'logout-request-3',
  };
  await renderLoginClient();

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
  await renderLoginClient();

  await user.type(screen.getByLabelText('用户名或邮箱'), '  reader@example.test  ');
  await user.type(screen.getByLabelText('密码'), 'correct-password');
  await user.click(screen.getByRole('button', { name: '登录' }));

  const pendingButton = screen.getByRole('button', { name: '请稍候…' });
  expect(pendingButton).toBeDisabled();
  await user.click(pendingButton);
  expect(loginPageMocks.auth.login).toHaveBeenCalledOnce();
  expect(loginPageMocks.auth.login).toHaveBeenCalledWith('reader@example.test', 'correct-password');

  resolveLogin?.();
  await waitFor(() =>
    expect(loginPageMocks.navigate).toHaveBeenCalledWith({
      href: '/favorites?folder=4',
      replace: true,
    }),
  );
}

/**
 * Verify invited registration submits normalized identity fields and enters the requested route.
 */
async function registersInvitedUserAndReturnsToRequestedPath(): Promise<void> {
  loginPageMocks.nextParam = '/tracking';
  const user = userEvent.setup();
  await renderLoginClient();

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
      '',
    ),
  );
  expect(loginPageMocks.navigate).toHaveBeenCalledWith({ href: '/tracking', replace: true });
}

/**
 * Verify the optional registration email is offered only while registering and is trimmed.
 */
async function registersWithOptionalEmail(): Promise<void> {
  const user = userEvent.setup();
  await renderLoginClient();

  expect(screen.queryByLabelText('邮箱（可选）')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '注册' }));
  const emailInput = screen.getByLabelText('邮箱（可选）');
  expect(emailInput).toHaveAttribute('type', 'email');
  expect(emailInput).not.toBeRequired();
  await user.type(screen.getByLabelText('用户名'), 'reader');
  await user.type(screen.getByLabelText('密码'), 'password-1234');
  await user.type(emailInput, ' r@e.test ');
  await user.type(screen.getByLabelText('邀请码'), 'code');
  await user.click(screen.getByRole('button', { name: '注册' }));

  await waitFor(() =>
    expect(loginPageMocks.auth.register).toHaveBeenCalledWith(
      'reader',
      'password-1234',
      'code',
      'r@e.test',
    ),
  );
}

/**
 * Verify single sign-on is a full-page link that preserves the normalized return path.
 */
async function offersSingleSignOnWhenEnabled(): Promise<void> {
  loginPageMocks.nextParam = '/favorites?folder=4';
  loginPageMocks.getAuthProviders.mockResolvedValue({
    oidc_enabled: true,
    oidc_display_name: 'authentik',
  });
  await renderLoginClient();

  const ssoLink = await screen.findByRole('link', { name: '使用 authentik 登录' });
  const target = new URL(ssoLink.getAttribute('href') ?? '');
  expect(target.pathname).toBe('/api/auth/oidc/login');
  expect(target.searchParams.get('next')).toBe('/favorites?folder=4');
}

/**
 * Verify the single sign-on entry stays hidden when disabled or during logout recovery.
 */
async function hidesSingleSignOnWhenUnavailable(): Promise<void> {
  const view = await renderLoginClient();
  await waitFor(() => expect(loginPageMocks.getAuthProviders).toHaveBeenCalled());
  expect(screen.queryByRole('link', { name: /登录$/ })).not.toBeInTheDocument();
  view.unmount();

  loginPageMocks.logoutRecoveryParam = '1';
  loginPageMocks.getAuthProviders.mockClear().mockResolvedValue({
    oidc_enabled: true,
    oidc_display_name: 'authentik',
  });
  await renderLoginClient();
  expect(screen.getByRole('heading', { name: '撤销全部会话' })).toBeInTheDocument();
  expect(loginPageMocks.getAuthProviders).not.toHaveBeenCalled();
  expect(document.querySelector('[data-auth-oidc]')).toBeNull();
}

const OIDC_ERROR_CASES = [
  ['unavailable', '单点登录暂不可用'],
  ['state', '请求已失效或校验失败'],
  ['denied', '授权已取消'],
  ['provider', '与身份提供方通信失败'],
  ['email_unverified', '邮箱尚未验证'],
  ['email_conflict', '该邮箱已被现有 LitRadar 账号使用'],
  ['account_conflict', '已绑定其他 LitRadar 账号'],
  ['expired_session', '登录会话已过期'],
  ['something_new', '单点登录失败，请重试。'],
] as const;

/**
 * Verify each callback failure code is announced as readable text.
 *
 * @param code - `oidc_error` query value.
 * @param expectedText - Fragment of the expected message.
 */
async function announcesOidcError(code: string, expectedText: string): Promise<void> {
  loginPageMocks.oidcErrorParam = code;
  await renderLoginClient();

  const alert = screen.getByRole('alert');
  expect(alert).toHaveAttribute('data-auth-feedback', 'oidc-error');
  expect(alert).toHaveTextContent(expectedText);
}

/**
 * Verify logout recovery reauthenticates, revokes every session, and stays logged out.
 */
async function reauthenticatesForLogoutRecovery(): Promise<void> {
  loginPageMocks.logoutRecoveryParam = '1';
  const user = userEvent.setup();
  await renderLoginClient();

  expect(screen.getByRole('heading', { name: '撤销全部会话' })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '注册' })).not.toBeInTheDocument();
  await user.type(screen.getByLabelText('用户名或邮箱'), '  recovery_user  ');
  await user.type(screen.getByLabelText('密码'), 'recovery-password');
  await user.click(screen.getByRole('button', { name: '重新认证并撤销全部会话' }));

  await waitFor(() =>
    expect(loginPageMocks.auth.recoverLogout).toHaveBeenCalledWith(
      'recovery_user',
      'recovery-password',
    ),
  );
  expect(await screen.findByRole('status')).toHaveTextContent(
    '全部会话和个人访问令牌已撤销。现在可以重新登录。',
  );
  expect(document.querySelector('[data-auth-state="recovery-complete"]')).toBeInTheDocument();
  await waitFor(() =>
    expect(screen.queryByRole('form', { name: '会话撤销表单' })).not.toBeInTheDocument(),
  );
  expect(screen.getByRole('link', { name: '返回登录' })).toHaveAttribute('href', '/login');
  expect(loginPageMocks.navigate).not.toHaveBeenCalled();
}

beforeEach(resetLoginPageMocks);

describe('login page', () => {
  test('hides the form until authentication settles', hidesFormUntilAuthenticationSettles);
  test('rejects external return paths', rejectsExternalReturnPaths);
  test('rejects browser-normalized backslash return paths', rejectsBackslashReturnPaths);
  test('normalizes safe internal return paths', normalizesSafeInternalReturnPaths);
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
  test('registers with an optional email', registersWithOptionalEmail, 15_000);
  test('offers single sign-on when enabled', offersSingleSignOnWhenEnabled);
  test('hides single sign-on when unavailable', hidesSingleSignOnWhenUnavailable);
  test.each(OIDC_ERROR_CASES)('announces the %s single sign-on error', announcesOidcError);
});
