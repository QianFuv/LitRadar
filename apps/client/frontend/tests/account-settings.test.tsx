/**
 * Account sign-in methods, password, invite, access-token, and CNKI settings behavior coverage.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import type { AuthUser } from '@/lib/api';

const accountSettingsMocks = vi.hoisted(() => ({
  copy: vi.fn().mockResolvedValue(undefined),
  logout: vi.fn().mockResolvedValue(undefined),
  updateUser: vi.fn(),
  user: null as AuthUser | null,
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({
    logout: accountSettingsMocks.logout,
    updateUser: accountSettingsMocks.updateUser,
    user: accountSettingsMocks.user,
  }),
}));

import { AccessTokensCard } from '@/components/settings/access-tokens-card';
import { AccountSignInCard } from '@/components/settings/account-card';
import { CnkiSettingsCard } from '@/components/settings/cnki-card';
import { InviteCodeCard } from '@/components/settings/invite-code-card';
import { PasswordCard } from '@/components/settings/password-card';
import type {
  CnkiLoginPollResponse,
  CnkiLoginStartResponse,
  CnkiSessionStatus,
  InviteCode,
} from '@/lib/api';
import { createAccountScenarioHandlers, OIDC_AUTHORIZATION_URL } from '@/tests/mocks/handlers/auth';
import { createAuthUserScenario } from '@/tests/mocks/scenarios';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const EMPTY_CNKI_SESSION: CnkiSessionStatus = {
  configured: false,
  status: 'empty',
  has_bff_user_token: false,
  expires_at: null,
  seconds_remaining: null,
  cookie_names: [],
  updated_at: null,
  last_used_at: null,
};

/**
 * Build a waiting CNKI login challenge.
 *
 * @param sequence - Request sequence used in stable fixture values.
 * @returns CNKI start response.
 */
function createCnkiLoginStart(sequence: number): CnkiLoginStartResponse {
  return {
    uuid: `uuid-${sequence}`,
    status: '等待扫码',
    qr_code: `qr-payload-${sequence}`,
    session: {
      ...EMPTY_CNKI_SESSION,
      configured: true,
      status: 'waiting_scan',
      updated_at: 1_700_000_000 + sequence,
    },
  };
}

/**
 * Build an active CNKI poll response.
 *
 * @returns Successful CNKI poll response.
 */
function createActiveCnkiPoll(): CnkiLoginPollResponse {
  return {
    status: 'active',
    session: {
      configured: true,
      status: 'active',
      has_bff_user_token: true,
      expires_at: 2_000_000_000,
      seconds_remaining: 3600,
      cookie_names: ['session'],
      updated_at: 1_900_000_000,
      last_used_at: 1_900_000_100,
    },
  };
}

/**
 * Render the CNKI settings owner with shared copy feedback disabled.
 */
async function renderCnkiSettings(): Promise<void> {
  await renderWithQuery(
    <CnkiSettingsCard userId={21} copyFeedback={null} handleCopy={accountSettingsMocks.copy} />,
  );
}

/**
 * Install account handlers and expose one local user to the mocked authentication context.
 *
 * @param overrides - Authenticated user fields.
 * @param providerOverrides - Sign-in provider summary fields.
 */
function installAccountUser(
  overrides: Partial<AuthUser> = {},
  providerOverrides: { oidc_enabled?: boolean; oidc_display_name?: string } = {},
): void {
  const user = createAuthUserScenario({ id: 21, username: 'account_user', ...overrides });
  accountSettingsMocks.user = user;
  server.use(...createAccountScenarioHandlers(user, providerOverrides));
}

/**
 * Verify a local account can set, reject, and clear its email through the server projection.
 */
