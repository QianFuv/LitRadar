/**
 * Accessible account controls, settings entry, theme preferences, and logout.
 */

import { Menu as DropdownMenuPrimitive } from '@animate-ui/react/primitives/base-ui/menu';
import { Menu } from '@base-ui/react/menu';
import {
  AlertTriangle,
  IconStyleProvider,
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
import { useRef, type ComponentProps, type MouseEvent } from 'react';
import { createPortal } from 'react-dom';

import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { preloadAdminCenter, preloadSettingsCenter } from '@/components/shell/lazy-workspaces';
import { UserAvatar } from '@/components/shell/user-avatar';
import { controlIconStyle } from '@/components/ui/button';
import { menuItemIconStyle } from '@/components/ui/dropdown-menu';
import { parseAdminSection, withAdminSection } from '@/lib/admin-center';
import { useAuth, type LogoutRevocationWarning } from '@/lib/auth-context';
import { WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { withSettingsSection } from '@/lib/settings-center';
import { getFocusedOrigin, revealThemeChange } from '@/lib/theme-transition';
import { popupLayerStyle } from '@/components/ui/popup-layer';
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
 * Render durable warning metadata when server-side logout was not confirmed.
 *
 * @param props - Persisted warning metadata.
 * @returns Recovery notice linking to fresh reauthentication.
 */
function LogoutRevocationNotice({ warning }: { warning: LogoutRevocationWarning }) {
  return (
    <div role="alert" {...stylex.props(styles.logoutRevocationNoticeDiv)}>
      <div {...stylex.props(styles.logoutRevocationNoticeDiv2)}>
        <AlertTriangle xstyle={styles.logoutRevocationNoticeAlertTriangle} aria-hidden="true" />
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
 * @param props - Persisted warning metadata.
 * @returns Notice portaled into the document body.
 */
function FixedLogoutRevocationNotice({ warning }: { warning: LogoutRevocationWarning }) {
  const notice = (
    <div data-slot="user-menu-position" {...stylex.props(styles.userMenuPosition)}>
      <LogoutRevocationNotice warning={warning} />
    </div>
  );
  return createPortal(notice, document.body);
}

/**
 * Render an account menu item whose icon plays while it is hovered or tapped. It uses the plain
 * Base UI item, so hover paints the item's own background instead of a highlight sliding between
 * items.
 *
 * @param props - Base UI menu item properties.
 * @returns Account menu item.
 */
function UserMenuItem(props: ComponentProps<typeof Menu.Item>) {
  const { isTriggered, triggerHandlers } = useIconTrigger(props);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <IconStyleProvider xstyle={[controlIconStyle, menuItemIconStyle]}>
        <Menu.Item data-slot="user-menu-item" {...props} {...triggerHandlers} />
      </IconStyleProvider>
    </IconTriggerProvider>
  );
}

/**
 * Render a theme choice whose icon plays while it is hovered or tapped, painted like
 * `UserMenuItem`.
 *
 * @param props - Base UI menu radio item properties.
 * @returns Theme radio item.
 */
function ThemeRadioItem(props: ComponentProps<typeof Menu.RadioItem>) {
  const { isTriggered, triggerHandlers } = useIconTrigger(props);
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <Menu.RadioItem {...props} {...triggerHandlers} />
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

  if (loading) {
    return null;
  }

  if (!user) {
    return logoutWarning && placement === 'rail' ? (
      <FixedLogoutRevocationNotice warning={logoutWarning} />
    ) : null;
  }

  const selectedTheme = theme ?? 'system';
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
    selectedTheme,
    handleThemeChange,
  };
  return (
    <>
      {placement === 'rail' && logoutWarning && (
        <FixedLogoutRevocationNotice warning={logoutWarning} />
      )}
      <DropdownMenuPrimitive.Root>
        <DropdownMenuPrimitive.Trigger
          ref={accountTriggerRef}
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
        </DropdownMenuPrimitive.Trigger>

        <DropdownMenuPrimitive.Portal {...stylex.props(popupLayerStyle)}>
          <DropdownMenuPrimitive.Positioner
            align="end"
            side={placement === 'rail' ? 'right' : 'top'}
            sideOffset={placement === 'rail' ? 14 : 6}
            collisionPadding={8}
            collisionAvoidance={{ align: 'shift', fallbackAxisSide: 'none' }}
            {...stylex.props(styles.menuPositioner)}
          >
            <DropdownMenuPrimitive.Popup
              aria-label="账号菜单"
              {...stylex.props(styles.menuContent, styles.userMenuDropdownMenuPrimitiveContent)}
            >
              <div {...stylex.props(styles.userMenuDropdownMenuPrimitiveLabel)}>
                <UserAvatar username={user.username} size="lg" />
                <span {...stylex.props(styles.logoutRevocationNoticeDiv3)}>
                  <span {...stylex.props(styles.userMenuSpan3)}>{user.username}</span>
                  <span {...stylex.props(styles.userMenuSpan4)}>
                    {user.is_admin ? '管理员' : '普通用户'}
                  </span>
                </span>
              </div>

              <DropdownMenuPrimitive.Separator
                {...stylex.props(styles.userMenuDropdownMenuPrimitiveSeparator)}
              />

              <UserMenuItem
                render={
                  <Link
                    to="/"
                    search={settingsSearch}
                    onClick={handleSectionedDialogOpen}
                    onFocus={preloadSettingsCenter}
                    onPointerEnter={preloadSettingsCenter}
                  />
                }
                {...stylex.props(styles.menuItem)}
              >
                <Settings />
                <span>打开设置中心</span>
              </UserMenuItem>

              {renderUserThemeControls(state)}

              {user.is_admin && (
                <UserMenuItem
                  render={
                    <Link
                      to="/"
                      search={adminSearch}
                      aria-current={isAdminOpen ? 'page' : undefined}
                      onClick={handleSectionedDialogOpen}
                      onFocus={preloadAdminCenter}
                      onPointerEnter={preloadAdminCenter}
                    />
                  }
                  {...stylex.props(styles.menuItem, isAdminOpen && styles.userMenuLinkAdminOpen)}
                >
                  <ShieldCheck />
                  <span>管理面板</span>
                </UserMenuItem>
              )}

              <DropdownMenuPrimitive.Separator
                {...stylex.props(styles.userMenuDropdownMenuPrimitiveSeparator)}
              />

              <UserMenuItem
                {...stylex.props(styles.menuItem, styles.userMenuUserMenuItem)}
                onClick={handleLogout}
              >
                <LogOut xstyle={styles.userMenuLogOut} />
                <span>退出登录</span>
              </UserMenuItem>
            </DropdownMenuPrimitive.Popup>
          </DropdownMenuPrimitive.Positioner>
        </DropdownMenuPrimitive.Portal>
      </DropdownMenuPrimitive.Root>
    </>
  );
}

/**
 * Render the hydration-gated theme choices. Radio items keep the menu open after a choice, so the
 * user can compare themes or switch back.
 *
 * @param state - Selected theme and theme change handler.
 * @returns Theme choice row.
 */
function renderUserThemeControls(state: UserMenuViewState) {
  const { selectedTheme, handleThemeChange } = state;

  return (
    <div data-slot="user-menu-theme-label" {...stylex.props(styles.userMenuThemeLabel)}>
      <span {...stylex.props(styles.renderUserThemeControlsSpan)}>
        <SunMoon xstyle={styles.renderUserThemeControlsSunMoon} aria-hidden="true" />
        <span>外观主题</span>
      </span>
      <Menu.RadioGroup
        aria-label="外观主题"
        value={selectedTheme}
        onValueChange={handleThemeChange}
        {...stylex.props(styles.themeRadioGroup)}
      >
        {THEME_ITEMS.map((item) => {
          const Icon = item.icon;

          return (
            <ThemeRadioItem
              key={item.value}
              value={item.value}
              label={item.label}
              aria-label={item.label}
              title={item.label}
              {...stylex.props(styles.renderUserThemeControlsThemeRadioItem)}
            >
              <Icon xstyle={styles.renderUserThemeControlsIcon} aria-hidden="true" />
            </ThemeRadioItem>
          );
        })}
      </Menu.RadioGroup>
    </div>
  );
}

type UserMenuViewState = {
  selectedTheme: string;
  handleThemeChange: (value: string) => void;
};

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
      ':is([data-popup-open])': colors.primaryText,
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
      [stylex.when.ancestor('[data-popup-open]', accountMenuMarker)]:
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
  menuPositioner: {
    zIndex: 50,
  },
  menuContent: {
    transformOrigin: 'var(--transform-origin)',
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
  themeRadioGroup: {
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
      ':is([data-checked])': colors.foreground,
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
      ':is([data-checked])': colors.accent,
    },
  },
  renderUserThemeControlsIcon: {
    width: '1rem',
    height: '1rem',
  },
});
