/**
 * Authenticated application frame: desktop navigation rail, main region, and mobile tab bar.
 */

import {
  IconTriggerProvider,
  Settings,
  ShieldCheck,
  useIconTrigger,
  type AppIcon,
} from '@/components/ui/animated-icons';
import { Link, useSearch } from '@tanstack/react-router';
import type { MouseEvent, PointerEvent, ReactNode } from 'react';

import { UserMenu } from '@/components/feature/user-menu';
import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { preloadAdminCenter, preloadSettingsCenter } from '@/components/shell/lazy-workspaces';
import { RailNavigation, TabNavigation } from '@/components/shell/primary-navigation';
import { HoverHighlightGroup, hoverHighlightItemProps } from '@/components/ui/hover-highlight';
import { parseAdminSection, withAdminSection } from '@/lib/admin-center';
import { useAuth } from '@/lib/auth-context';
import { WORKSPACE_ROUTE_ID, type WorkspaceSearch } from '@/lib/search-params';
import { parseSettingsSection, withSettingsSection } from '@/lib/settings-center';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

/**
 * Mark a rail shortcut as the focus target restored after its sectioned dialog closes.
 *
 * @param event - Plain current-tab click on the shortcut.
 */
function markReturnFocus(event: MouseEvent<HTMLAnchorElement>): void {
  if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
    return;
  }
  event.currentTarget.setAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE, '');
}

/**
 * Render one icon shortcut in the lower rail group.
 *
 * @param props - Destination, icon, accessible label, current state, and intent callback.
 * @returns Rail shortcut link.
 */
function RailShortcut({
  search,
  icon: Icon,
  isCurrent,
  label,
  onIntent,
}: {
  search: WorkspaceSearch;
  icon: AppIcon;
  isCurrent: boolean;
  label: string;
  onIntent: () => void;
}) {
  const { isTriggered, triggerHandlers } = useIconTrigger<PointerEvent<HTMLAnchorElement>>(
    {},
    isCurrent,
  );
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <Link
        {...triggerHandlers}
        to="/"
        search={search}
        aria-label={label}
        aria-current={isCurrent ? 'page' : undefined}
        title={label}
        onClick={markReturnFocus}
        onFocus={onIntent}
        onPointerEnter={(event) => {
          onIntent();
          triggerHandlers.onPointerEnter(event);
        }}
        onPointerDown={(event) => {
          onIntent();
          triggerHandlers.onPointerDown(event);
        }}
        {...hoverHighlightItemProps}
        {...stylex.props(styles.railShortcutLink, isCurrent && styles.railShortcutLinkCurrent)}
      >
        <Icon xstyle={styles.railShortcutIcon} strokeWidth={1.7} aria-hidden="true" />
      </Link>
    </IconTriggerProvider>
  );
}

/**
 * Render the dark desktop rail with brand, workspaces, shortcuts, and account menu.
 *
 * @returns Desktop navigation rail.
 */
function AppRail() {
  const { user } = useAuth();
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const isSettingsOpen = parseSettingsSection(search.settings) !== null;
  const isAdminOpen = parseAdminSection(search.admin) !== null;

  return (
    <aside data-slot="app-rail" {...stylex.props(styles.appRail)}>
      <Link
        to="/"
        activeOptions={{ exact: true }}
        aria-label="LitRadar 首页"
        title="LitRadar"
        {...stylex.props(styles.appRailLink)}
      >
        <img
          src="/litradar-logo.png"
          alt=""
          width={36}
          height={36}
          loading="eager"
          fetchPriority="high"
          {...stylex.props(styles.appRailImg)}
        />
      </Link>

      <RailNavigation />

      <HoverHighlightGroup xstyle={styles.appRailDiv}>
        <RailShortcut
          search={withSettingsSection(search, 'general')}
          icon={Settings}
          isCurrent={isSettingsOpen}
          label="设置中心"
          onIntent={preloadSettingsCenter}
        />
        {user?.is_admin && (
          <RailShortcut
            search={withAdminSection(search, 'overview')}
            icon={ShieldCheck}
            isCurrent={isAdminOpen}
            label="管理面板"
            onIntent={preloadAdminCenter}
          />
        )}
        <div {...stylex.props(styles.appRailDiv2)}>
          <UserMenu placement="rail" />
        </div>
      </HoverHighlightGroup>
    </aside>
  );
}

/**
 * Render the fixed mobile tab bar with workspace links and the account entry.
 *
 * @returns Mobile tab bar.
 */
function MobileTabBar() {
  return (
    <div data-slot="mobile-tab-bar" {...stylex.props(styles.mobileTabBar)}>
      <div {...stylex.props(styles.mobileTabBarDiv)}>
        <TabNavigation />
        <UserMenu placement="tab" />
      </div>
    </div>
  );
}

/**
 * Compose authenticated pages inside the rail, main landmark, and mobile tab bar.
 *
 * @param props - Page content.
 * @returns Application frame.
 */
export function AppFrame({ children }: { children: ReactNode }) {
  return (
    <div {...stylex.props(styles.appFrameDiv)}>
      <AppRail />
      <main id="main-content" {...stylex.props(styles.appFrameMain)}>
        {children}
      </main>
      <MobileTabBar />
    </div>
  );
}

const styles = stylex.create({
  railShortcutLink: {
    display: 'flex',
    width: '2.5rem',
    height: '2.5rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '12px',
    color: {
      default: colors.railForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.railActive,
      },
    },
    transitionProperty: 'color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.railHover,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  railShortcutLinkCurrent: {
    backgroundColor: {
      default: colors.railHover,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.railHover,
      },
    },
    color: {
      default: colors.railActive,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.railActive,
      },
    },
  },
  railShortcutIcon: {
    pointerEvents: 'none',
    width: '1.25rem',
    height: '1.25rem',
  },
  appRail: {
    display: {
      default: 'none',
      '@media (min-width: 48rem)': 'flex',
    },
    height: '100dvh',
    width: '4rem',
    flexShrink: 0,
    flexDirection: 'column',
    alignItems: 'center',
    borderRightStyle: 'solid',
    borderRightWidth: '1px',
    borderColor: colors.railBorder,
    backgroundColor: colors.rail,
    paddingBlock: '0.75rem',
  },
  appRailLink: {
    marginBottom: '1.25rem',
    borderRadius: '12px',
    transitionProperty: 'opacity',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    opacity: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': '85%',
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  appRailImg: {
    width: '2.25rem',
    height: '2.25rem',
    borderRadius: '10px',
    objectFit: 'cover',
  },
  appRailDiv: {
    marginTop: 'auto',
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    gap: '0.375rem',
  },
  appRailDiv2: {
    marginTop: '0.5rem',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    borderColor: colors.railBorder,
    paddingTop: '0.75rem',
  },
  mobileTabBar: {
    position: 'fixed',
    insetInline: '0px',
    bottom: '0px',
    zIndex: 40,
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    backgroundColor: colors.background,
    paddingBottom: 'var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px))',
    display: {
      default: null,
      '@media (min-width: 48rem)': 'none',
    },
  },
  mobileTabBarDiv: {
    display: 'grid',
    gridTemplateColumns: 'repeat(6, minmax(0, 1fr))',
  },
  appFrameDiv: {
    display: 'flex',
    height: '100dvh',
    width: '100%',
    overflow: 'hidden',
    backgroundColor: colors.background,
    color: colors.foreground,
  },
  appFrameMain: {
    position: 'relative',
    display: 'flex',
    height: '100dvh',
    minWidth: '0px',
    flex: '1',
    flexDirection: 'column',
  },
});
