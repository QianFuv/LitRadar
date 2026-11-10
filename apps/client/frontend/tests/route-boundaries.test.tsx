/**
 * Route metadata, error boundary, global failure, not-found, and auth guard coverage.
 */

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';
import { useState, type ReactNode } from 'react';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createMemoryHistory, RouterProvider } from '@tanstack/react-router';
import { beforeEach, describe, expect, test, vi } from 'vite-plus/test';

const routeBoundaryMocks = vi.hoisted(() => ({
  auth: {
    loading: false,
    user: null as { id: number; username: string; is_admin: boolean } | null,
  },
  isHomeFailing: false,
}));

vi.mock('@/lib/auth-context', () => ({
  AuthProvider: ({ children }: { children: ReactNode }) => children,
  useAuth: () => routeBoundaryMocks.auth,
}));

vi.mock('@/components/settings/settings-center-dialog', () => ({
  SettingsCenterDialog: () => <div>settings center marker</div>,
}));

vi.mock('@/components/admin/admin-center-dialog', () => ({
  AdminCenterDialog: () => <div>admin center marker</div>,
}));

vi.mock('@/components/feature/user-menu', () => ({
  UserMenu: () => <div>user menu marker</div>,
}));

vi.mock('@/src/routes/login-client', () => ({
  default: () => <div>login client marker</div>,
}));

vi.mock('@/src/routes/home-page', async () => {
  const { useLocation } = await import('@tanstack/react-router');
  return {
    default: function MockHomePage() {
      useLocation();
      if (routeBoundaryMocks.isHomeFailing) {
        throw new Error('sensitive home failure');
      }
      return <div>protected route content</div>;
    },
  };
});

import { ErrorBoundary, type ErrorFallbackProps } from '@/src/error-boundary';
import GlobalError from '@/src/global-error';
import RouteError from '@/src/route-error';
import { createAppRouter } from '@/src/router';
import LoginPage from '@/src/routes/login-page';
import NotFound from '@/src/routes/not-found-page';
import { readRouterUrl, renderWithQuery } from '@/tests/render';

/** Allow code-split route modules to transform and load on a busy test machine. */
const LAZY_ROUTE_WAIT = { timeout: 30_000 };

/**
 * Render the application route tree at one initial URL.
 *
 * @param initialEntry - Initial router URL.
 * @returns Application router over an in-memory history.
 */
async function renderAppRoute(initialEntry: string) {
  const router = createAppRouter(createMemoryHistory({ initialEntries: [initialEntry] }));
  await router.load();
  render(<RouterProvider router={router} />);
  return router;
}

/**
 * Verify the static document and route-level React metadata.
 */
async function exposesRouteMetadata(): Promise<void> {
  const indexHtml = readFileSync(path.resolve(process.cwd(), 'index.html'), 'utf8');
  expect(indexHtml).toContain('<title>LitRadar | QianFuv</title>');
  expect(indexHtml).toContain('<meta name="description" content="检索、收藏并追踪学术文献。" />');
  expect(indexHtml).toContain('<link rel="icon" href="/litradar-logo.png" />');
  expect(indexHtml).toContain('<script src="/theme-init.js"></script>');

  const loginView = await renderWithQuery(<LoginPage />, { url: '/login' });
  await waitFor(() => expect(document.title).toBe('登录 | LitRadar'));
  expect(
    document.head.querySelector('meta[name="description"][content="登录或注册 LitRadar 账号。"]'),
  ).not.toBeNull();
  loginView.unmount();

  await renderWithQuery(<NotFound />);
  await waitFor(() => expect(document.title).toBe('页面未找到 | LitRadar'));
  expect(
    document.head.querySelector(
      'meta[name="description"][content="所请求的 LitRadar 页面不存在。"]',
    ),
  ).not.toBeNull();
}

/**
 * Verify route failures are announced safely and expose a working retry action.
 */
