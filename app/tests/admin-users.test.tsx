/**
 * Administrator user role, self-guard, password-reset, deletion, and failure coverage.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test } from 'vitest';

import { AdminUsersCard } from '@/components/admin/users-card';
import type { AdminUserInfo } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let users: AdminUserInfo[] = [];
let userListRequestCount = 0;

/**
 * Build one administrator user row.
 *
 * @param id - User identifier.
 * @param username - Display username.
 * @param isAdmin - Whether the user is an administrator.
 * @returns Administrator user response row.
 */
function createAdminUser(id: number, username: string, isAdmin: boolean): AdminUserInfo {
  return {
    id,
    username,
    is_admin: isAdmin,
    created_at: 1_900_000_000 + id,
    updated_at: 1_900_000_100 + id,
    folder_count: id,
    favorite_count: id * 2,
    notify_enabled: id % 2 === 0,
  };
}

/**
 * Return the mutable user list and count authoritative refetches.
 *
 * @returns Current administrator user list.
 */
function usersResponse(): Response {
  userListRequestCount += 1;
  return HttpResponse.json(users);
}

/**
 * Install the common administrator user list handler.
 */
function installUserListHandler(): void {
  server.use(http.get('http://localhost/api/admin/users', usersResponse));
}

/**
 * Verify self-protection plus successful promotion/demotion and failed-role rollback.
 */
async function changesRolesAndProtectsSelf(): Promise<void> {
  const rolePayloads: unknown[] = [];
  server.use(
    http.get('http://localhost/api/admin/users', usersResponse),
    http.put('http://localhost/api/admin/users/2/admin', async ({ request }) => {
      const payload = (await request.json()) as { is_admin: boolean };
      rolePayloads.push(payload);
      if (rolePayloads.length === 3) {
        return HttpResponse.json({ detail: 'Role update denied' }, { status: 409 });
      }
      users = users.map((user) => (user.id === 2 ? { ...user, is_admin: payload.is_admin } : user));
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<AdminUsersCard currentUserId={1} isEnabled />);

  expect(await screen.findByRole('button', { name: '取消 admin 的管理员' })).toBeDisabled();
  expect(screen.getByRole('button', { name: '删除用户 admin' })).toBeDisabled();

  await user.click(screen.getByRole('button', { name: '设为 member 为管理员' }));
  expect(await screen.findByRole('button', { name: '取消 member 的管理员' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '取消 member 的管理员' }));
  expect(await screen.findByRole('button', { name: '设为 member 为管理员' })).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '设为 member 为管理员' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Role update denied');
  expect(screen.getByRole('button', { name: '设为 member 为管理员' })).toBeInTheDocument();
  expect(rolePayloads).toEqual([{ is_admin: true }, { is_admin: false }, { is_admin: true }]);
  expect(userListRequestCount).toBeGreaterThanOrEqual(3);
}

/**
 * Verify password-reset failure retains the form and can retry successfully.
 */
async function retriesPasswordReset(): Promise<void> {
  const resetPayloads: unknown[] = [];
  installUserListHandler();
  server.use(
    http.post('http://localhost/api/admin/users/2/reset-password', async ({ request }) => {
      resetPayloads.push(await request.json());
      return resetPayloads.length === 1
        ? HttpResponse.json({ detail: 'Password policy rejected' }, { status: 400 })
        : HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<AdminUsersCard currentUserId={1} isEnabled />);

  await user.click(await screen.findByRole('button', { name: '重置 member 的密码' }));
  const dialog = screen.getByRole('dialog', { name: '重置密码' });
  const passwordInput = within(dialog).getByLabelText('新密码');
  await user.type(passwordInput, 'replacement-password');
  await user.click(within(dialog).getByRole('button', { name: '确认重置' }));

  expect(await within(dialog).findByRole('alert')).toHaveTextContent('Password policy rejected');
  expect(passwordInput).toHaveValue('replacement-password');
  await user.click(within(dialog).getByRole('button', { name: '确认重置' }));
  await waitFor(() =>
    expect(screen.queryByRole('dialog', { name: '重置密码' })).not.toBeInTheDocument(),
  );
  expect(resetPayloads).toEqual([
    { new_password: 'replacement-password' },
    { new_password: 'replacement-password' },
  ]);
}

/**
 * Verify failed deletion remains targeted and a retry removes only that user.
 */
async function retriesUserDeletion(): Promise<void> {
  let deleteRequestCount = 0;
  installUserListHandler();
  server.use(
    http.delete('http://localhost/api/admin/users/2', () => {
      deleteRequestCount += 1;
      if (deleteRequestCount === 1) {
        return HttpResponse.json({ detail: 'User deletion unavailable' }, { status: 503 });
      }
      users = users.filter((user) => user.id !== 2);
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<AdminUsersCard currentUserId={1} isEnabled />);

  await user.click(await screen.findByRole('button', { name: '删除用户 member' }));
  const dialog = screen.getByRole('dialog', { name: '确认删除用户' });
  expect(dialog).toHaveTextContent('用户 #2');
  await user.click(within(dialog).getByRole('button', { name: '确认删除' }));

  expect(await within(dialog).findByRole('alert')).toHaveTextContent('User deletion unavailable');
  expect(dialog).toHaveTextContent('用户 #2');
  await user.click(within(dialog).getByRole('button', { name: '确认删除' }));

  await waitFor(() =>
    expect(screen.queryByRole('button', { name: '删除用户 member' })).not.toBeInTheDocument(),
  );
  expect(screen.getByRole('button', { name: '删除用户 admin' })).toBeDisabled();
  expect(deleteRequestCount).toBe(2);
  expect(userListRequestCount).toBeGreaterThanOrEqual(2);
}

beforeEach(() => {
  users = [createAdminUser(1, 'admin', true), createAdminUser(2, 'member', false)];
  userListRequestCount = 0;
});

describe('administrator users', () => {
  test('changes roles and protects the current administrator', changesRolesAndProtectsSelf);
  test('retries a failed password reset', retriesPasswordReset);
  test('retries a failed user deletion', retriesUserDeletion);
});
