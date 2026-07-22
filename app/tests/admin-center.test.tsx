/**
 * Administrator-center URL, permission, navigation, and lifecycle coverage.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { AdminCenterDialog } from '@/components/admin/admin-center-dialog';
import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { ADMIN_SECTION_IDS, buildAdminCenterHref, parseAdminSection } from '@/lib/admin-center';
import { renderWithQuery } from '@/tests/render';

const adminCenterMocks = vi.hoisted(() => ({
  auth: {
    user: { id: 71, username: 'admin_user', is_admin: true } as {
      id: number;
      is_admin: boolean;
      username: string;
    } | null,
  },
  pathname: '/',
  router: {
    replace: vi.fn(),
  },
  searchParams: new URLSearchParams('q=graph&area=Accounting+%26+Auditing&admin=overview'),
}));

vi.mock('next/navigation', () => ({
  usePathname: () => adminCenterMocks.pathname,
  useRouter: () => adminCenterMocks.router,
  useSearchParams: () => adminCenterMocks.searchParams,
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => adminCenterMocks.auth,
}));

vi.mock('@/components/admin/overview-card', () => ({
  AdminOverviewCard: () => <div>overview card marker</div>,
}));

vi.mock('@/components/admin/users-card', () => ({
  AdminUsersCard: () => (
    <label>
      user card marker
      <input aria-label="用户草稿" />
    </label>
  ),
}));

vi.mock('@/components/admin/invite-codes-card', () => ({
  AdminInviteCodesCard: () => <div>invite codes card marker</div>,
}));

vi.mock('@/components/admin/runtime-settings-card', () => ({
  RuntimeSettingsCard: () => <div>runtime settings card marker</div>,
}));

vi.mock('@/components/admin/scheduled-tasks-card', () => ({
  ScheduledTasksCard: () => <div>scheduled tasks card marker</div>,
}));

vi.mock('@/components/admin/announcements-card', () => ({
  AnnouncementsCard: () => <div>announcements card marker</div>,
}));

/** Verify stable administrator sections and mutually exclusive URL construction. */
function preservesAdministratorQueryState(): void {
  for (const section of ADMIN_SECTION_IDS) {
    expect(parseAdminSection(section)).toBe(section);
  }
  expect(parseAdminSection('unknown')).toBeNull();
  expect(parseAdminSection(null)).toBeNull();
  expect(
    buildAdminCenterHref('/', new URLSearchParams('q=graph&settings=general&folder=4'), 'users'),
  ).toBe('/?q=graph&folder=4&admin=users');
  expect(buildAdminCenterHref('/', new URLSearchParams('q=graph&admin=users&folder=4'), null)).toBe(
    '/?q=graph&folder=4',
  );
}

/** Verify category changes preserve mounted panels, workspace state, and close focus. */
async function navigatesPersistentAdministratorPanels(): Promise<void> {
  const user = userEvent.setup();
  const menuTrigger = document.createElement('button');
  menuTrigger.textContent = '账号菜单';
  menuTrigger.setAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE, '');
  document.body.append(menuTrigger);

  const view = renderWithQuery(<AdminCenterDialog />);

  expect(await screen.findByRole('dialog', { name: '管理面板' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '概览' })).toBeInTheDocument();
  expect(screen.getAllByRole('navigation', { name: '管理分类' })).toHaveLength(2);
  expect(screen.getAllByRole('tabpanel', { hidden: true })).toHaveLength(6);
  expect(screen.getByRole('tabpanel', { name: '概览面板' })).not.toHaveAttribute('hidden');
  expect(menuTrigger).not.toHaveAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE);

  adminCenterMocks.searchParams = new URLSearchParams(
    'q=graph&area=Accounting+%26+Auditing&admin=users',
  );
  view.rerender(<AdminCenterDialog />);

  await waitFor(() => expect(screen.getByRole('heading', { name: '用户' })).toBeInTheDocument());
  const userDraft = screen.getByLabelText('用户草稿');
  await user.type(userDraft, 'preserved');
  adminCenterMocks.searchParams = new URLSearchParams(
    'q=graph&area=Accounting+%26+Auditing&admin=overview',
  );
  view.rerender(<AdminCenterDialog />);
  await waitFor(() => expect(screen.getByRole('heading', { name: '概览' })).toBeInTheDocument());
  adminCenterMocks.searchParams = new URLSearchParams(
    'q=graph&area=Accounting+%26+Auditing&admin=users',
  );
  view.rerender(<AdminCenterDialog />);
  await waitFor(() => expect(screen.getByRole('heading', { name: '用户' })).toBeInTheDocument());

  expect(screen.getByRole('tabpanel', { name: '用户面板' })).not.toHaveAttribute('hidden');
  expect(
    screen
      .getByRole('dialog', { name: '管理面板' })
      .querySelector('[role="tabpanel"][aria-label="概览面板"]'),
  ).toHaveAttribute('hidden');
  expect(screen.getByLabelText('用户草稿')).toBe(userDraft);
  expect(userDraft).toHaveValue('preserved');
  expect(
    screen
      .getAllByRole('button', { name: '用户' })
      .every((button) => button.hasAttribute('aria-current')),
  ).toBe(true);

  const historyLength = window.history.length;
  await user.click(screen.getAllByRole('button', { name: '运行配置' })[0]);
  expect(`${window.location.pathname}${window.location.search}`).toBe(
    '/?q=graph&area=Accounting+%26+Auditing&admin=runtime-settings',
  );
  expect(window.history.length).toBe(historyLength);
  expect(adminCenterMocks.router.replace).not.toHaveBeenCalled();

  await user.click(screen.getByRole('button', { name: '关闭' }));
  expect(adminCenterMocks.router.replace).toHaveBeenLastCalledWith(
    '/?q=graph&area=Accounting+%26+Auditing',
    { scroll: false },
  );
  await waitFor(() => expect(menuTrigger).toHaveFocus());
  menuTrigger.remove();
}

