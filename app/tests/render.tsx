/**
 * React Query render utilities with deterministic retry and garbage-collection settings.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, type RenderOptions, type RenderResult } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';

export interface QueryRenderResult extends RenderResult {
  queryClient: QueryClient;
}

/**
 * Create a QueryClient suitable for deterministic component tests.
 *
 * @returns Query client with retries disabled and infinite test garbage-collection time.
 */
export function createTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Number.POSITIVE_INFINITY },
      mutations: { retry: false },
    },
  });
}

/**
 * Render a component under an isolated QueryClientProvider.
 *
 * @param element - React element to render.
 * @param options - Optional Testing Library render options.
 * @returns Render result and the owning query client.
 */
export function renderWithQuery(
  element: ReactElement,
  options?: Omit<RenderOptions, 'wrapper'>,
): QueryRenderResult {
  const queryClient = createTestQueryClient();

  /**
   * Provide the test query client to the rendered tree.
   *
   * @param props - Wrapper children.
   * @returns Query provider tree.
   */
  function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }

  return { ...render(element, { ...options, wrapper: Wrapper }), queryClient };
}
