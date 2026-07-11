/**
 * Focused rendering coverage for extracted settings and administrator cards.
 */

import { act, renderHook, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test, vi } from 'vitest';

import { AdminInviteCodesCard } from '@/components/admin/invite-codes-card';
import { AdminUsersCard } from '@/components/admin/users-card';
import { AccessTokensCard } from '@/components/settings/access-tokens-card';
import { AccountCard } from '@/components/settings/account-card';
import { CnkiSettingsCard } from '@/components/settings/cnki-card';
import { InviteCodeCard } from '@/components/settings/invite-code-card';
import { PasswordCard } from '@/components/settings/password-card';
import { useSettingsCopy } from '@/components/settings/use-settings-copy';
import { AuthProvider } from '@/lib/auth-context';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

/**
 * Ignore copy actions in static card rendering tests.
 */
function ignoreCopy(): Promise<void> {
  return Promise.resolve();
}

/**
 * Verify extracted administrator cards own their original list queries.
 */
async function rendersAdministratorCards(): Promise<void> {
  server.use(
    http.get('http://localhost/api/admin/users', () => HttpResponse.json([])),
    http.get('http://localhost/api/admin/invite-codes', () => HttpResponse.json([])),
  );

  renderWithQuery(
    <>
      <AdminUsersCard currentUserId={1} isEnabled />
      <AdminInviteCodesCard isEnabled />
    </>,
  );

  expect(await screen.findByText('账号管理')).toBeInTheDocument();
  expect(await screen.findAllByText('暂无邀请码')).toHaveLength(2);
}

/**
 * Verify extracted settings cards retain their independent query boundaries.
 */
async function rendersSettingsCards(): Promise<void> {
  server.use(
    http.get('http://localhost/api/cnki/session', () =>
      HttpResponse.json({
        configured: false,
        status: 'empty',
        has_bff_user_token: false,
        expires_at: null,
        seconds_remaining: null,
        cookie_names: [],
        updated_at: null,
        last_used_at: null,
      }),
    ),
    http.get('http://localhost/api/auth/invite-code', () => HttpResponse.json(null)),
    http.get('http://localhost/api/auth/tokens', () => HttpResponse.json([])),
  );

  renderWithQuery(
    <>
      <AccountCard username="fixture_user" />
      <CnkiSettingsCard userId={1} copyFeedback={null} handleCopy={ignoreCopy} />
      <InviteCodeCard copyFeedback={null} handleCopy={ignoreCopy} />
      <AccessTokensCard copyFeedback={null} handleCopy={ignoreCopy} />
    </>,
  );

  expect(screen.getByText('fixture_user')).toBeInTheDocument();
  expect(await screen.findByText('未配置')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '生成邀请码' })).toBeInTheDocument();
  expect(await screen.findByText('暂无访问令牌')).toBeInTheDocument();
}

/**
 * Render the access-token card with an empty initial list.
 *
 * @param createHandler - MSW handler for access-token creation.
 */
function renderAccessTokens(createHandler: Parameters<typeof http.post>[1]): void {
  server.use(
    http.get('http://localhost/api/auth/tokens', () => HttpResponse.json([])),
    http.post('http://localhost/api/auth/tokens', createHandler),
  );
  renderWithQuery(<AccessTokensCard copyFeedback={null} handleCopy={ignoreCopy} />);
}

/**
 * Verify the token form counts astral characters by code point and sends the raw name.
 */
async function createsAccessTokenAtRawCodePointBoundary(): Promise<void> {
  const createdNames: string[] = [];
  renderAccessTokens(async ({ request }) => {
    const payload = (await request.json()) as { name: string; ttl: number };
    createdNames.push(payload.name);
    return HttpResponse.json({
      id: 7,
      token: 'raw-token-value',
      name: payload.name.trim(),
      expires_at: 2_000_000_000,
    });
  });
  const user = userEvent.setup();

  await user.click(screen.getByRole('button', { name: '新建' }));
  const input = screen.getByLabelText('名称');
  const astralName = '😀'.repeat(100);
  expect(input).not.toHaveAttribute('maxlength');
  await user.type(input, astralName);

  expect(input).toHaveValue(astralName);
  expect((input as HTMLInputElement).value.length).toBe(200);
  expect(screen.getByText('100/100 Unicode code points')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '创建' }));

  await waitFor(() => expect(createdNames).toEqual([astralName]));
  expect(await screen.findByText('raw-token-value')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '复制新访问令牌' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  await user.click(screen.getByRole('button', { name: '新建' }));
  expect(screen.getByLabelText('名称')).toHaveValue('');
}

