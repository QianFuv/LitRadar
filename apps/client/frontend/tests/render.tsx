/**
 * React Query, router, and motion render utilities with deterministic test settings.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, waitFor, type RenderOptions, type RenderResult } from '@testing-library/react';
import type { UserEvent } from '@testing-library/user-event';
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
  useRouterState,
} from '@tanstack/react-router';
import { createContext, use, type ReactElement, type ReactNode } from 'react';
import { expect } from 'vite-plus/test';

import { MotionProvider } from '@/components/ui/motion';
import { RevealScope } from '@/components/ui/reveal';
import {
  LOGIN_SEARCH_SCHEMA,
  parseSearchString,
  stringifySearchObject,
  WORKSPACE_SEARCH_SCHEMA,
} from '@/lib/search-params';

/** Router created for one rendered test tree. */
export type TestRouter = ReturnType<typeof createTestRouter>;

export interface QueryRenderResult extends RenderResult {
  queryClient: QueryClient;
  router: TestRouter;
}

/** Options for rendering a component inside the test providers and router. */
export type QueryRenderOptions = Omit<RenderOptions, 'wrapper'> & {
  /** Initial in-memory location, such as `/?view=favorites&folder=4` or `/login`. */
  url?: string;
};

const TestContentContext = createContext<ReactNode>(null);

/**
 * Render the element under test at whichever route the test location matches.
 *
 * @returns Element passed to the render helper.
 */
function TestContent() {
  return use(TestContentContext);
}

/**
 * Create an in-memory router with the application's route ids, search schemas, and query
 * serialization, rendering the test element at both the workspace index and the login page.
 *
 * @param url - Initial location.
 * @returns Router over the test route tree.
 */
function createTestRouter(url = '/') {
  const rootRoute = createRootRoute();
  const loginRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: 'login',
    validateSearch: LOGIN_SEARCH_SCHEMA,
    component: TestContent,
  });
  const workspaceRoute = createRoute({
    getParentRoute: () => rootRoute,
    id: '_workspace',
    validateSearch: WORKSPACE_SEARCH_SCHEMA,
  });
  const homeRoute = createRoute({
    getParentRoute: () => workspaceRoute,
    path: '/',
    component: TestContent,
  });
  return createRouter({
    routeTree: rootRoute.addChildren([loginRoute, workspaceRoute.addChildren([homeRoute])]),
    history: createMemoryHistory({ initialEntries: [url] }),
    notFoundMode: 'root',
    parseSearch: parseSearchString,
    stringifySearch: stringifySearchObject,
  });
}

/**
 * Show the raw value of one URL search parameter for assertions.
 *
 * @param props - Parameter name and test id of the output element.
 * @returns Output element with the parameter value, or empty text when absent.
 */
export function SearchParamProbe({ parameter, testId }: { parameter: string; testId: string }) {
  const value = useRouterState({
    select: (state) => new URLSearchParams(state.location.searchStr).get(parameter),
  });
  return <output data-testid={testId}>{value ?? ''}</output>;
}

/**
 * Read the current in-memory location of a test router as a relative URL.
 *
 * @param router - Router returned by a render helper.
 * @returns Path, query, and hash of the current location.
 */
export function readRouterUrl(router: TestRouter): string {
  return router.state.location.href;
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
 * The router finishes its initial load before rendering, so route hooks are ready on the first
 * synchronous query.
 *
 * @param element - React element to render.
 * @param options - Initial location and Testing Library render options.
 * @returns Render result, the owning query client, and the router.
 */
export async function renderWithQuery(
  element: ReactElement,
  { url, ...options }: QueryRenderOptions = {},
): Promise<QueryRenderResult> {
  const queryClient = createTestQueryClient();
  const router = createTestRouter(url);
  await router.load();

  /**
   * Provide the test query client and router context to the rendered tree.
   *
   * @param props - Wrapper children.
   * @returns Router and query provider tree.
   */
  function Wrapper({ children }: { children: ReactNode }) {
    return (
      <MotionProvider>
        <QueryClientProvider client={queryClient}>
          <RevealScope>
            <TestContentContext value={children}>
              <RouterProvider router={router} />
            </TestContentContext>
          </RevealScope>
        </QueryClientProvider>
      </MotionProvider>
    );
  }

  return { ...render(element, { ...options, wrapper: Wrapper }), queryClient, router };
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
