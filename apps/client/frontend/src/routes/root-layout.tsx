/**
 * Root route layout: the matched route plus development-only devtools.
 */

import { Outlet } from '@tanstack/react-router';
import { lazy, Suspense } from 'react';

/** Development-only TanStack Devtools, kept out of production builds and test runs. */
const DEVELOPMENT_DEVTOOLS =
  import.meta.env.DEV && import.meta.env.MODE !== 'test'
    ? lazy(() => import('@/src/devtools'))
    : null;

/**
 * Render the matched route, plus the devtools panel during development.
 *
 * @returns Root route content.
 */
export default function RootLayout() {
  return (
    <>
      <Outlet />
      {DEVELOPMENT_DEVTOOLS && (
        <Suspense fallback={null}>
          <DEVELOPMENT_DEVTOOLS />
        </Suspense>
      )}
    </>
  );
}
