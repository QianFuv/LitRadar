/**
 * React Query, router, and motion render utilities with deterministic test settings.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, waitFor, type RenderOptions, type RenderResult } from '@testing-library/react';
import type { UserEvent } from '@testing-library/user-event';
import type { ReactElement, ReactNode } from 'react';
import { MemoryRouter } from 'react-router';
import { expect } from 'vite-plus/test';

import { MotionProvider } from '@/components/ui/motion';
import { RevealScope } from '@/components/ui/reveal';

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
 * Render a component under an isolated QueryClientProvider and an in-memory router.
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
   * Provide the test query client and router context to the rendered tree.
   *
   * @param props - Wrapper children.
   * @returns Router and query provider tree.
   */
  function Wrapper({ children }: { children: ReactNode }) {
    return (
      <MemoryRouter>
        <MotionProvider>
          <QueryClientProvider client={queryClient}>
            <RevealScope>{children}</RevealScope>
          </QueryClientProvider>
        </MotionProvider>
      </MemoryRouter>
    );
  }

  return { ...render(element, { ...options, wrapper: Wrapper }), queryClient };
}

/**
 * Open the focused Select from the keyboard and wait until its checked option holds focus, which
 * happens once the list has positioned itself below the trigger.
 *
 * @param user - User event session driving the keyboard.
 */
export async function openSelectFromKeyboard(user: UserEvent): Promise<void> {
  await user.keyboard('{ArrowDown}');
  await waitFor(() => expect(document.activeElement).toHaveAttribute('role', 'option'));
}
