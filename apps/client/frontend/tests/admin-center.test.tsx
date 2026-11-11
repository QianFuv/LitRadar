/**
 * Administrator-center URL, permission, navigation, and lifecycle coverage.
 */

import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import { AdminCenterDialog } from '@/components/admin/admin-center-dialog';
import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { ADMIN_SECTION_IDS, parseAdminSection, withAdminSection } from '@/lib/admin-center';
import { stringifySearchObject } from '@/lib/search-params';
import { readRouterUrl, renderWithQuery, type TestRouter } from '@/tests/render';

const adminCenterMocks = vi.hoisted(() => ({
  auth: {
    user: { id: 71, username: 'admin_user', is_admin: true } as {
      id: number;
      is_admin: boolean;
      username: string;
    } | null,
  },
}));

const WORKSPACE_URL = '/?q=graph&area=Accounting+%26+Auditing';

/**
 * Find the panel a section tab controls, including hidden retained panels, which have no
 * accessible name while hidden.
 *
 * @param tabName - Section tab name.
 * @returns Controlled panel element, or null when it is not mounted.
 */
function findPanelOfTab(tabName: string): HTMLElement | null {
  const tab = screen.getAllByRole('tab', { name: tabName })[0];
  return document.getElementById(tab.getAttribute('aria-controls') ?? '');
}
/**
 * Move the in-memory location as an external navigation would.
 *
 * @param router - Router of the rendered dialog.
 * @param href - Next location.
 */
async function visit(router: TestRouter, href: string): Promise<void> {
  await act(() => router.navigate({ href, replace: true }));
}

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => adminCenterMocks.auth,
}));

vi.mock('@/components/admin/overview-card', () => ({
  AdminOverviewCard: () => <div>overview card marker</div>,
}));

vi.mock('@/components/admin/load-telemetry-card', () => ({
  LoadTelemetryCard: () => <div>load telemetry card marker</div>,
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
  ScheduledTasksCard: ({ isActive }: { isActive?: boolean }) => (
    <div data-testid="scheduler-panel" data-active={String(isActive)}>
      scheduled tasks card marker
    </div>
  ),
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
  expect(parseAdminSection(undefined)).toBeNull();
  expect(
    stringifySearchObject(
      withAdminSection({ q: 'graph', settings: 'general', folder: 4 }, 'users'),
    ),
  ).toBe('?q=graph&folder=4&admin=users');
  expect(
    stringifySearchObject(withAdminSection({ q: 'graph', admin: 'users', folder: 4 }, null)),
  ).toBe('?q=graph&folder=4');
}

/** Verify category changes preserve mounted panels, workspace state, and close focus. */
async function navigatesPersistentAdministratorPanels(): Promise<void> {
  const user = userEvent.setup();
  const menuTrigger = document.createElement('button');
  menuTrigger.textContent = '账号菜单';
  menuTrigger.setAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE, '');
  document.body.append(menuTrigger);

  const view = await renderWithQuery(<AdminCenterDialog />, {
    url: `${WORKSPACE_URL}&admin=overview`,
  });

  expect(await screen.findByRole('dialog', { name: '管理面板' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '概览' })).toBeInTheDocument();
  expect(screen.getAllByRole('tablist', { name: '管理分类' })).toHaveLength(1);
  expect(screen.getAllByRole('tabpanel', { hidden: true })).toHaveLength(7);
  expect(screen.getByText('overview card marker')).toBeInTheDocument();
  for (const marker of [
    'load telemetry card marker',
    'user card marker',
    'invite codes card marker',
    'runtime settings card marker',
    'scheduled tasks card marker',
    'announcements card marker',
  ]) {
    expect(screen.queryByText(marker)).not.toBeInTheDocument();
  }
  const overviewPanel = screen.getByRole('tabpanel', { name: '概览' });
  const retainedUserPanel = findPanelOfTab('用户');
  expect(overviewPanel).not.toHaveAttribute('hidden');
  expect(overviewPanel).toHaveAttribute('data-admin-panel-state', 'active');
  expect(retainedUserPanel).not.toBeNull();
  expect(retainedUserPanel).toHaveAttribute('data-admin-panel-state', 'inactive');
  expect(document.querySelector('[data-mobile-overflow-cue="true"]')).toBeInTheDocument();
  expect(document.querySelector('[data-motion-section-header="overview"]')).toBeInTheDocument();
  expect(menuTrigger).not.toHaveAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE);

  await visit(view.router, `${WORKSPACE_URL}&admin=users`);

  await waitFor(() => expect(screen.getByRole('heading', { name: '用户' })).toBeInTheDocument());
  const userDraft = screen.getByLabelText('用户草稿');
  await user.type(userDraft, 'preserved');
  await visit(view.router, `${WORKSPACE_URL}&admin=overview`);
  await waitFor(() => expect(screen.getByRole('heading', { name: '概览' })).toBeInTheDocument());
  await visit(view.router, `${WORKSPACE_URL}&admin=users`);
  await waitFor(() => expect(screen.getByRole('heading', { name: '用户' })).toBeInTheDocument());

  expect(screen.getByRole('tabpanel', { name: '用户' })).not.toHaveAttribute('hidden');
  expect(screen.getByRole('tabpanel', { name: '用户' })).toBe(retainedUserPanel);
  expect(retainedUserPanel).toHaveAttribute('data-admin-panel-state', 'active');
  expect(overviewPanel).toHaveAttribute('data-admin-panel-state', 'inactive');
  expect(findPanelOfTab('概览')).toHaveAttribute('hidden');
  expect(screen.getByLabelText('用户草稿')).toBe(userDraft);
  expect(userDraft).toHaveValue('preserved');
  expect(screen.queryByText('scheduled tasks card marker')).not.toBeInTheDocument();
  expect(
    screen
      .getAllByRole('tab', { name: '用户' })
      .every((tab) => tab.getAttribute('aria-selected') === 'true'),
  ).toBe(true);

  await user.click(screen.getAllByRole('tab', { name: '运行配置' })[0]);
  await waitFor(() =>
    expect(readRouterUrl(view.router)).toBe(`${WORKSPACE_URL}&admin=runtime-settings`),
  );

  await user.click(screen.getByRole('button', { name: '返回' }));
  await waitFor(() => expect(readRouterUrl(view.router)).toBe(WORKSPACE_URL));
  await waitFor(() => expect(menuTrigger).toHaveFocus());
  menuTrigger.remove();
}

