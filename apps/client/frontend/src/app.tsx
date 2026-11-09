/**
 * Root application tree: global failure boundary, router, providers, and routes.
 */

import { BrowserRouter, Route, Routes, useLocation } from 'react-router';
import type { ReactNode } from 'react';

import { ErrorBoundary } from '@/src/error-boundary';
import GlobalError from '@/src/global-error';
import Providers from '@/src/providers';
import RouteError from '@/src/route-error';
import { ServerSetupGate } from '@/src/server-setup-gate';
import Home from '@/src/routes/home-page';
import LoginPage from '@/src/routes/login-page';
import NotFound from '@/src/routes/not-found-page';
import RequireAuth from '@/src/routes/require-auth';

/**
 * Render the application route table.
 *
 * @returns Route elements for the home workspace, login, and not-found pages.
 */
export function AppRoutes() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route element={<RequireAuth />}>
        <Route index element={<Home />} />
      </Route>
      <Route path="*" element={<NotFound />} />
    </Routes>
  );
}

/**
 * Clear route failures after navigation so links in the fallback recover the page.
 *
 * @param props - Routed children protected by the boundary.
 * @returns Route-level error boundary.
 */
function RouteErrorBoundary({ children }: { children: ReactNode }) {
  const location = useLocation();
  return (
    <ErrorBoundary fallback={RouteError} resetKey={`${location.pathname}${location.search}`}>
      {children}
    </ErrorBoundary>
  );
}

/**
 * Render the complete application, behind server selection on native hosts.
 *
 * @returns Application root element.
 */
export function App() {
  return (
    <ErrorBoundary fallback={GlobalError}>
      <ServerSetupGate>
        <BrowserRouter>
          <Providers>
            <RouteErrorBoundary>
              <AppRoutes />
            </RouteErrorBoundary>
          </Providers>
        </BrowserRouter>
      </ServerSetupGate>
    </ErrorBoundary>
  );
}