async function managesLocalAccountEmail(): Promise<void> {
  const emailPayloads: unknown[] = [];
  installAccountUser();
  server.events.on('request:start', async ({ request }) => {
    if (request.method === 'PUT' && request.url.endsWith('/api/auth/email')) {
      emailPayloads.push(await request.clone().json());
    }
  });
  const user = userEvent.setup();
  await renderWithQuery(<AccountSignInCard oidcLinkResult={null} />);

  const emailInput = screen.getByLabelText('邮箱');
  const saveButton = screen.getByRole('button', { name: '保存邮箱' });
  expect(saveButton).toBeDisabled();
  await user.type(emailInput, 'taken@example.test');
  await user.click(saveButton);
  expect(await screen.findByRole('alert')).toHaveTextContent('该邮箱已被其他账号使用');
  expect(accountSettingsMocks.updateUser).not.toHaveBeenCalled();

  await user.clear(emailInput);
  await user.type(emailInput, '  Reader@Example.test ');
  await user.click(saveButton);
  expect(await screen.findByRole('status')).toHaveTextContent('邮箱已更新');
  expect(accountSettingsMocks.updateUser).toHaveBeenLastCalledWith(
    expect.objectContaining({ id: 21, email: 'reader@example.test' }),
  );
  expect(emailInput).toHaveValue('reader@example.test');

  accountSettingsMocks.user = createAuthUserScenario({
    id: 21,
    username: 'account_user',
    email: 'reader@example.test',
  });
  await user.clear(emailInput);
  await user.click(saveButton);
  expect(await screen.findByText('邮箱已清除')).toBeInTheDocument();
  expect(accountSettingsMocks.updateUser).toHaveBeenLastCalledWith(
    expect.objectContaining({ email: null }),
  );
  expect(emailPayloads).toEqual([
    { email: 'taken@example.test' },
    { email: 'Reader@Example.test' },
    { email: null },
  ]);
}

/**
 * Verify linking navigates the whole page to the provider authorization URL.
 */
async function startsSingleSignOnLink(): Promise<void> {
  const assign = vi.fn();
  const { href, origin, protocol, host, hostname, port, pathname, search, hash } = window.location;
  vi.stubGlobal('location', {
    href,
    origin,
    protocol,
    host,
    hostname,
    port,
    pathname,
    search,
    hash,
    assign,
  });
  installAccountUser();
  const user = userEvent.setup();
  await renderWithQuery(<AccountSignInCard oidcLinkResult={null} />);

  const linkButton = await screen.findByRole('button', { name: '绑定 authentik' });
  await waitFor(() => expect(linkButton).toBeEnabled());
  await user.click(linkButton);

  await waitFor(() => expect(assign).toHaveBeenCalledWith(OIDC_AUTHORIZATION_URL));
  expect(linkButton).toBeDisabled();
}

/**
 * Verify linking is unavailable until the server enables single sign-on.
 */
async function disablesLinkWhenSingleSignOnIsOff(): Promise<void> {
  installAccountUser({}, { oidc_enabled: false });
  await renderWithQuery(<AccountSignInCard oidcLinkResult={null} />);

  expect(await screen.findByText('服务器尚未启用单点登录')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: /^绑定/ })).toBeDisabled();
}

/**
 * Verify a linked local account confirms before unlinking and adopts the returned user.
 */
async function unlinksSingleSignOnAfterConfirmation(): Promise<void> {
  let unlinkCount = 0;
  installAccountUser({ is_oidc_linked: true });
  server.events.on('request:start', ({ request }) => {
    if (request.method === 'DELETE' && request.url.endsWith('/api/auth/oidc/link')) {
      unlinkCount += 1;
    }
  });
  const user = userEvent.setup();
  await renderWithQuery(<AccountSignInCard oidcLinkResult={null} />);

  expect(screen.getByText('已绑定')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '解除绑定' }));
  const dialog = await screen.findByRole('alertdialog');
  expect(unlinkCount).toBe(0);
  await user.click(within(dialog).getByRole('button', { name: '解除绑定' }));

  await waitFor(() =>
    expect(accountSettingsMocks.updateUser).toHaveBeenCalledWith(
      expect.objectContaining({ id: 21, is_oidc_linked: false }),
    ),
  );
  expect(unlinkCount).toBe(1);
  expect(await screen.findByRole('status')).toHaveTextContent('已解除 authentik 绑定');
}

