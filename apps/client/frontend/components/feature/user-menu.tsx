/**
 * Accessible account controls, settings entry, theme preferences, and logout.
 */

import * as DropdownMenuPrimitive from '@animate-ui/react/primitives/radix-ui/dropdown-menu';
import { DropdownMenu as RadixDropdownMenu } from 'radix-ui';
import {
  AlertTriangle,
  IconTriggerProvider,
  LogOut,
  Moon,
  Settings,
  ShieldCheck,
  Sun,
  SunMoon,
  useIconTrigger,
  type AppIcon,
} from '@/components/ui/animated-icons';
import { useTheme } from 'next-themes';
import { Link, useSearch } from '@tanstack/react-router';
import { useRef, useSyncExternalStore, type ComponentProps, type MouseEvent } from 'react';
import { createPortal } from 'react-dom';

import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { preloadAdminCenter, preloadSettingsCenter } from '@/components/shell/lazy-workspaces';
import { UserAvatar } from '@/components/shell/user-avatar';
import { keepRadixCloseFocus } from '@/components/ui/dropdown-menu';
import { parseAdminSection, withAdminSection } from '@/lib/admin-center';
import { useAuth, type LogoutRevocationWarning } from '@/lib/auth-context';
import { WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { withSettingsSection } from '@/lib/settings-center';
import { getFocusedOrigin, revealThemeChange } from '@/lib/theme-transition';
import * as stylex from '@stylexjs/stylex';
import { accountMenuMarker } from '@/lib/markers.stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

type ThemePreference = 'system' | 'light' | 'dark';

type UserMenuProps = {
  placement?: 'rail' | 'tab';
};

type ThemeItem = {
  icon: AppIcon;
  label: string;
  value: ThemePreference;
};

const THEME_ITEMS: readonly ThemeItem[] = [
  { icon: SunMoon, label: '跟随系统', value: 'system' },
  { icon: Sun, label: '浅色', value: 'light' },
  { icon: Moon, label: '深色', value: 'dark' },
];

/**
 * Subscribe to the immutable client-environment signal.
 *
 * @returns No-op unsubscribe callback.
 */
function subscribeToClientEnvironment(): () => void {
  return () => undefined;
}

/**
 * Return the browser snapshot for hydration-safe client detection.
 *
 * @returns Always true in the browser.
 */
function getClientEnvironmentSnapshot(): boolean {
  return true;
}

/**
 * Return the server snapshot for hydration-safe client detection.
 *
 * @returns Always false during server rendering and hydration.
 */
function getServerEnvironmentSnapshot(): boolean {
  return false;
}

/**
 * Render durable warning metadata when server-side logout was not confirmed.
 *
 * @param props - Persisted warning metadata.
 * @returns Recovery notice linking to fresh reauthentication.
 */
function LogoutRevocationNotice({ warning }: { warning: LogoutRevocationWarning }) {
  return (
    <div role="alert" {...stylex.props(styles.logoutRevocationNoticeDiv)}>
      <div {...stylex.props(styles.logoutRevocationNoticeDiv2)}>
        <AlertTriangle
          {...stylex.props(styles.logoutRevocationNoticeAlertTriangle)}
          aria-hidden="true"
        />
        <div {...stylex.props(styles.logoutRevocationNoticeDiv3)}>
          <p
            {...stylex.props(styles.logoutRevocationNoticeDivItem, styles.logoutRevocationNoticeP)}
          >
            服务端会话撤销未确认
          </p>
          <p
            {...stylex.props(styles.logoutRevocationNoticeDivItem, styles.logoutRevocationNoticeP2)}
          >
            本地会话信息已清除，但旧令牌可能仍有效。请重新认证后撤销全部会话。
          </p>
          {warning.requestId && (
            <p
              {...stylex.props(
                styles.logoutRevocationNoticeDivItem,
                styles.logoutRevocationNoticeP3,
              )}
            >
              请求 ID：{warning.requestId}
            </p>
          )}
          <Link
            to="/login"
            search={{ logout_recovery: '1' }}
            {...stylex.props(
              styles.logoutRevocationNoticeDivItem,
              styles.logoutRevocationNoticeLink,
            )}
          >
            重新认证并撤销全部会话
          </Link>
        </div>
      </div>
    </div>
  );
}

/**
 * Render the logout recovery notice in a fixed layer that stays visible on every breakpoint.
 *
 * @param props - Persisted warning metadata and hydration state.
 * @returns Portaled notice once mounted in the browser.
 */
function FixedLogoutRevocationNotice({
  isMounted,
  warning,
}: {
  isMounted: boolean;
  warning: LogoutRevocationWarning;
}) {
  const notice = (
    <div data-slot="user-menu-position" {...stylex.props(styles.userMenuPosition)}>
      <LogoutRevocationNotice warning={warning} />
    </div>
  );
  return isMounted ? createPortal(notice, document.body) : notice;
}

/**
 * Render an account menu item whose icon plays while it is hovered or tapped. It uses the plain
 * Radix item, so hover paints the item's own background instead of a highlight sliding between
 * items.
 *
 * @param props - Radix menu item properties.
 * @returns Account menu item.
 */
function UserMenuItem(props: ComponentProps<typeof RadixDropdownMenu.Item>) {
  const { isTriggered, triggerHandlers } = useIconTrigger(props);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <RadixDropdownMenu.Item data-slot="user-menu-item" {...props} {...triggerHandlers} />
    </IconTriggerProvider>
  );
}

