/**
 * Code-split route modules and their intent preloaders.
 */

import { lazyRouteComponent } from '@tanstack/react-router';

/** Public login route, loaded only when an anonymous visitor needs it. */
export const LOGIN_PAGE = lazyRouteComponent(() => import('@/src/routes/login-page'));

/** Authenticated frame with navigation, workspace outlet, and query-driven centers. */
export const PROTECTED_LAYOUT = lazyRouteComponent(() => import('@/src/routes/protected-layout'));

/** Root workspace route with the default search view. */
export const HOME_PAGE = lazyRouteComponent(() => import('@/src/routes/home-page'));

/** Start fetching the authenticated frame and root workspace before they render. */
export function preloadWorkspaceRoutes(): void {
  void PROTECTED_LAYOUT.preload?.();
  void HOME_PAGE.preload?.();
}