/**
 * Verify single sign-on accounts see a read-only email and cannot unlink or change it.
 */
async function showsReadOnlyOidcAccount(): Promise<void> {
  installAccountUser({ auth_kind: 'oidc', email: 'sso@example.test', is_oidc_linked: true });
  await renderWithQuery(<AccountSignInCard oidcLinkResult={null} />);

  expect(screen.getByText('sso@example.test')).toBeInTheDocument();
  expect(screen.queryByRole('textbox', { name: '邮箱' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '保存邮箱' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: '解除绑定' })).not.toBeInTheDocument();
  expect(screen.getByText('单点登录账号无法解除绑定')).toBeInTheDocument();
}

/**
 * Verify the link callback result is announced as success or a readable failure.
 */
async function announcesLinkCallbackResult(): Promise<void> {
  installAccountUser({ is_oidc_linked: true });
  const view = await renderWithQuery(<AccountSignInCard oidcLinkResult="linked" />);
  expect(screen.getByRole('status')).toHaveTextContent('之后可以使用它登录');
  view.unmount();

  installAccountUser();
  await renderWithQuery(<AccountSignInCard oidcLinkResult="account_conflict" />);
  expect(screen.getByRole('alert')).toHaveTextContent('已绑定其他 LitRadar 账号');
}

/**
 * Verify password submission is bounded, reports success, and delays logout until completion.
 */
async function changesPasswordThenLogsOut(): Promise<void> {
  let submittedPayload: unknown;
  let releaseRequest: (() => void) | undefined;
  const requestGate = new Promise<void>((resolve) => {
    releaseRequest = resolve;
  });
  server.use(
    http.post('http://localhost/api/auth/change-password', async ({ request }) => {
      submittedPayload = await request.json();
      await requestGate;
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  await renderWithQuery(<PasswordCard />);

  await user.type(screen.getByLabelText('原密码'), 'old-password');
  await user.type(screen.getByLabelText('新密码'), 'new-password-123');
  await user.click(screen.getByRole('button', { name: '修改密码' }));
  expect(screen.getByRole('button', { name: '修改密码' })).toBeDisabled();
  expect(accountSettingsMocks.logout).not.toHaveBeenCalled();

  releaseRequest?.();
  expect(await screen.findByRole('status')).toHaveTextContent('密码修改成功，请重新登录');
  expect(submittedPayload).toEqual({
    old_password: 'old-password',
    new_password: 'new-password-123',
  });
  await waitFor(() => expect(accountSettingsMocks.logout).toHaveBeenCalledOnce(), {
    timeout: 2500,
  });
}

/**
 * Verify password errors remain visible and never schedule logout.
 */
async function reportsPasswordChangeFailure(): Promise<void> {
  server.use(
    http.post('http://localhost/api/auth/change-password', () =>
      HttpResponse.json({ detail: 'Current password is incorrect' }, { status: 400 }),
    ),
  );
  const user = userEvent.setup();
  await renderWithQuery(<PasswordCard />);

  await user.type(screen.getByLabelText('原密码'), 'wrong-password');
  await user.type(screen.getByLabelText('新密码'), 'new-password-123');
  await user.click(screen.getByRole('button', { name: '修改密码' }));

  expect(await screen.findByRole('alert')).toHaveTextContent('Current password is incorrect');
  expect(accountSettingsMocks.logout).not.toHaveBeenCalled();
}

/**
 * Verify personal invite generation, rotation, and revocation refetch committed lifecycle state.
 */
async function generatesAndCopiesPersonalInvite(): Promise<void> {
  let inviteCode: InviteCode | null = null;
  server.use(
    http.get('http://localhost/api/auth/invite-code', () => HttpResponse.json(inviteCode)),
    http.post('http://localhost/api/auth/invite-code', () => {
      inviteCode = {
        id: 41,
        code: 'personal-invite',
        used: false,
        status: 'active',
        expires_at: 2_200_000_000,
        revoked_at: null,
        max_uses: 1,
        use_count: 0,
        created_at: 1_900_000_000,
      };
      return HttpResponse.json(inviteCode);
    }),
    http.post('http://localhost/api/auth/invite-code/rotate', () => {
      inviteCode = {
        id: 42,
        code: 'rotated-personal-invite',
        used: false,
        status: 'active',
        expires_at: 2_200_100_000,
        revoked_at: null,
        max_uses: 1,
        use_count: 0,
        created_at: 1_900_100_000,
      };
      return HttpResponse.json(inviteCode);
    }),
    http.delete('http://localhost/api/auth/invite-code', () => {
      if (inviteCode) {
        inviteCode = {
          ...inviteCode,
          status: 'revoked',
          revoked_at: 1_900_200_000,
        };
      }
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <InviteCodeCard copyFeedback={null} handleCopy={accountSettingsMocks.copy} />,
  );

  await user.click(await screen.findByRole('button', { name: '生成邀请码' }));
  expect(await screen.findByText('personal-invite')).toBeInTheDocument();
  expect(screen.getByText('可用')).toBeInTheDocument();
  expect(screen.getByText('已使用 0/1 次')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '复制邀请码' }));
  expect(accountSettingsMocks.copy).toHaveBeenCalledWith(
    'personal-invite',
    '邀请码已复制。',
    'invite',
  );

  await user.click(screen.getByRole('button', { name: '轮换邀请码' }));
  expect(await screen.findByText('rotated-personal-invite')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '撤销邀请码' }));
  const confirmation = screen.getByRole('alertdialog', { name: '撤销邀请码？' });
  await user.click(within(confirmation).getByRole('button', { name: '确认撤销' }));
  expect(await screen.findByText('已撤销')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '复制邀请码' })).toBeDisabled();
}