/** Verify invalid administrator query state is removed without mounting card data owners. */
async function normalizesUnknownAdministratorSection(): Promise<void> {
  const view = await renderWithQuery(<AdminCenterDialog />, { url: '/?q=graph&admin=unknown' });

  expect(screen.queryByRole('dialog', { name: '管理面板' })).not.toBeInTheDocument();
  expect(screen.queryByText('overview card marker')).not.toBeInTheDocument();
  await waitFor(() => expect(readRouterUrl(view.router)).toBe('/?q=graph'));
}

/** Verify a non-administrator cannot mount management cards through a hand-written query. */
async function rejectsUnauthorizedAdministratorQuery(): Promise<void> {
  adminCenterMocks.auth.user = { id: 72, username: 'reader', is_admin: false };
  const view = await renderWithQuery(<AdminCenterDialog />, { url: '/?q=graph&admin=users' });

  expect(screen.queryByRole('dialog', { name: '管理面板' })).not.toBeInTheDocument();
  expect(screen.queryByText('user card marker')).not.toBeInTheDocument();
  await waitFor(() => expect(readRouterUrl(view.router)).toBe('/?q=graph'));
}

/** Verify valid settings state wins and removes a conflicting administrator query. */
async function givesSettingsConflictPriority(): Promise<void> {
  const view = await renderWithQuery(<AdminCenterDialog />, {
    url: '/?q=graph&settings=general&admin=users',
  });

  expect(screen.queryByRole('dialog', { name: '管理面板' })).not.toBeInTheDocument();
  await waitFor(() => expect(readRouterUrl(view.router)).toBe('/?q=graph&settings=general'));
}

/** Verify a direct scheduler URL mounts its data owner without starting unrelated panels. */
async function defersUnvisitedPanelsForDirectLinks(): Promise<void> {
  const view = await renderWithQuery(<AdminCenterDialog />, { url: '/?admin=scheduled-tasks' });
  expect(await screen.findByText('scheduled tasks card marker')).toBeInTheDocument();
  expect(screen.getAllByRole('tabpanel', { hidden: true })).toHaveLength(7);
  expect(screen.queryByText('overview card marker')).not.toBeInTheDocument();
  expect(screen.queryByText('user card marker')).not.toBeInTheDocument();
  const scheduler = screen.getByTestId('scheduler-panel');
  expect(scheduler).toHaveAttribute('data-active', 'true');
  await visit(view.router, '/?admin=overview');
  await waitFor(() => expect(scheduler).toHaveAttribute('data-active', 'false'));
  expect(screen.getByTestId('scheduler-panel')).toBe(scheduler);
}

beforeEach(() => {
  adminCenterMocks.auth.user = { id: 71, username: 'admin_user', is_admin: true };
});

describe('AdminCenterDialog', () => {
  test('defers unvisited panels for a direct section link', defersUnvisitedPanelsForDirectLinks);
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
