/**
 * Code-split route modules and their intent preloaders.
 */

import { createPreloadableComponent } from '@/lib/lazy-component';

/** Public login route, loaded only when an anonymous visitor needs it. */
export const LOGIN_PAGE = createPreloadableComponent(
  async () => (await import('@/src/routes/login-page')).default,
);

/** Authenticated frame with navigation, workspace outlet, and query-driven centers. */
export const PROTECTED_LAYOUT = createPreloadableComponent(
  async () => (await import('@/src/routes/protected-layout')).default,
);

/** Root workspace route with the default search view. */
export const HOME_PAGE = createPreloadableComponent(
  async () => (await import('@/src/routes/home-page')).default,
);

/** Start fetching the authenticated frame and root workspace before they render. */
export function preloadWorkspaceRoutes(): void {
  PROTECTED_LAYOUT.preload();
  HOME_PAGE.preload();
}