/** Verify invalid administrator query state is removed without mounting card data owners. */
async function normalizesUnknownAdministratorSection(): Promise<void> {
  adminCenterMocks.searchParams = new URLSearchParams('q=graph&admin=unknown');
  renderWithQuery(<AdminCenterDialog />);

  expect(screen.queryByRole('dialog', { name: '管理面板' })).not.toBeInTheDocument();
  expect(screen.queryByText('overview card marker')).not.toBeInTheDocument();
  await waitFor(() =>
    expect(adminCenterMocks.router.replace).toHaveBeenCalledWith('/?q=graph', { scroll: false }),
  );
}

/** Verify a non-administrator cannot mount management cards through a hand-written query. */
async function rejectsUnauthorizedAdministratorQuery(): Promise<void> {
  adminCenterMocks.auth.user = { id: 72, username: 'reader', is_admin: false };
  adminCenterMocks.searchParams = new URLSearchParams('q=graph&admin=users');
  renderWithQuery(<AdminCenterDialog />);

  expect(screen.queryByRole('dialog', { name: '管理面板' })).not.toBeInTheDocument();
  expect(screen.queryByText('user card marker')).not.toBeInTheDocument();
  await waitFor(() =>
    expect(adminCenterMocks.router.replace).toHaveBeenCalledWith('/?q=graph', { scroll: false }),
  );
}

/** Verify valid settings state wins and removes a conflicting administrator query. */
async function givesSettingsConflictPriority(): Promise<void> {
  adminCenterMocks.searchParams = new URLSearchParams('q=graph&settings=general&admin=users');
  renderWithQuery(<AdminCenterDialog />);

  expect(screen.queryByRole('dialog', { name: '管理面板' })).not.toBeInTheDocument();
  await waitFor(() =>
    expect(adminCenterMocks.router.replace).toHaveBeenCalledWith('/?q=graph&settings=general', {
      scroll: false,
    }),
  );
}

beforeEach(() => {
  adminCenterMocks.auth.user = { id: 71, username: 'admin_user', is_admin: true };
  adminCenterMocks.pathname = '/';
  adminCenterMocks.searchParams = new URLSearchParams(
    'q=graph&area=Accounting+%26+Auditing&admin=overview',
  );
  window.history.replaceState(null, '', '/?q=graph&area=Accounting+%26+Auditing&admin=overview');
  adminCenterMocks.router.replace.mockReset();
});

describe('AdminCenterDialog', () => {
  test(
    'preserves unrelated query state for every administrator section',
    preservesAdministratorQueryState,
  );
  test(
    'navigates persistent administrator panels and restores focus',
    navigatesPersistentAdministratorPanels,
  );
  test('normalizes an unknown administrator section', normalizesUnknownAdministratorSection);
  test('rejects an unauthorized administrator query', rejectsUnauthorizedAdministratorQuery);
  test('gives valid settings state conflict priority', givesSettingsConflictPriority);
});
