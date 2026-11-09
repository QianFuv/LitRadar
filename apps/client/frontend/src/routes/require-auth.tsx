/**
 * Authenticated route guard that renders the protected application frame.
 */

import { Suspense, useEffect } from 'react';
import { Navigate, useLocation } from 'react-router';

import { useAuth } from '@/lib/auth-context';
import { PROTECTED_LAYOUT, preloadWorkspaceRoutes } from '@/src/routes/lazy-routes';
import { RouteLoadingFallback } from '@/src/routes/route-loading-fallback';

/**
 * Render authenticated child routes and redirect unauthenticated users to login.
 *
 * The protected frame and root workspace start downloading while the session is restored, so a
 * valid session does not wait for a second round trip.
 *
 * @returns Protected route content, loading state, or login redirect.
 */
export default function RequireAuth() {
  const { user, loading } = useAuth();
  const location = useLocation();
  const shouldPreload = loading || user !== null;

  useEffect(() => {
    if (shouldPreload) {
      preloadWorkspaceRoutes();
    }
  }, [shouldPreload]);

  if (loading) {
    return <RouteLoadingFallback />;
  }

  if (!user) {
    const nextPath = `${location.pathname}${location.search}`;
    return <Navigate to={`/login?next=${encodeURIComponent(nextPath)}`} replace />;
  }

  return (
    <Suspense fallback={<RouteLoadingFallback />}>
      <PROTECTED_LAYOUT.Component />
    </Suspense>
  );
}
