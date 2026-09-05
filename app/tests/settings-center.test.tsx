/**
 * Aggregated settings URL, dialog navigation, and unsaved-transition coverage.
 */

import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { Activity } from 'react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { SettingsCenterDialog } from '@/components/settings/settings-center-dialog';
import {
  buildSettingsCenterHref,
  parseSettingsSection,
  SETTINGS_SECTION_IDS,
} from '@/lib/settings-center';
import { idleManualPushStatusResponse } from '@/tests/mocks/handlers/tracking';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const navigationMocks = vi.hoisted(() => ({
  pathname: '/',
  router: {
    push: vi.fn(),
    replace: vi.fn(),
  },
  searchParams: new URLSearchParams('view=favorites&folder=4&settings=general'),
}));

const themeMocks = vi.hoisted(() => ({
  setTheme: vi.fn(),
  theme: 'system',
}));

vi.mock('next/navigation', () => ({
  usePathname: () => navigationMocks.pathname,
  useRouter: () => navigationMocks.router,
  useSearchParams: () => navigationMocks.searchParams,
}));

vi.mock('next-themes', () => ({
  useTheme: () => themeMocks,
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({
    loading: false,
    logout: vi.fn(),
    user: { id: 51, username: 'settings_user', is_admin: false },
  }),
}));

/** Install the tracking API fixtures used by guarded-draft tests. */
function installTrackingHandlers(): void {
  server.use(
    http.get('http://localhost/api/tracking/status', () =>
      HttpResponse.json({
        tracking_folder: { id: 4, name: 'Tracking' },
        total_folders: 1,
        weekly_articles_available: 3,
        notification_configured: false,
      }),
    ),
    http.get('http://localhost/api/tracking/push-weekly/status', idleManualPushStatusResponse),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(['fixture.sqlite'])),
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json([
        { id: 4, name: 'Tracking', is_tracking: true, article_count: 0, created_at: 1 },
      ]),
    ),
    http.get('http://localhost/api/tracking/notification-settings', () => HttpResponse.json(null)),
    http.get('http://localhost/api/tracking/ai-endpoints', () => HttpResponse.json([])),
    http.get('http://localhost/api/auth/invite-code', () => HttpResponse.json(null)),
  );
}

/** Verify every stable section parses and unrelated query state is preserved. */
function preservesSettingsQueryState(): void {
  for (const section of SETTINGS_SECTION_IDS) {
    expect(parseSettingsSection(section)).toBe(section);
  }
  expect(parseSettingsSection('unknown')).toBeNull();
  expect(parseSettingsSection(null)).toBeNull();
  expect(
    buildSettingsCenterHref(
      '/',
      new URLSearchParams('view=favorites&folder=4&settings=general&admin=users'),
      'tokens',
    ),
  ).toBe('/?view=favorites&folder=4&settings=tokens');
  expect(
    buildSettingsCenterHref(
      '/',
      new URLSearchParams('view=favorites&folder=4&settings=general'),
      null,
    ),
  ).toBe('/?view=favorites&folder=4');
}

/** Verify the query opens the named dialog and category navigation updates only its key. */
async function opensAndNavigatesSettingsDialog(): Promise<void> {
  const user = userEvent.setup();
  renderWithQuery(<SettingsCenterDialog />);

  expect(await screen.findByRole('dialog', { name: '设置中心' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '常规' })).toBeInTheDocument();
  expect(screen.getAllByRole('navigation', { name: '设置分类' })).toHaveLength(2);
  expect(screen.getByRole('region', { name: '常规设置内容' })).toBeInTheDocument();
  expect(screen.getByRole('radiogroup', { name: '外观主题' })).toBeInTheDocument();
  expect(document.querySelector('[data-mobile-overflow-cue="true"]')).toBeInTheDocument();
  expect(document.querySelector('[data-motion-section-header="general"]')).toBeInTheDocument();
  expect(document.querySelector('[data-settings-content-key="general"]')).toBeInTheDocument();
  expect(
    screen
      .getAllByRole('button', { name: '常规' })
      .every((button) => button.getAttribute('data-section-active') === 'true'),
  ).toBe(true);

  await user.click(screen.getAllByRole('button', { name: '通知与推送' })[0]);
  expect(navigationMocks.router.replace).toHaveBeenCalledWith(
    '/?view=favorites&folder=4&settings=notifications',
    { scroll: false },
  );

  navigationMocks.router.replace.mockClear();
  await user.click(screen.getByRole('button', { name: '关闭' }));
  expect(navigationMocks.router.replace).toHaveBeenCalledWith('/?view=favorites&folder=4', {
    scroll: false,
  });
}