/**
 * Render a theme choice whose icon plays while it is hovered or tapped, painted like
 * `UserMenuItem`.
 *
 * @param props - Radix radio item properties.
 * @returns Theme radio item.
 */
function ThemeRadioItem(props: ComponentProps<typeof RadixDropdownMenu.RadioItem>) {
  const { isTriggered, triggerHandlers } = useIconTrigger(props);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <RadixDropdownMenu.RadioItem {...props} {...triggerHandlers} />
    </IconTriggerProvider>
  );
}

/**
 * Render the authenticated account trigger and account-only menu.
 *
 * @param props - Rail or mobile tab-bar placement.
 * @returns Account menu or null while authentication is unresolved.
 */
export function UserMenu({ placement = 'rail' }: UserMenuProps) {
  const { user, loading, logout, logoutWarning } = useAuth();
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const { setTheme, theme } = useTheme();
  const accountTriggerRef = useRef<HTMLButtonElement>(null);
  const isMounted = useSyncExternalStore(
    subscribeToClientEnvironment,
    getClientEnvironmentSnapshot,
    getServerEnvironmentSnapshot,
  );

  if (loading) {
    return null;
  }

  if (!user) {
    return logoutWarning && placement === 'rail' ? (
      <FixedLogoutRevocationNotice isMounted={isMounted} warning={logoutWarning} />
    ) : null;
  }

  const selectedTheme = getUserMenuTheme(isMounted, theme);
  const settingsSearch = withSettingsSection(search, 'general');
  const adminSearch = withAdminSection(search, 'overview');
  const isAdminOpen = parseAdminSection(search.admin) !== null;

  /**
   * Clear the authenticated session after the menu selection closes.
   */
  function handleLogout(): void {
    void logout().catch(() => undefined);
  }

  /**
   * Mark the persistent account trigger before current-tab sectioned-dialog navigation.
   *
   * @param event - Sectioned-dialog link click event.
   */
  function handleSectionedDialogOpen(event: MouseEvent<HTMLAnchorElement>): void {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
      return;
    }
    accountTriggerRef.current?.setAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE, '');
  }

  /**
   * Persist the selected application theme, revealing it from the chosen menu item.
   *
   * @param value - Selected next-themes preference.
   */
  function handleThemeChange(value: string): void {
    revealThemeChange(() => setTheme(value), getFocusedOrigin());
  }

  const state = {
    isMounted,
    selectedTheme,
    handleThemeChange,
  };
  return (
    <>
      {placement === 'rail' && logoutWarning && (
        <FixedLogoutRevocationNotice isMounted={isMounted} warning={logoutWarning} />
      )}
      <DropdownMenuPrimitive.Root>
        <DropdownMenuPrimitive.Trigger asChild>
          <button
            ref={accountTriggerRef}
            type="button"
            data-slot="user-menu-trigger"
            data-placement={placement}
            {...stylex.props(
              styles.userMenuTrigger,
              accountMenuMarker,
              placement === 'rail' ? styles.userMenuTriggerRail : styles.userMenuTriggerNotRail,
            )}
            aria-label={`打开账号菜单：${user.username}`}
          >
            <UserAvatar
              username={user.username}
              size={placement === 'rail' ? 'md' : 'sm'}
              xstyle={[
                styles.userMenuUserAvatar,
                placement === 'rail' && styles.userMenuUserAvatarRail,
              ]}
            />
            {placement === 'tab' && <span {...stylex.props(styles.userMenuSpan)}>我的</span>}
            <span {...stylex.props(styles.userMenuSpan2)}>{user.username}</span>
          </button>
        </DropdownMenuPrimitive.Trigger>

        <DropdownMenuPrimitive.Portal>
          <DropdownMenuPrimitive.Content
            aria-label="账号菜单"
            align="end"
            side={placement === 'rail' ? 'right' : 'top'}
            sideOffset={placement === 'rail' ? 14 : 6}
            collisionPadding={8}
            {...stylex.props(styles.menuContent, styles.userMenuDropdownMenuPrimitiveContent)}
            onCloseAutoFocus={keepRadixCloseFocus}
          >
            <DropdownMenuPrimitive.Label
              {...stylex.props(styles.userMenuDropdownMenuPrimitiveLabel)}
            >
              <UserAvatar username={user.username} size="lg" />
              <span {...stylex.props(styles.logoutRevocationNoticeDiv3)}>
                <span {...stylex.props(styles.userMenuSpan3)}>{user.username}</span>
                <span {...stylex.props(styles.userMenuSpan4)}>
                  {user.is_admin ? '管理员' : '普通用户'}
                </span>
              </span>
            </DropdownMenuPrimitive.Label>

            <DropdownMenuPrimitive.Separator
              {...stylex.props(styles.userMenuDropdownMenuPrimitiveSeparator)}
            />

            <UserMenuItem asChild>
              <Link
                to="/"
                search={settingsSearch}
                {...stylex.props(styles.menuItem)}
                onClick={handleSectionedDialogOpen}
                onFocus={preloadSettingsCenter}
                onPointerEnter={preloadSettingsCenter}
              >
                <Settings />
                <span>打开设置中心</span>
              </Link>
            </UserMenuItem>

            {renderUserThemeControls(state)}

            {user.is_admin && (
              <UserMenuItem asChild>
                <Link
                  to="/"
                  search={adminSearch}
                  aria-current={isAdminOpen ? 'page' : undefined}
                  {...stylex.props(styles.menuItem, isAdminOpen && styles.userMenuLinkAdminOpen)}
                  onClick={handleSectionedDialogOpen}
                  onFocus={preloadAdminCenter}
                  onPointerEnter={preloadAdminCenter}
                >
                  <ShieldCheck />
                  <span>管理面板</span>
                </Link>
              </UserMenuItem>
            )}

            <DropdownMenuPrimitive.Separator
              {...stylex.props(styles.userMenuDropdownMenuPrimitiveSeparator)}
            />

            <UserMenuItem
              {...stylex.props(styles.menuItem, styles.userMenuUserMenuItem)}
              onSelect={handleLogout}
            >
              <LogOut {...stylex.props(styles.userMenuLogOut)} />
              <span>退出登录</span>
            </UserMenuItem>
          </DropdownMenuPrimitive.Content>
        </DropdownMenuPrimitive.Portal>
      </DropdownMenuPrimitive.Root>
    </>
  );
}

