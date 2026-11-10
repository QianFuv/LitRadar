/**
 * Authenticated route guard that renders the protected application frame.
 */

import { Suspense, useEffect } from 'react';
import { useNavigate, useRouter } from '@tanstack/react-router';

import { useAuth } from '@/lib/auth-context';
import { PROTECTED_LAYOUT, preloadWorkspaceRoutes } from '@/src/routes/lazy-routes';
import { RouteLoadingFallback } from '@/src/routes/route-loading-fallback';

/**
 * Render authenticated child routes and redirect unauthenticated users to login.
 *
 * The protected frame and root workspace start downloading while the session is restored, so a
 * valid session does not wait for a second round trip. The login redirect is decided once per
 * authentication change from the location being left, so the pending navigation cannot feed its
 * own target back into the return path.
 *
 * @returns Protected route content, loading state, or login redirect.
 */
export default function RequireAuth() {
  const { user, loading } = useAuth();
  const router = useRouter();
  const navigate = useNavigate();
  const shouldPreload = loading || user !== null;
  const shouldRedirect = !loading && !user;

  useEffect(() => {
    if (shouldPreload) {
      preloadWorkspaceRoutes();
    }
  }, [shouldPreload]);

  useEffect(() => {
    if (!shouldRedirect) {
      return;
    }
    const { pathname, searchStr } = router.state.location;
    void navigate({ to: '/login', search: { next: `${pathname}${searchStr}` }, replace: true });
  }, [navigate, router, shouldRedirect]);

  if (loading) {
    return <RouteLoadingFallback />;
  }

  if (!user) {
    return null;
  }

  return (
    <Suspense fallback={<RouteLoadingFallback />}>
      <PROTECTED_LAYOUT.Component />
    </Suspense>
  );
}