/** Verify temporary React effect cleanup does not close URL-requested settings. */
async function keepsSettingsOpenAfterActivityResumes(): Promise<void> {
  const user = userEvent.setup();
  const view = renderWithQuery(
    <Activity mode="visible">
      <SettingsCenterDialog />
    </Activity>,
  );
  expect(await screen.findByRole('dialog', { name: '设置中心' })).toBeInTheDocument();
  view.rerender(
    <Activity mode="hidden">
      <SettingsCenterDialog />
    </Activity>,
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  view.rerender(
    <Activity mode="visible">
      <SettingsCenterDialog />
    </Activity>,
  );

  await user.click(await screen.findByRole('radio', { name: /^深色/ }));

  expect(themeMocks.setTheme).toHaveBeenCalledWith('dark');
  expect(screen.getByRole('dialog', { name: '设置中心' })).toBeInTheDocument();
  expect(navigationMocks.router.replace).not.toHaveBeenCalled();
}

/** Verify tracking and notification categories retain one shared draft controller. */
async function preservesSharedTrackingCategoryState(): Promise<void> {
  installTrackingHandlers();
  navigationMocks.searchParams = new URLSearchParams('view=favorites&folder=4&settings=tracking');
  const user = userEvent.setup();
  const view = renderWithQuery(<SettingsCenterDialog />);

  const recommendationSwitch = await screen.findByRole('switch', { name: '启用推荐' });
  await user.click(recommendationSwitch);
  const expectedCheckedState = recommendationSwitch.getAttribute('aria-checked');
  const saveButton = screen.getByRole('button', { name: '保存更改' });
  const sharedContent = document.querySelector(
    '[data-settings-content-key="tracking-notifications"]',
  );
  expect(sharedContent).toBeInTheDocument();
  expect(saveButton).toBeEnabled();

  navigationMocks.searchParams = new URLSearchParams(
    'view=favorites&folder=4&settings=notifications',
  );
  view.rerender(<SettingsCenterDialog />);

  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 2, name: '通知与推送' })).toBeInTheDocument(),
  );
  expect(document.querySelector('[data-settings-content-key="tracking-notifications"]')).toBe(
    sharedContent,
  );
  expect(screen.getByRole('button', { name: '保存更改' })).toBe(saveButton);
  expect(saveButton).toBeEnabled();

  navigationMocks.searchParams = new URLSearchParams('view=favorites&folder=4&settings=tracking');
  view.rerender(<SettingsCenterDialog />);

  await waitFor(() =>
    expect(screen.getByRole('switch', { name: '启用推荐' })).toHaveAttribute(
      'aria-checked',
      expectedCheckedState,
    ),
  );
}

/** Verify closing settings returns focus to a menu trigger after its menu portal unmounts. */
async function restoresFocusToTransientMenuTrigger(): Promise<void> {
  const user = userEvent.setup();
  const menuTrigger = document.createElement('button');
  const menu = document.createElement('div');
  const menuItem = document.createElement('a');
  menuTrigger.textContent = '账号菜单';
  menuTrigger.setAttribute('aria-controls', 'settings-source-menu');
  menuTrigger.setAttribute('aria-expanded', 'true');
  menuTrigger.setAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE, '');
  menu.id = 'settings-source-menu';
  menu.setAttribute('role', 'menu');
  menuItem.href = '/?view=favorites&folder=4&settings=general';
  menuItem.textContent = '打开设置中心';
  menuItem.setAttribute('role', 'menuitem');
  menu.tabIndex = -1;
  menu.append(menuItem);
  document.body.append(menuTrigger, menu);
  menuItem.focus();

  renderWithQuery(<SettingsCenterDialog />);
  expect(await screen.findByRole('dialog', { name: '设置中心' })).toBeInTheDocument();
  expect(menuTrigger).not.toHaveAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE);
  menu.remove();

  await user.click(screen.getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(menuTrigger).toHaveFocus());
  menuTrigger.remove();
}

/** Verify an unknown query value is removed without opening an empty dialog. */
async function normalizesUnknownSettingsSection(): Promise<void> {
  navigationMocks.searchParams = new URLSearchParams('view=favorites&folder=4&settings=unknown');
  renderWithQuery(<SettingsCenterDialog />);

  await waitFor(() =>
    expect(navigationMocks.router.replace).toHaveBeenCalledWith('/?view=favorites&folder=4', {
      scroll: false,
    }),
  );
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
}

/** Verify an unsaved recommendation draft blocks a cross-category transition until discarded. */
async function guardsUnsavedCrossCategoryNavigation(): Promise<void> {
  installTrackingHandlers();
  navigationMocks.searchParams = new URLSearchParams('view=favorites&folder=4&settings=tracking');
  const user = userEvent.setup();
  renderWithQuery(<SettingsCenterDialog />);

  await user.click(await screen.findByRole('switch', { name: '启用推荐' }));
  await user.click(screen.getAllByRole('button', { name: '账号与安全' })[0]);

  expect(screen.getByRole('alertdialog', { name: '放弃未保存的配置？' })).toBeInTheDocument();
  expect(navigationMocks.router.replace).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole('button', { name: '继续编辑' }));
  expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  expect(screen.getByRole('switch', { name: '启用推荐' })).not.toBeChecked();

  await user.click(screen.getAllByRole('button', { name: '账号与安全' })[0]);
  fireEvent.click(screen.getByRole('button', { name: '放弃更改' }));
  expect(navigationMocks.router.replace).toHaveBeenCalledWith(
    '/?view=favorites&folder=4&settings=account',
    { scroll: false },
  );
}

beforeEach(() => {
  navigationMocks.pathname = '/';
  navigationMocks.searchParams = new URLSearchParams('view=favorites&folder=4&settings=general');
  navigationMocks.router.push.mockReset();
  navigationMocks.router.replace.mockReset();
  themeMocks.setTheme.mockReset();
  themeMocks.theme = 'system';
});

describe('settings center', () => {
  test('preserves unrelated query state for every settings section', preservesSettingsQueryState);
  test('opens and navigates the query-driven dialog', opensAndNavigatesSettingsDialog);
  test(
    'keeps URL-requested settings open after Activity resumes',
    keepsSettingsOpenAfterActivityResumes,
  );
  test(
    'restores focus to a menu trigger after its portal unmounts',
    restoresFocusToTransientMenuTrigger,
  );
  test('preserves one draft across tracking categories', preservesSharedTrackingCategoryState);
  test('normalizes an unknown settings section', normalizesUnknownSettingsSection);
  test('guards unsaved cross-category navigation', guardsUnsavedCrossCategoryNavigation, 20_000);
});
