/**
 * Code-split workspace views and query-driven centers with their intent preloaders.
 */

import { Suspense, useState } from 'react';
import { lazyRouteComponent, useSearch } from '@tanstack/react-router';

import { Skeleton } from '@/components/ui/skeleton';
import { WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import type { WorkspaceView } from '@/lib/workspace-view';
import * as stylex from '@stylexjs/stylex';

/** Favorites workspace, loaded on its first visit. */
export const FAVORITES_WORKSPACE = lazyRouteComponent(
  () => import('@/components/favorites/favorites-workspace'),
  'FavoritesWorkspace',
);

/** Weekly updates workspace, loaded on its first visit. */
export const WEEKLY_UPDATES_VIEW = lazyRouteComponent(
  () => import('@/components/weekly/weekly-updates-view'),
  'WeeklyUpdatesView',
);

/** Call-for-papers workspace, loaded on its first visit. */
export const CFP_TRACKING_VIEW = lazyRouteComponent(
  () => import('@/components/cfp/cfp-tracking-view'),
  'CfpTrackingView',
);

/** Relation graph workspace with the graph canvas and force layout, loaded on its first visit. */
export const RELATIONS_WORKSPACE_VIEW = lazyRouteComponent(
  () => import('@/components/relations/relations-workspace-view'),
  'RelationsWorkspaceView',
);

const SETTINGS_CENTER_DIALOG = lazyRouteComponent(
  () => import('@/components/settings/settings-center-dialog'),
  'SettingsCenterDialog',
);

const ADMIN_CENTER_DIALOG = lazyRouteComponent(
  () => import('@/components/admin/admin-center-dialog'),
  'AdminCenterDialog',
);

/** Code-split component behind each lazily loaded workspace view. */
const LAZY_WORKSPACE_VIEWS: Readonly<
  Partial<Record<WorkspaceView, { preload?: () => Promise<void> | undefined }>>
> = {
  favorites: FAVORITES_WORKSPACE,
  'weekly-updates': WEEKLY_UPDATES_VIEW,
  'cfp-tracking': CFP_TRACKING_VIEW,
  relations: RELATIONS_WORKSPACE_VIEW,
};

/**
 * Start fetching a workspace view the user is about to open.
 *
 * @param view - Workspace view behind the hovered, focused, or touched link.
 */
export function preloadWorkspaceView(view: WorkspaceView): void {
  void LAZY_WORKSPACE_VIEWS[view]?.preload?.();
}

/** Start fetching the settings center the user is about to open. */
export function preloadSettingsCenter(): void {
  void SETTINGS_CENTER_DIALOG.preload?.();
}

/** Start fetching the administrator center the user is about to open. */
export function preloadAdminCenter(): void {
  void ADMIN_CENTER_DIALOG.preload?.();
}

/**
 * Render the workspace placeholder shown while a view's code loads.
 *
 * @returns Accessible workspace loading skeleton.
 */
export function WorkspaceViewFallback() {
  return (
    <div {...stylex.props(styles.workspaceViewFallbackDiv)}>
      <output {...stylex.props(styles.workspaceViewFallbackSpan)}>加载中…</output>
      <Skeleton aria-hidden="true" xstyle={styles.workspaceViewFallbackSkeleton} />
      <Skeleton aria-hidden="true" xstyle={styles.workspaceViewFallbackSkeleton2} />
      <Skeleton aria-hidden="true" xstyle={styles.workspaceViewFallbackSkeleton3} />
    </div>
  );
}

/**
 * Remember whether a query-driven center has been requested during this protected session.
 *
 * The center stays mounted after its first request so it keeps owning its close animation,
 * focus return, and URL cleanup exactly as an eagerly mounted center would.
 *
 * @param isRequested - Whether the current URL addresses the center.
 * @returns Whether the center module should be rendered.
 */
function useStickyRequest(isRequested: boolean): boolean {
  const [hasBeenRequested, setHasBeenRequested] = useState(isRequested);
  if (isRequested && !hasBeenRequested) {
    setHasBeenRequested(true);
  }
  return hasBeenRequested || isRequested;
}

/**
 * Render the settings center once the URL first asks for it.
 *
 * @returns Lazily loaded settings center, or nothing before its first request.
 */
export function LazySettingsCenterDialog() {
  const isRequested = useSearch({
    from: WORKSPACE_ROUTE_ID,
    select: (search) => search.settings !== undefined || search.oidc_link !== undefined,
  });
  const shouldRender = useStickyRequest(isRequested);
  if (!shouldRender) {
    return null;
  }
  return (
    <Suspense fallback={<CenterLoadingStatus label="正在打开设置中心…" />}>
      <SETTINGS_CENTER_DIALOG />
    </Suspense>
  );
}

/**
 * Render the administrator center once the URL first asks for it.
 *
 * @returns Lazily loaded administrator center, or nothing before its first request.
 */
export function LazyAdminCenterDialog() {
  const isRequested = useSearch({
    from: WORKSPACE_ROUTE_ID,
    select: (search) => search.admin !== undefined,
  });
  const shouldRender = useStickyRequest(isRequested);
  if (!shouldRender) {
    return null;
  }
  return (
    <Suspense fallback={<CenterLoadingStatus label="正在打开管理面板…" />}>
      <ADMIN_CENTER_DIALOG />
    </Suspense>
  );
}

/**
 * Announce that a center is loading without drawing a placeholder dialog.
 *
 * @param props - Screen reader announcement.
 * @returns Visually hidden live status.
 */
function CenterLoadingStatus({ label }: { label: string }) {
  return (
    <output {...stylex.props(styles.statusOutput, styles.workspaceViewFallbackSpan)}>
      {label}
    </output>
  );
}

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  workspaceViewFallbackDiv: {
    display: 'flex',
    height: '100%',
    minHeight: '0px',
    flexDirection: 'column',
    gap: '1rem',
    padding: {
      default: '1rem',
      '@media (min-width: 48rem)': '1.5rem',
    },
  },
  workspaceViewFallbackSpan: {
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
  workspaceViewFallbackSkeleton: {
    height: '2.5rem',
    width: '100%',
    maxWidth: '36rem',
    borderRadius: '12px',
  },
  workspaceViewFallbackSkeleton2: {
    height: '9rem',
    width: '100%',
    borderRadius: '16px',
  },
  workspaceViewFallbackSkeleton3: {
    height: '18rem',
    width: '100%',
    borderRadius: '16px',
  },
});
