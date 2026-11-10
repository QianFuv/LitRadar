/**
 * Login route shell that code-splits the login page.
 */

import { Suspense } from 'react';

import { LOGIN_PAGE } from '@/src/routes/lazy-routes';
import { RouteLoadingFallback } from '@/src/routes/route-loading-fallback';

/**
 * Render the code-split login page behind the shared loading state.
 *
 * @returns Login route content.
 */
export default function LoginRoute() {
  return (
    <Suspense fallback={<RouteLoadingFallback />}>
      <LOGIN_PAGE.Component />
    </Suspense>
  );
}
