/**
 * TanStack Devtools panel with router and query inspectors, loaded only in development.
 */

import { TanStackDevtools } from '@tanstack/react-devtools';
import { ReactQueryDevtoolsPanel } from '@tanstack/react-query-devtools';
import { TanStackRouterDevtoolsPanel } from '@tanstack/react-router-devtools';

/**
 * Render the unified devtools shell for the current router and query client.
 *
 * @returns Devtools panel.
 */
export default function Devtools() {
  return (
    <TanStackDevtools
      plugins={[
        { name: 'TanStack Router', render: <TanStackRouterDevtoolsPanel /> },
        { name: 'TanStack Query', render: <ReactQueryDevtoolsPanel /> },
      ]}
    />
  );
}
