/**
 * Focused rendering coverage for extracted settings and administrator cards.
 */

import { act, renderHook, screen } from '@testing-library/react';
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
  test('keeps password changes inside the auth context', rendersPasswordCard);
  test('shares scoped copy feedback across settings cards', reportsSharedCopyScope);
});