/**
 * Verify invite generation failure remains actionable without inventing a code.
 */
async function reportsPersonalInviteFailure(): Promise<void> {
  server.use(
    http.get('http://localhost/api/auth/invite-code', () => HttpResponse.json(null)),
    http.post('http://localhost/api/auth/invite-code', () =>
      HttpResponse.json({ detail: 'Invite quota reached' }, { status: 409 }),
    ),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <InviteCodeCard copyFeedback={null} handleCopy={accountSettingsMocks.copy} />,
  );

  await user.click(await screen.findByRole('button', { name: '生成邀请码' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Invite quota reached');
  expect(screen.queryByRole('button', { name: '复制邀请码' })).not.toBeInTheDocument();
}

/**
 * Verify token creation uses the selected TTL and a failed revocation preserves its owner dialog.
 */
async function createsTokenAndRetainsFailedRevocation(): Promise<void> {
  let creationPayload: unknown;
  server.use(
    http.get('http://localhost/api/auth/tokens', () =>
      HttpResponse.json([
        {
          id: 9,
          name: 'automation',
          expires_at: 2_000_000_000,
          created_at: 1_900_000_000,
        },
      ]),
    ),
    http.post('http://localhost/api/auth/tokens', async ({ request }) => {
      creationPayload = await request.json();
      return HttpResponse.json({
        id: 10,
        token: 'new-raw-token',
        name: 'integration',
        expires_at: 2_000_000_000,
      });
    }),
    http.delete('http://localhost/api/auth/tokens/9', () =>
      HttpResponse.json({ detail: 'Token is still in use' }, { status: 409 }),
    ),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <AccessTokensCard copyFeedback={null} handleCopy={accountSettingsMocks.copy} />,
  );

  expect(await screen.findByText('automation')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '新建' }));
  await user.type(screen.getByLabelText('名称'), 'integration');
  await user.click(screen.getByRole('radio', { name: '30天' }));
  await user.click(screen.getByRole('button', { name: '创建' }));

  expect(await screen.findByText('new-raw-token')).toBeInTheDocument();
  expect(creationPayload).toEqual({ name: 'integration', ttl: 30 * 86400 });
  await user.click(screen.getByRole('button', { name: '复制新访问令牌' }));
  expect(accountSettingsMocks.copy).toHaveBeenCalledWith(
    'new-raw-token',
    '访问令牌已复制。',
    'token',
  );
  await user.click(screen.getByRole('button', { name: '关闭' }));

  await user.click(screen.getByRole('button', { name: '新建' }));
  await waitFor(() => expect(screen.queryByText('new-raw-token')).not.toBeInTheDocument());
  expect(screen.getByLabelText('名称')).toHaveValue('');
  expect(screen.queryByRole('button', { name: '复制新访问令牌' })).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '创建' }));
  expect(await screen.findByText('new-raw-token')).toBeInTheDocument();
  expect(creationPayload).toEqual({ name: '', ttl: 30 * 86400 });
  await user.click(screen.getByRole('button', { name: '关闭' }));

  await user.click(await screen.findByRole('button', { name: '撤销访问令牌 automation' }));
  const confirmation = screen.getByRole('alertdialog', { name: '撤销访问令牌？' });
  await user.click(within(confirmation).getByRole('button', { name: '确认撤销' }));
  expect(await within(confirmation).findByRole('alert')).toHaveTextContent('Token is still in use');
  expect(screen.getByText('automation')).toBeInTheDocument();
}

