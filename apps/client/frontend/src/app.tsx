/**
 * Root application tree: global failure boundary, server setup, providers, and router.
 */

import { RouterProvider } from '@tanstack/react-router';
import { useState } from 'react';

import { ErrorBoundary } from '@/src/error-boundary';
import GlobalError from '@/src/global-error';
import Providers from '@/src/providers';
import { createAppRouter } from '@/src/router';
import { ServerSetupGate } from '@/src/server-setup-gate';

/**
 * Render the complete application, behind server selection on native hosts.
 *
 * Route failures render the route error page inside the providers; failures in the router or
 * providers fall back to the standalone global error screen.
 *
 * @returns Application root element.
 */
export function App() {
  const [router] = useState(() => createAppRouter());
  return (
    <ErrorBoundary fallback={GlobalError}>
      <ServerSetupGate>
        <Providers>
          <RouterProvider router={router} />
        </Providers>
      </ServerSetupGate>
    </ErrorBoundary>
  );
}