/**
 * Verify raw surrounding spaces and 101 astral code points are blocked without a request.
 */
async function blocksOverlengthRawAccessTokenNames(): Promise<void> {
  let requestCount = 0;
  renderAccessTokens(() => {
    requestCount += 1;
    return HttpResponse.json({
      id: 8,
      token: 'unexpected-token',
      name: 'unexpected',
      expires_at: 2_000_000_000,
    });
  });
  const user = userEvent.setup();

  await user.click(screen.getByRole('button', { name: '新建' }));
  const input = screen.getByLabelText('名称');
  const lengthDetail = 'Access token name must be at most 100 Unicode code points';
  const surroundingSpaces = ` ${'a'.repeat(99)} `;
  await user.type(input, surroundingSpaces);

  expect(screen.getByRole('alert')).toHaveTextContent(lengthDetail);
  expect(screen.getByText('101/100 Unicode code points')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '创建' })).toBeDisabled();
  expect(requestCount).toBe(0);

  await user.clear(input);
  await user.type(input, '😀'.repeat(101));

  expect(screen.getByRole('alert')).toHaveTextContent(lengthDetail);
  expect(screen.getByText('101/100 Unicode code points')).toBeInTheDocument();
  expect(requestCount).toBe(0);
}

/**
 * Verify every backend issuance error stays visible with the untrimmed form value.
 */
async function rendersAccessTokenCreationErrorsAndRetainsRawInput(): Promise<void> {
  const details = new Map([
    ['server-length', 'Access token name must be at most 100 Unicode code points'],
    ['  login  ', 'Access token name "login" is reserved'],
    ['ttl-error', 'Access token TTL must be between 3600 and 31536000 seconds'],
    [
      'quota-error',
      'Active access token limit of 50 reached; revoke a token before creating another',
    ],
  ]);
  const receivedNames: string[] = [];
  renderAccessTokens(async ({ request }) => {
    const payload = (await request.json()) as { name: string; ttl: number };
    receivedNames.push(payload.name);
    const detail = details.get(payload.name);
    if (!detail) {
      return HttpResponse.json({ detail: 'Unexpected test payload' }, { status: 400 });
    }
    return HttpResponse.json({ detail }, { status: payload.name === 'quota-error' ? 409 : 400 });
  });
  const user = userEvent.setup();

  await user.click(screen.getByRole('button', { name: '新建' }));
  const input = screen.getByLabelText('名称');
  for (const [name, detail] of details) {
    await user.clear(input);
    await user.type(input, name);
    await user.click(screen.getByRole('button', { name: '创建' }));

    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(detail));
    expect(input).toHaveValue(name);
    expect(screen.getByRole('dialog', { name: '创建访问令牌' })).toBeInTheDocument();
  }

  expect(receivedNames).toEqual(Array.from(details.keys()));
}

/**
 * Verify the password card remains bound to the shared authentication context.
 */
async function rendersPasswordCard(): Promise<void> {
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ id: 1, username: 'fixture_user', is_admin: false }),
    ),
  );

  renderWithQuery(
    <AuthProvider>
      <PasswordCard />
    </AuthProvider>,
  );

  expect(await screen.findByLabelText('原密码')).toBeInTheDocument();
  expect(screen.getByLabelText('新密码')).toHaveAttribute('minlength', '12');
}

/**
 * Verify one shared copy hook reports the owning card scope.
 */
async function reportsSharedCopyScope(): Promise<void> {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  });
  const { result } = renderHook(() => useSettingsCopy());

  await act(async () => {
    await result.current.handleCopy('invite-code', '邀请码已复制。', 'invite');
  });

  expect(writeText).toHaveBeenCalledWith('invite-code');
  expect(result.current.copyFeedback).toEqual({
    message: '邀请码已复制。',
    scope: 'invite',
    tone: 'success',
  });
}

describe('settings and administrator feature boundaries', () => {
  test('renders administrator user and invite cards', rendersAdministratorCards);
  test('renders account, CNKI, invite, and token cards', rendersSettingsCards);
  test(
    'creates a token at the raw Unicode code-point boundary',
    createsAccessTokenAtRawCodePointBoundary,
  );
  test('blocks overlength raw access-token names', blocksOverlengthRawAccessTokenNames);
  test(
    'renders access-token creation errors and retains raw input',
    rendersAccessTokenCreationErrorsAndRetainsRawInput,
  );
  test('keeps password changes inside the auth context', rendersPasswordCard);
  test('shares scoped copy feedback across settings cards', reportsSharedCopyScope);
});
