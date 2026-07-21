/**
 * Account password, invite, access-token, and CNKI settings behavior coverage.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

const accountSettingsMocks = vi.hoisted(() => ({
  copy: vi.fn().mockResolvedValue(undefined),
  logout: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({ logout: accountSettingsMocks.logout }),
}));

import { AccessTokensCard } from '@/components/settings/access-tokens-card';
import { CnkiSettingsCard } from '@/components/settings/cnki-card';
import { InviteCodeCard } from '@/components/settings/invite-code-card';
import { PasswordCard } from '@/components/settings/password-card';
import type {
  CnkiLoginPollResponse,
  CnkiLoginStartResponse,
  CnkiSessionStatus,
  InviteCode,
} from '@/lib/api';
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
function renderCnkiSettings(): void {
  renderWithQuery(
    <CnkiSettingsCard userId={21} copyFeedback={null} handleCopy={accountSettingsMocks.copy} />,
  );
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
  renderWithQuery(<PasswordCard />);

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
  renderWithQuery(<PasswordCard />);

  await user.type(screen.getByLabelText('原密码'), 'wrong-password');
  await user.type(screen.getByLabelText('新密码'), 'new-password-123');
  await user.click(screen.getByRole('button', { name: '修改密码' }));

  expect(await screen.findByRole('alert')).toHaveTextContent('Current password is incorrect');
  expect(accountSettingsMocks.logout).not.toHaveBeenCalled();
}

/**
 * Verify personal invite generation refetches the committed value and exposes scoped copy.
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
        created_at: 1_900_000_000,
      };
      return HttpResponse.json(inviteCode);
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<InviteCodeCard copyFeedback={null} handleCopy={accountSettingsMocks.copy} />);

  await user.click(await screen.findByRole('button', { name: '生成邀请码' }));
  expect(await screen.findByText('personal-invite')).toBeInTheDocument();
  expect(screen.getByText('此邀请码尚未使用')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '复制邀请码' }));
  expect(accountSettingsMocks.copy).toHaveBeenCalledWith(
    'personal-invite',
    '邀请码已复制。',
    'invite',
  );
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
  renderWithQuery(<InviteCodeCard copyFeedback={null} handleCopy={accountSettingsMocks.copy} />);

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
  renderWithQuery(<AccessTokensCard copyFeedback={null} handleCopy={accountSettingsMocks.copy} />);

  expect(await screen.findByText('automation')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '新建' }));
  await user.type(screen.getByLabelText('名称'), 'integration');
  await user.click(screen.getByRole('button', { name: '30天' }));
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
  renderCnkiSettings();

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
            detail: {
              code: 'cnki_login_timeout',
              phase: 'login',
              message: 'scan not confirmed',
            },
          },
          { status: 504 },
        );
      }
      return HttpResponse.json(
        {
          detail: {
            code: 'cnki_warmup_failed',
            phase: 'warmup',
            message: 'proxy unavailable',
          },
        },
        { status: 502 },
      );
    }),
  );
  const user = userEvent.setup();
  renderCnkiSettings();

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
  renderCnkiSettings();

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
});

describe('account settings', () => {
  test('changes password then logs out', changesPasswordThenLogsOut, 5_000);
  test('reports password change failure', reportsPasswordChangeFailure);
  test('generates and copies a personal invite', generatesAndCopiesPersonalInvite);
  test('reports personal invite failure', reportsPersonalInviteFailure);
  test('creates a token and retains failed revocation', createsTokenAndRetainsFailedRevocation);
  test('completes CNKI QR login', completesCnkiQrLogin);
  test('reports CNKI timeout and warmup failures', reportsCnkiTimeoutAndWarmupFailures);
  test('retains CNKI session when clear fails', retainsCnkiSessionWhenClearFails);
});