/**
 * Verify QR start, regenerate, copy, poll, and active-session transitions use committed responses.
 */
async function completesCnkiQrLogin(): Promise<void> {
  let startCount = 0;
  let pollPayload: unknown;
  let cnkiSession = EMPTY_CNKI_SESSION;
  server.use(
    http.get('http://localhost/api/cnki/session', () => HttpResponse.json(cnkiSession)),
    http.post('http://localhost/api/cnki/login/start', () => {
      startCount += 1;
      return HttpResponse.json(createCnkiLoginStart(startCount));
    }),
    http.post('http://localhost/api/cnki/login/poll', async ({ request }) => {
      pollPayload = await request.json();
      const response = createActiveCnkiPoll();
      cnkiSession = response.session;
      return HttpResponse.json(response);
    }),
  );
  const user = userEvent.setup();
  await renderCnkiSettings();

  await user.click(await screen.findByRole('button', { name: '扫码登录' }));
  expect(await screen.findByText('qr-payload-1')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '复制 CNKI 登录二维码内容' }));
  expect(accountSettingsMocks.copy).toHaveBeenCalledWith(
    'qr-payload-1',
    'CNKI 登录二维码内容已复制。',
    'cnkiQr',
  );

  await user.click(screen.getByRole('button', { name: '重新生成' }));
  expect(await screen.findByText('qr-payload-2')).toBeInTheDocument();
  expect(startCount).toBe(2);
  await user.click(screen.getByRole('button', { name: '完成登录' }));

  expect(await screen.findByRole('status')).toHaveTextContent('登录已完成，全文权限已预热。');
  expect(await screen.findByText('已登录')).toBeInTheDocument();
  expect(pollPayload).toEqual({ timeout_seconds: 15, interval_seconds: 1.5 });
}

/**
 * Verify timeout and warmup failures retain the QR challenge with specific recovery guidance.
 */