async function resetsRouteErrors(): Promise<void> {
  const user = userEvent.setup();
  const reset = vi.fn();
  const report = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const error = Object.assign(new Error('sensitive route detail'), { digest: 'route-digest' });
  await renderWithQuery(<RouteError error={error} reset={reset} />);

  const alert = screen.getByRole('alert');
  expect(alert).toHaveTextContent('页面加载失败');
  expect(alert).not.toHaveTextContent('sensitive route detail');
  expect(screen.getByRole('link', { name: '返回首页' })).toHaveAttribute('href', '/');
  await user.click(screen.getByRole('button', { name: '重试' }));

  expect(reset).toHaveBeenCalledOnce();
  expect(report).toHaveBeenCalledOnce();
  expect(report.mock.calls[0][0]).toBe('[client.error] route_boundary: error');
  expect(report.mock.calls[0][1]).toMatchObject({
    component: 'browser',
    digest: 'route-digest',
    error_kind: 'error',
    event: 'client.error',
    route: '/',
    source: 'route_boundary',
  });
  expect(JSON.stringify(report.mock.calls[0])).not.toContain('sensitive route detail');
}

/**
 * Verify the global failure screen is self-contained and does not expose error details.
 */
function rendersGlobalFailureScreen(): void {
  const error = Object.assign(new Error('sensitive global detail'), { digest: 'global-digest' });
  const markup = renderToStaticMarkup(<GlobalError error={error} reset={vi.fn()} />);

  expect(markup).not.toContain('<html');
  expect(markup).toContain('<title>页面错误 | LitRadar</title>');
  expect(markup).toContain('role="alert"');
  expect(markup).toContain('重新加载');
  expect(markup).toContain('返回首页');
  expect(markup).not.toContain('sensitive global detail');
}

/**
 * Verify the custom not-found page offers a direct home route.
 */
async function rendersCustomNotFoundPage(): Promise<void> {
  routeBoundaryMocks.auth.user = { id: 7, username: 'reader', is_admin: false };
  for (const removedPath of ['/favorites', '/weekly-updates', '/settings', '/admin']) {
    await renderAppRoute(removedPath);
    expect(screen.getByRole('heading', { name: '页面未找到' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: '返回首页' })).toHaveAttribute('href', '/');
    expect(screen.queryByText('user menu marker')).not.toBeInTheDocument();
    cleanup();
  }
}

/**
 * Verify protected routes wait for restoration, preserve deep links, reveal authenticated UI,
 * and load each query-driven center only when the URL requests it.
 */
async function protectsRenderedRouteContent(): Promise<void> {
  routeBoundaryMocks.auth.loading = true;
  await renderAppRoute('/?view=favorites&folder=4');
  expect(screen.getByRole('status')).toHaveTextContent('加载中');
  expect(screen.queryByText('protected route content')).not.toBeInTheDocument();
  cleanup();

  routeBoundaryMocks.auth.loading = false;
  routeBoundaryMocks.auth.user = null;
  const anonymousRouter = await renderAppRoute('/?view=favorites&folder=4');
  await waitFor(() =>
    expect(readRouterUrl(anonymousRouter)).toBe('/login?next=%2F%3Fview%3Dfavorites%26folder%3D4'),
  );
  expect(await screen.findByText('login client marker', {}, LAZY_ROUTE_WAIT)).toBeInTheDocument();
  expect(screen.queryByText('protected route content')).not.toBeInTheDocument();
  cleanup();

  routeBoundaryMocks.auth.user = { id: 7, username: 'reader', is_admin: false };
  await renderAppRoute('/?view=favorites&folder=4');
  expect(
    await screen.findByText('protected route content', {}, LAZY_ROUTE_WAIT),
  ).toBeInTheDocument();
  expect(screen.getAllByText('user menu marker')).toHaveLength(2);
  expect(screen.queryByText('settings center marker')).not.toBeInTheDocument();
  expect(screen.queryByText('admin center marker')).not.toBeInTheDocument();
  cleanup();

  await renderAppRoute('/?settings=general');
  expect(
    await screen.findByText('settings center marker', {}, LAZY_ROUTE_WAIT),
  ).toBeInTheDocument();
  expect(screen.queryByText('admin center marker')).not.toBeInTheDocument();
  cleanup();

  await renderAppRoute('/?admin=overview');
  expect(await screen.findByText('admin center marker', {}, LAZY_ROUTE_WAIT)).toBeInTheDocument();
  expect(screen.queryByText('settings center marker')).not.toBeInTheDocument();
}

