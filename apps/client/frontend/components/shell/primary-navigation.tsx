/**
 * Primary workspace navigation rendered as a desktop rail or a mobile tab bar.
 */

import {
  CalendarDays,
  Megaphone,
  Search,
  Share2,
  Star,
  IconTriggerProvider,
  useIconTrigger,
  type AppIcon,
} from '@/components/ui/animated-icons';
import { Highlight, HighlightItem } from '@animate-ui/react/primitives/effects/highlight';
import { Link, useSearch } from '@tanstack/react-router';
import type { ComponentProps, PointerEvent } from 'react';

import { preloadWorkspaceView } from '@/components/shell/lazy-workspaces';
import { WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import {
  DEFAULT_WORKSPACE_VIEW,
  getWorkspaceViewSearch,
  type WorkspaceView,
} from '@/lib/workspace-view';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

type PrimaryNavigationItem = {
  icon: AppIcon;
  label: string;
  shortLabel: string;
  view: WorkspaceView;
};

/**
 * Name a navigation link so that its accessible name contains the visible short label, as voice
 * control expects, while keeping the full destination name.
 *
 * @param item - Navigation item.
 * @returns Accessible name of the link.
 */
function getNavigationAccessibleName(item: PrimaryNavigationItem): string {
  return item.label.includes(item.shortLabel) ? item.label : `${item.shortLabel} ${item.label}`;
}

/** Root workspaces in navigation order. */
const PRIMARY_NAVIGATION_ITEMS: readonly PrimaryNavigationItem[] = [
  { view: 'search', icon: Search, label: '文献检索', shortLabel: '检索' },
  { view: 'favorites', icon: Star, label: '我的收藏', shortLabel: '收藏' },
  { view: 'weekly-updates', icon: CalendarDays, label: '每周更新', shortLabel: '周报' },
  { view: 'cfp-tracking', icon: Megaphone, label: '征稿追踪', shortLabel: '征稿' },
  { view: 'relations', icon: Share2, label: '文献关系图', shortLabel: '关系' },
];

/**
 * Read the current root workspace from the URL.
 *
 * @returns Current workspace view, defaulting to search.
 */
function useCurrentWorkspaceView(): WorkspaceView {
  return useSearch({
    from: WORKSPACE_ROUTE_ID,
    select: (search) => search.view ?? DEFAULT_WORKSPACE_VIEW,
  });
}

/**
 * Render one workspace link whose icon plays on hover, on touch, and when it becomes current,
 * and that reports navigation intent so the destination's code can be fetched early.
 *
 * @param props - Anchor properties, target view, whether it is current, and the intent callback.
 * @returns Navigation link that drives its animated icon.
 */
function NavigationItemLink({
  view,
  isCurrent,
  onIntent,
  onFocus,
  onPointerDown,
  onPointerEnter,
  ...props
}: Omit<ComponentProps<'a'>, 'href'> & {
  view: WorkspaceView;
  isCurrent: boolean;
  onIntent: () => void;
}) {
  const { isTriggered, triggerHandlers } = useIconTrigger<PointerEvent<HTMLAnchorElement>>(
    {
      ...props,
      onPointerDown: (event) => {
        onIntent();
        onPointerDown?.(event);
      },
      onPointerEnter: (event) => {
        onIntent();
        onPointerEnter?.(event);
      },
    },
    isCurrent,
  );
  return (
    <IconTriggerProvider isTriggered={isTriggered}>
      <Link
        {...props}
        {...triggerHandlers}
        to="/"
        search={getWorkspaceViewSearch(view)}
        activeOptions={{ exact: true }}
        onFocus={(event) => {
          onIntent();
          onFocus?.(event);
        }}
        aria-current={isCurrent ? 'page' : undefined}
      />
    </IconTriggerProvider>
  );
}

/**
 * Render the vertical workspace navigation used inside the desktop rail.
 *
 * @returns Accessible rail navigation.
 */
export function RailNavigation() {
  const view = useCurrentWorkspaceView();

  return (
    <nav aria-label="页面导航" {...stylex.props(styles.railNavigationNav)}>
      <Highlight
        controlledItems
        value={view}
        click={false}
        {...stylex.props(styles.railNavigationHighlight)}
      >
        {PRIMARY_NAVIGATION_ITEMS.map((item) => {
          const Icon = item.icon;
          const isCurrent = item.view === view;

          return (
            <HighlightItem key={item.view} value={item.view}>
              <NavigationItemLink
                key={item.view}
                view={item.view}
                aria-label={getNavigationAccessibleName(item)}
                isCurrent={isCurrent}
                onIntent={() => preloadWorkspaceView(item.view)}
                title={item.label}
                {...stylex.props(
                  styles.railNavigationNavigationItemLink,
                  isCurrent && styles.railNavigationNavigationItemLinkCurrent,
                )}
              >
                {isCurrent && (
                  <span aria-hidden="true" {...stylex.props(styles.railNavigationSpan)} />
                )}
                <Icon
                  xstyle={styles.railNavigationIcon}
                  strokeWidth={isCurrent ? 2.1 : 1.7}
                  aria-hidden="true"
                />
                <span {...stylex.props(styles.railNavigationSpan2)}>{item.shortLabel}</span>
              </NavigationItemLink>
            </HighlightItem>
          );
        })}
      </Highlight>
    </nav>
  );
}

/**
 * Render the workspace links of the mobile bottom tab bar.
 *
 * @returns Accessible tab-bar navigation links.
 */
export function TabNavigation() {
  const view = useCurrentWorkspaceView();

  return (
    <nav aria-label="页面导航" {...stylex.props(styles.tabNavigationNav)}>
      <Highlight
        controlledItems
        value={view}
        click={false}
        {...stylex.props(styles.tabNavigationHighlight)}
      >
        {PRIMARY_NAVIGATION_ITEMS.map((item) => {
          const Icon = item.icon;
          const isCurrent = item.view === view;

          return (
            <HighlightItem key={item.view} value={item.view}>
              <NavigationItemLink
                key={item.view}
                view={item.view}
                aria-label={getNavigationAccessibleName(item)}
                isCurrent={isCurrent}
                onIntent={() => preloadWorkspaceView(item.view)}
                {...stylex.props(
                  styles.tabNavigationNavigationItemLink,
                  isCurrent && styles.tabNavigationNavigationItemLinkCurrent,
                )}
              >
                <Icon
                  xstyle={styles.railNavigationIcon}
                  strokeWidth={isCurrent ? 2.1 : 1.7}
                  aria-hidden="true"
                />
                <span
                  {...stylex.props(
                    styles.tabNavigationSpan,
                    isCurrent && styles.tabNavigationSpanCurrent,
                  )}
                >
                  {item.shortLabel}
                </span>
              </NavigationItemLink>
            </HighlightItem>
          );
        })}
      </Highlight>
    </nav>
  );
}

const styles = stylex.create({
  railNavigationNav: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    gap: '0.375rem',
  },
  railNavigationHighlight: {
    inset: 0,
    borderRadius: '12px',
    backgroundColor: colors.railHover,
  },
  railNavigationNavigationItemLink: {
    position: 'relative',
    display: 'flex',
    width: '3rem',
    flexDirection: 'column',
    alignItems: 'center',
    gap: '0.25rem',
    borderRadius: '12px',
    paddingBlock: '0.5rem',
    color: {
      default: colors.railForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.railActive,
      },
    },
    transitionProperty: 'background-color, color',
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
  railNavigationNavigationItemLinkCurrent: {
    color: {
      default: colors.railActive,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.railActive,
      },
    },
  },
  railNavigationSpan: {
    position: 'absolute',
    top: 'calc(1 / 2 * 100%)',
    left: '-0.5rem',
    height: '1.5rem',
    width: '3px',
    translate: '0 calc(calc(1 / 2 * 100%) * -1)',
    borderTopRightRadius: 'calc(infinity * 1px)',
    borderBottomRightRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.primary,
  },
  railNavigationIcon: {
    width: '1.25rem',
    height: '1.25rem',
  },
  railNavigationSpan2: {
    fontSize: '10px',
    lineHeight: 1,
  },
  tabNavigationNav: {
    gridColumnStart: 'span 5',
    gridColumnEnd: 'span 5',
    display: 'grid',
    gridTemplateColumns: 'repeat(5, minmax(0, 1fr))',
  },
  tabNavigationHighlight: {
    insetInline: '0.5rem',
    insetBlock: '0.375rem',
    borderRadius: '12px',
    backgroundColor: colors.accent,
  },
  tabNavigationNavigationItemLink: {
    display: 'flex',
    height: '3.5rem',
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    gap: '0.25rem',
    color: colors.mutedForeground,
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `inset 0 0 0 2px ${colors.ring}`,
    },
  },
  tabNavigationNavigationItemLinkCurrent: {
    color: colors.primaryText,
  },
  tabNavigationSpan: {
    fontSize: '11px',
    lineHeight: 1,
  },
  tabNavigationSpanCurrent: {
    fontWeight: 700,
  },
});