/**
 * Keep the account menu open after a theme choice so the user can compare or switch back.
 *
 * @param event - Radix menu item selection event.
 */
function keepMenuOpenOnSelect(event: Event): void {
  event.preventDefault();
}

/** Retain hydration-gated theme choices and exact Radix radio structure. */
function renderUserThemeControls(state: UserMenuViewState) {
  const { isMounted, selectedTheme, handleThemeChange } = state;

  return (
    <div data-slot="user-menu-theme-label" {...stylex.props(styles.userMenuThemeLabel)}>
      <span {...stylex.props(styles.renderUserThemeControlsSpan)}>
        <SunMoon {...stylex.props(styles.renderUserThemeControlsSunMoon)} aria-hidden="true" />
        <span>外观主题</span>
      </span>
      {isMounted && (
        <RadixDropdownMenu.RadioGroup
          aria-label="外观主题"
          value={selectedTheme}
          onValueChange={handleThemeChange}
          {...stylex.props(styles.renderUserThemeControlsRadixDropdownMenuRadioGroup)}
        >
          {THEME_ITEMS.map((item) => {
            const Icon = item.icon;

            return (
              <ThemeRadioItem
                key={item.value}
                value={item.value}
                textValue={item.label}
                onSelect={keepMenuOpenOnSelect}
                aria-label={item.label}
                title={item.label}
                {...stylex.props(styles.renderUserThemeControlsThemeRadioItem)}
              >
                <Icon {...stylex.props(styles.renderUserThemeControlsIcon)} aria-hidden="true" />
              </ThemeRadioItem>
            );
          })}
        </RadixDropdownMenu.RadioGroup>
      )}
    </div>
  );
}