/**
 * Render a test fallback exposing the captured message and reset action.
 *
 * @param props - Captured error and reset callback.
 * @returns Fallback content.
 */
function TestFallback({ error, reset }: ErrorFallbackProps) {
  return (
    <div role="alert">
      <span>{error.message}</span>
      <button type="button" onClick={reset}>
        fallback reset
      </button>
    </div>
  );
}

/**
 * Throw while the shared flag is set so the boundary has a failure to recover from.
 *
 * @param props - Shared failure switch.
 * @returns Healthy content when the switch is off.
 */
function FailingChild({ failure }: { failure: { isActive: boolean } }) {
  if (failure.isActive) {
    throw new Error('child failure');
  }
  return <p>healthy child</p>;
}

/**
 * Verify the shared boundary renders its fallback and retries the subtree on reset.
 */
async function recoversThroughErrorBoundary(): Promise<void> {
  const user = userEvent.setup();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const failure = { isActive: true };

  /**
   * Host the boundary next to a control that re-renders it without resetting.
   *
   * @returns Boundary host.
   */
  function BoundaryHost() {
    const [renderCount, setRenderCount] = useState(0);
    return (
      <>
        <button type="button" onClick={() => setRenderCount(renderCount + 1)}>
          rerender host
        </button>
        <ErrorBoundary fallback={TestFallback}>
          <FailingChild failure={failure} />
        </ErrorBoundary>
      </>
    );
  }

  render(<BoundaryHost />);
  expect(screen.getByRole('alert')).toHaveTextContent('child failure');

  await user.click(screen.getByRole('button', { name: 'rerender host' }));
  expect(screen.getByRole('alert')).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'fallback reset' }));
  expect(screen.getByRole('alert')).toBeInTheDocument();

  failure.isActive = false;
  await user.click(screen.getByRole('button', { name: 'fallback reset' }));
  expect(screen.getByText('healthy child')).toBeInTheDocument();
  expect(screen.queryByRole('alert')).toBeNull();
}

/**
 * Verify a failing route renders the route error page, which retries on demand and clears after
 * navigation.
 */
async function recoversThroughRouteErrorPage(): Promise<void> {
  const user = userEvent.setup();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  routeBoundaryMocks.auth.user = { id: 7, username: 'reader', is_admin: false };
  routeBoundaryMocks.isHomeFailing = true;
  const router = await renderAppRoute('/?q=first');

  const alert = await screen.findByRole('alert', {}, LAZY_ROUTE_WAIT);
  expect(alert).toHaveTextContent('页面加载失败');
  expect(alert).not.toHaveTextContent('sensitive home failure');

  routeBoundaryMocks.isHomeFailing = false;
  await user.click(screen.getByRole('button', { name: '重试' }));
  expect(await screen.findByText('protected route content')).toBeInTheDocument();

  routeBoundaryMocks.isHomeFailing = true;
  await act(() => router.navigate({ to: '/', search: { q: 'second' } }));
  expect(await screen.findByRole('alert')).toHaveTextContent('页面加载失败');
  routeBoundaryMocks.isHomeFailing = false;
  await act(() => router.navigate({ to: '/', search: { q: 'third' } }));
  expect(await screen.findByText('protected route content')).toBeInTheDocument();
  expect(screen.queryByRole('alert')).toBeNull();
}

beforeEach(() => {
  routeBoundaryMocks.auth.loading = false;
  routeBoundaryMocks.auth.user = null;
  routeBoundaryMocks.isHomeFailing = false;
});

describe('route boundaries', () => {
  test('defines specific metadata for every documented route', exposesRouteMetadata);
  test('announces route errors and resets safely', resetsRouteErrors);
  test('renders a self-contained global failure screen', rendersGlobalFailureScreen);
  test('renders the custom not-found page for removed routes', rendersCustomNotFoundPage);
  test('protects rendered route content and preserves deep links', protectsRenderedRouteContent);
  test('recovers through the shared error boundary', recoversThroughErrorBoundary);
  test('recovers through the route error page', recoversThroughRouteErrorPage);
});
