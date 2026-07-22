/**
 * Authenticated user menu navigation, focus, and theme coverage.
 */

import type { ReactNode } from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import Providers from '@/app/providers';
import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { UserMenu } from '@/components/feature/user-menu';

type MockUser = {
  id: number;
  username: string;
  is_admin: boolean;
};

const userMenuMocks = vi.hoisted(() => ({
  auth: {
    loading: false,
    logout: vi.fn().mockResolvedValue(undefined),
    user: {
      id: 21,
      username: 'menu_user',
      is_admin: false,
    } as MockUser | null,
  },
  pathname: '/',
  providerProps: vi.fn(),
  searchParams: new URLSearchParams('view=favorites&folder=4'),
  setTheme: vi.fn(),
  theme: 'system',
}));

vi.mock('next/navigation', () => ({
  usePathname: () => userMenuMocks.pathname,
  useSearchParams: () => userMenuMocks.searchParams,
}));

vi.mock('@/lib/auth-context', () => ({
  AuthProvider: ({ children }: { children: ReactNode }) => children,
  useAuth: () => userMenuMocks.auth,
}));

vi.mock('next-themes', () => ({
  ThemeProvider: ({ children, ...props }: { children: ReactNode }) => {
    userMenuMocks.providerProps(props);
    return children;
  },
  useTheme: () => ({
    setTheme: userMenuMocks.setTheme,
    theme: userMenuMocks.theme,
  }),
}));

vi.mock('nuqs/adapters/next/app', () => ({
  NuqsAdapter: ({ children }: { children: ReactNode }) => children,
}));

/**
 * Restore the default authenticated non-admin fixture.
 */
function resetUserMenuMocks(): void {
  userMenuMocks.auth.loading = false;
  userMenuMocks.auth.user = {
    id: 21,
    username: 'menu_user',
    is_admin: false,
  };
  userMenuMocks.pathname = '/';
  userMenuMocks.searchParams = new URLSearchParams('view=favorites&folder=4');
  userMenuMocks.theme = 'system';
}

/**
 * Prevent jsdom from attempting document navigation during link interaction tests.
 *
 * @param event - Native anchor click event.
 */
function preventNavigation(event: Event): void {
  event.preventDefault();
}

/**
 * Verify loading and anonymous states expose no global menu control.
 */
function hidesMenuWithoutAuthenticatedUser(): void {
  userMenuMocks.auth.loading = true;
  render(<UserMenu />);
  expect(screen.queryByRole('button', { name: /打开账号菜单/ })).not.toBeInTheDocument();

  userMenuMocks.auth.loading = false;
  userMenuMocks.auth.user = null;
  render(<UserMenu />);
  expect(screen.queryByRole('button', { name: /打开账号菜单/ })).not.toBeInTheDocument();
}

/**
 * Verify account actions, query preservation, admin gating, and logout.
 */
async function exposesAccountActions(): Promise<void> {
  const user = userEvent.setup();
  const { rerender } = render(<UserMenu />);
  const trigger = screen.getByRole('button', { name: '打开账号菜单：menu_user' });

  expect(trigger).toHaveTextContent('menu_user');
  await user.click(trigger);
  expect(screen.getByRole('menuitem', { name: '打开设置中心' })).toHaveAttribute(
    'href',
    '/?view=favorites&folder=4&settings=general',
  );
  expect(screen.getByRole('menuitem', { name: '外观主题' })).toBeInTheDocument();
  expect(screen.queryByRole('menuitem', { name: '文献检索' })).not.toBeInTheDocument();
  expect(screen.queryByRole('menuitem', { name: '我的收藏' })).not.toBeInTheDocument();
  expect(screen.queryByRole('menuitem', { name: '每周更新' })).not.toBeInTheDocument();
  expect(screen.queryByRole('menuitem', { name: '管理面板' })).not.toBeInTheDocument();

  await user.click(screen.getByRole('menuitem', { name: '退出登录' }));
  expect(userMenuMocks.auth.logout).toHaveBeenCalledOnce();

  userMenuMocks.auth.user = {
    id: 22,
    username: 'admin_user',
    is_admin: true,
  };
  rerender(<UserMenu />);
  await user.click(screen.getByRole('button', { name: '打开账号菜单：admin_user' }));
  expect(screen.getByRole('menuitem', { name: '管理面板' })).toHaveAttribute('href', '/admin');
}

/**
 * Verify Escape closes the menu and restores focus to its trigger.
 */
async function restoresTriggerFocusAfterEscape(): Promise<void> {
  const user = userEvent.setup();
  render(<UserMenu />);
  const trigger = screen.getByRole('button', { name: /打开账号菜单/ });

  await user.click(trigger);
  expect(screen.getByRole('menu')).toBeInTheDocument();
  await user.keyboard('{Escape}');

  await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
  expect(trigger).toHaveFocus();
}

/**
 * Verify only an unmodified current-tab settings selection marks the persistent trigger.
 */
async function marksCurrentTabSettingsInitiator(): Promise<void> {
  const user = userEvent.setup();
  render(<UserMenu />);
  const trigger = screen.getByRole('button', { name: /打开账号菜单/ });

  await user.click(trigger);
  const modifiedSettingsLink = screen.getByRole('menuitem', { name: '打开设置中心' });
  modifiedSettingsLink.addEventListener('click', preventNavigation);
  fireEvent.click(modifiedSettingsLink, { ctrlKey: true });
  expect(trigger).not.toHaveAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE);

  fireEvent.click(modifiedSettingsLink);
  expect(trigger).toHaveAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE);
  trigger.removeAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE);
}

/**
 * Verify all supported theme choices call next-themes with stable values.
 */
async function selectsThemePreferences(): Promise<void> {
  const user = userEvent.setup();
  render(<UserMenu />);
  const trigger = screen.getByRole('button', { name: /打开账号菜单/ });

  await user.click(trigger);
  await user.hover(screen.getByRole('menuitem', { name: '外观主题' }));
  fireEvent.click(await screen.findByRole('menuitemradio', { name: '深色' }));
  expect(userMenuMocks.setTheme).toHaveBeenLastCalledWith('dark');

  await user.click(trigger);
  await user.hover(screen.getByRole('menuitem', { name: '外观主题' }));
  fireEvent.click(await screen.findByRole('menuitemradio', { name: '浅色' }));
  expect(userMenuMocks.setTheme).toHaveBeenLastCalledWith('light');

  await user.click(trigger);
  await user.hover(screen.getByRole('menuitem', { name: '外观主题' }));
  fireEvent.click(await screen.findByRole('menuitemradio', { name: '跟随系统' }));
  expect(userMenuMocks.setTheme).toHaveBeenLastCalledWith('system');
}

/**
 * Verify the application provider enables the system theme by default.
 */
function configuresSystemThemeProvider(): void {
  render(
    <Providers>
      <span>provider child</span>
    </Providers>,
  );

  expect(userMenuMocks.providerProps).toHaveBeenCalledWith(
    expect.objectContaining({
      attribute: 'class',
      defaultTheme: 'system',
      enableSystem: true,
    }),
  );
}

beforeEach(resetUserMenuMocks);

describe('UserMenu', () => {
  test('stays hidden before authentication completes', hidesMenuWithoutAuthenticatedUser);
  test('exposes account actions and admin gating', exposesAccountActions);
  test('restores trigger focus after Escape', restoresTriggerFocusAfterEscape);
  test(
    'marks only current-tab settings navigation for focus return',
    marksCurrentTabSettingsInitiator,
  );
  test('selects system, light, and dark themes', selectsThemePreferences);
  test('configures system theme as the application default', configuresSystemThemeProvider);
});