type UserMenuViewState = {
  isMounted: boolean;
  selectedTheme: string;
  handleThemeChange: (value: string) => void;
};

/** Resolve the theme only after the client environment is mounted. */
function getUserMenuTheme(isMounted: boolean, theme: string | undefined): string {
  return isMounted ? (theme ?? 'system') : 'system';
}

const styles = stylex.create({
  logoutRevocationNoticeDiv: {
    width: 'min(24rem, calc(100vw - 2rem))',
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.destructive} 30%, transparent)`,
    backgroundColor: colors.popover,
    padding: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.popoverForeground,
    boxShadow: shadows.overlay,
  },
  logoutRevocationNoticeDiv2: {
    display: 'flex',
    alignItems: 'flex-start',
    gap: '0.5rem',
  },
  logoutRevocationNoticeAlertTriangle: {
    marginTop: '0.125rem',
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    color: colors.destructive,
  },
  logoutRevocationNoticeDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  logoutRevocationNoticeDiv3: {
    minWidth: '0px',
  },
  logoutRevocationNoticeP: {
    fontWeight: 500,
  },
  logoutRevocationNoticeP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  logoutRevocationNoticeP3: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    wordBreak: 'break-all',
    color: colors.mutedForeground,
  },
  logoutRevocationNoticeLink: {
    display: 'inline-flex',
    height: '2rem',
    alignItems: 'center',
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: colors.input,
    backgroundColor: {
      default: colors.background,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    paddingInline: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 500,
    transitionProperty: 'background-color, color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    color: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  userMenuPosition: {
    position: 'fixed',
    zIndex: 40,
    right: 'calc(1rem + env(safe-area-inset-right, 0px))',
    bottom: {
      default: 'calc(4.5rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px)))',
      '@media (min-width: 48rem)': '1rem',
    },
  },
  userMenuTrigger: {
    outlineStyle: 'none',
    transitionDuration: '200ms',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
  },
  userMenuTriggerRail: {
    display: 'flex',
    width: '2.75rem',
    height: '2.75rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  userMenuTriggerNotRail: {
    display: 'flex',
    height: '3.5rem',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    gap: '0.25rem',
    color: {
      default: colors.mutedForeground,
      ':is([data-state="open"])': colors.primaryText,
    },
    boxShadow: {
      default: null,
      ':focus-visible': `inset 0 0 0 2px ${colors.ring}`,
    },
  },
  userMenuUserAvatar: {
    transitionProperty: 'box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.4, 0, 0.2, 1)',
    transitionDuration: '150ms',
  },
  userMenuUserAvatarRail: {
    boxShadow: {
      default: `0 0 0 2px ${colors.rail}, 0 0 0 3px ${colors.railBorder}`,
      [stylex.when.ancestor('[data-state="open"]', accountMenuMarker)]:
        `0 0 0 2px ${colors.rail}, 0 0 0 3px ${colors.primary}`,
      '@media (hover: hover)': {
        default: null,
        [stylex.when.ancestor(':hover', accountMenuMarker)]:
          `0 0 0 2px ${colors.rail}, 0 0 0 3px ${colors.railForeground}`,
      },
    },
  },
  userMenuSpan: {
    fontSize: '11px',
    lineHeight: 1,
  },
  userMenuSpan2: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  menuContent: {
    zIndex: 50,
    transformOrigin: 'var(--radix-dropdown-menu-content-transform-origin)',
    borderRadius: '12px',
    backgroundColor: colors.popover,
    padding: '0.375rem',
    color: colors.popoverForeground,
    boxShadow: shadows.overlay,
    outlineStyle: {
      default: 'none',
      '@media (forced-colors: active)': 'solid',
    },
    outlineWidth: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    outlineColor: {
      default: null,
      '@media (forced-colors: active)': 'transparent',
    },
    outlineOffset: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
  },
  userMenuDropdownMenuPrimitiveContent: {
    width: '16rem',
    maxWidth: 'calc(100vw - 1rem)',
  },
  userMenuDropdownMenuPrimitiveLabel: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
    paddingInline: '0.5rem',
    paddingBlock: '0.5rem',
  },
  userMenuSpan3: {
    display: 'block',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  userMenuSpan4: {
    display: 'block',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 400,
    color: colors.mutedForeground,
  },
  userMenuDropdownMenuPrimitiveSeparator: {
    marginInline: '-0.25rem',
    marginBlock: '0.25rem',
    height: '1px',
    backgroundColor: colors.border,
  },
  menuItem: {
    position: 'relative',
    display: 'flex',
    width: '100%',
    cursor: 'default',
    alignItems: 'center',
    gap: '0.5rem',
    borderRadius: '4px',
    paddingInline: '0.5rem',
    paddingBlock: '0.375rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    outlineStyle: {
      default: 'none',
      '@media (forced-colors: active)': 'solid',
    },
    transitionProperty: 'background-color, color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    userSelect: 'none',
    outlineWidth: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    outlineColor: {
      default: null,
      '@media (forced-colors: active)': 'transparent',
    },
    outlineOffset: {
      default: null,
      '@media (forced-colors: active)': '2px',
    },
    backgroundColor: {
      default: null,
      ':focus': colors.accent,
      ':active': {
        default: colors.accentPressed,
        ':is([data-highlighted])': colors.accentPressed,
      },
      ':is([data-highlighted])': colors.accent,
    },
    color: {
      default: null,
      ':focus': colors.accentForeground,
    },
    pointerEvents: {
      default: null,
      ':is([data-disabled])': 'none',
    },
    opacity: {
      default: null,
      ':is([data-disabled])': '50%',
    },
  },
  userMenuLinkAdminOpen: {
    backgroundColor: {
      default: colors.accent,
      ':active': {
        default: colors.accentPressed,
        ':is([data-highlighted])': colors.accentPressed,
      },
      ':focus': colors.accent,
      ':is([data-highlighted])': colors.accent,
    },
  },
  userMenuUserMenuItem: {
    color: {
      default: colors.destructive,
      ':focus': colors.destructive,
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':is([data-highlighted])': colors.accentPressed,
      },
      ':focus': {
        default: `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
        ':is([data-highlighted])': colors.accent,
      },
      ':is([data-highlighted])': colors.accent,
    },
  },
  userMenuLogOut: {
    color: colors.destructive,
  },
  userMenuThemeLabel: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.5rem',
    paddingInline: '0.5rem',
    paddingBlock: '0.375rem',
  },
  renderUserThemeControlsSpan: {
    display: 'flex',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderUserThemeControlsSunMoon: {
    width: '1rem',
    height: '1rem',
    color: colors.mutedForeground,
  },
  renderUserThemeControlsRadixDropdownMenuRadioGroup: {
    display: 'flex',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.25rem',
  },
  renderUserThemeControlsThemeRadioItem: {
    display: 'flex',
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.25rem',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.25rem',
    },
    flexShrink: 0,
    cursor: 'default',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '6px',
    color: {
      default: colors.mutedForeground,
      ':is([data-highlighted])': colors.foreground,
      ':is([data-state="checked"])': colors.foreground,
    },
    transitionProperty: 'background-color, color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
    backgroundColor: {
      default: null,
      ':is([data-highlighted])': colors.accent,
      ':is([data-state="checked"])': colors.accent,
    },
  },
  renderUserThemeControlsIcon: {
    width: '1rem',
    height: '1rem',
  },
});