async function reportsCnkiTimeoutAndWarmupFailures(): Promise<void> {
  let pollAttempt = 0;
  server.use(
    http.get('http://localhost/api/cnki/session', () => HttpResponse.json(EMPTY_CNKI_SESSION)),
    http.post('http://localhost/api/cnki/login/start', () =>
      HttpResponse.json(createCnkiLoginStart(1)),
    ),
    http.post('http://localhost/api/cnki/login/poll', () => {
      pollAttempt += 1;
      if (pollAttempt === 1) {
        return HttpResponse.json(
          {
            title: 'Gateway Timeout',
            status: 504,
            detail: 'scan not confirmed',
            errors: [
              { location: 'code', message: 'cnki_login_timeout' },
              { location: 'phase', message: 'login' },
            ],
          },
          { status: 504, headers: { 'Content-Type': 'application/problem+json' } },
        );
      }
      return HttpResponse.json(
        {
          title: 'Bad Gateway',
          status: 502,
          detail: 'proxy unavailable',
          errors: [
            { location: 'code', message: 'cnki_warmup_failed' },
            { location: 'phase', message: 'warmup' },
          ],
        },
        { status: 502, headers: { 'Content-Type': 'application/problem+json' } },
      );
    }),
  );
  const user = userEvent.setup();
  await renderCnkiSettings();

  await user.click(await screen.findByRole('button', { name: '扫码登录' }));
  await user.click(await screen.findByRole('button', { name: '完成登录' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('未检测到扫码确认');
  expect(screen.getByText('qr-payload-1')).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '完成登录' }));
  expect(await screen.findByRole('alert')).toHaveTextContent(
    '扫码登录已通过，但全文权限预热失败：proxy unavailable',
  );
  expect(pollAttempt).toBe(2);
}

/**
 * Verify a failed clear remains confirmed and keeps the active session visible.
 */
async function retainsCnkiSessionWhenClearFails(): Promise<void> {
  const activeSession = createActiveCnkiPoll().session;
  server.use(
    http.get('http://localhost/api/cnki/session', () => HttpResponse.json(activeSession)),
    http.delete('http://localhost/api/cnki/session', () =>
      HttpResponse.json({ detail: 'CNKI session clear unavailable' }, { status: 503 }),
    ),
  );
  const user = userEvent.setup();
  await renderCnkiSettings();

  await user.click(await screen.findByRole('button', { name: '清除' }));
  const confirmation = screen.getByRole('alertdialog', { name: '清除 CNKI 登录状态？' });
  await user.click(within(confirmation).getByRole('button', { name: '确认清除' }));

  expect(await within(confirmation).findByRole('alert')).toHaveTextContent(
    'CNKI session clear unavailable',
  );
  expect(screen.getByText('已登录')).toBeInTheDocument();
}

beforeEach(() => {
  accountSettingsMocks.copy.mockReset().mockResolvedValue(undefined);
  accountSettingsMocks.logout.mockReset().mockResolvedValue(undefined);
  accountSettingsMocks.updateUser.mockReset();
  accountSettingsMocks.user = null;
});

afterEach(() => {
  server.events.removeAllListeners();
  vi.unstubAllGlobals();
});

describe('account settings', () => {
  test('manages a local account email', managesLocalAccountEmail);
  test('starts single sign-on linking', startsSingleSignOnLink);
  test('disables linking when single sign-on is off', disablesLinkWhenSingleSignOnIsOff);
  test('unlinks single sign-on after confirmation', unlinksSingleSignOnAfterConfirmation);
  test('shows a read-only single sign-on account', showsReadOnlyOidcAccount);
  test('announces the link callback result', announcesLinkCallbackResult);
  test('changes password then logs out', changesPasswordThenLogsOut, 5_000);
  test('reports password change failure', reportsPasswordChangeFailure);
  test('generates and copies a personal invite', generatesAndCopiesPersonalInvite);
  test('reports personal invite failure', reportsPersonalInviteFailure);
  test('creates a token and retains failed revocation', createsTokenAndRetainsFailedRevocation);
  test('completes CNKI QR login', completesCnkiQrLogin);
  test('reports CNKI timeout and warmup failures', reportsCnkiTimeoutAndWarmupFailures);
  test('retains CNKI session when clear fails', retainsCnkiSessionWhenClearFails);
});
