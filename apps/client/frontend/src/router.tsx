/**
 * Application route tree and router factory.
 *
 * Routes: the public login page, and the authenticated workspace layout whose index route
 * renders the query-routed workspaces. Unknown paths render the not-found page at the root.
 */

import {
  createRootRoute,
  createRoute,
  createRouter,
  type RouterHistory,
} from '@tanstack/react-router';

import {
  LOGIN_SEARCH_SCHEMA,
  parseSearchString,
  stringifySearchObject,
  WORKSPACE_SEARCH_SCHEMA,
} from '@/lib/search-params';
import RouteError from '@/src/route-error';
import { HOME_PAGE } from '@/src/routes/lazy-routes';
import LoginRoute from '@/src/routes/login-route';
import NotFound from '@/src/routes/not-found-page';
import RequireAuth from '@/src/routes/require-auth';
import RootLayout from '@/src/routes/root-layout';

const rootRoute = createRootRoute({
  component: RootLayout,
  errorComponent: RouteError,
  notFoundComponent: NotFound,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: 'login',
  validateSearch: LOGIN_SEARCH_SCHEMA,
  component: LoginRoute,
});

const workspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: '_workspace',
  validateSearch: WORKSPACE_SEARCH_SCHEMA,
  component: RequireAuth,
});

const homeRoute = createRoute({
  getParentRoute: () => workspaceRoute,
  path: '/',
  component: HOME_PAGE,
});

const ROUTE_TREE = rootRoute.addChildren([loginRoute, workspaceRoute.addChildren([homeRoute])]);

/**
 * Create the application router.
 *
 * @param history - Optional history, such as an in-memory history in tests.
 * @returns Router over the application route tree.
 */
export function createAppRouter(history?: RouterHistory) {
  return createRouter({
    routeTree: ROUTE_TREE,
    history,
    notFoundMode: 'root',
    parseSearch: parseSearchString,
    stringifySearch: stringifySearchObject,
  });
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}
