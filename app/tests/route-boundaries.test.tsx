/**
 * Route metadata, error boundary, global failure, and not-found coverage.
 */

import { renderToStaticMarkup } from 'react-dom/server';
import type { ReactNode } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

const routeBoundaryMocks = vi.hoisted(() => ({
  auth: {
    loading: false,
    user: null as { id: number; username: string; is_admin: boolean } | null,
  },
  pathname: '/favorites',
  replace: vi.fn(),
  searchParams: new URLSearchParams('folder=4'),
}));

vi.mock('next/navigation', () => ({
  usePathname: () => routeBoundaryMocks.pathname,
  useRouter: () => ({ replace: routeBoundaryMocks.replace }),
  useSearchParams: () => routeBoundaryMocks.searchParams,
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

import { metadata as rootMetadata } from '@/app/layout';
import { metadata as loginMetadata } from '@/app/login/page';
import RouteError from '@/app/error';
import GlobalError from '@/app/global-error';
import NotFound, { metadata as notFoundMetadata } from '@/app/not-found';
import ProtectedLayout from '@/app/(protected)/layout';

/**
 * Verify the root title template and every documented route's static metadata.
 */
function exposesRouteMetadata(): void {
  expect(rootMetadata).toMatchObject({
    title: { default: 'LitRadar | QianFuv', template: '%s | LitRadar' },
    description: '检索、收藏并追踪学术文献。',
    icons: { icon: '/litradar-logo.png' },
  });
  expect(loginMetadata).toMatchObject({
    title: '登录',
    description: '登录或注册 LitRadar 账号。',
  });
  expect(notFoundMetadata).toMatchObject({
    title: '页面未找到',
    description: '所请求的 LitRadar 页面不存在。',
  });
}

/**
 * Verify route failures are announced safely and expose a working retry action.
 */
async function resetsRouteErrors(): Promise<void> {
  const user = userEvent.setup();
  const reset = vi.fn();
  const report = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const error = Object.assign(new Error('sensitive route detail'), { digest: 'route-digest' });
  render(<RouteError error={error} reset={reset} />);

  const alert = screen.getByRole('alert');
  expect(alert).toHaveTextContent('页面加载失败');
  expect(alert).not.toHaveTextContent('sensitive route detail');
  await user.click(screen.getByRole('button', { name: '重试' }));

  expect(reset).toHaveBeenCalledOnce();
  expect(report).toHaveBeenCalledOnce();
  expect(report.mock.calls[0][0]).toMatchObject({
    component: 'browser',
    digest: 'route-digest',
    error_kind: 'error',
    event: 'client.error',
    route: '/',
    source: 'route_boundary',
  });
  expect(JSON.stringify(report.mock.calls[0][0])).not.toContain('sensitive route detail');
}

/**
 * Verify the global failure document is self-contained and does not expose error details.
 */
function rendersGlobalFailureDocument(): void {
  const error = Object.assign(new Error('sensitive global detail'), { digest: 'global-digest' });
  const markup = renderToStaticMarkup(<GlobalError error={error} reset={vi.fn()} />);

  expect(markup).toContain('<html lang="zh-CN">');
  expect(markup).toContain('<body');
  expect(markup).toContain('<title>页面错误 | LitRadar</title>');
  expect(markup).toContain('role="alert"');
  expect(markup).toContain('重新加载');
  expect(markup).toContain('返回首页');
  expect(markup).not.toContain('sensitive global detail');
}

/**
 * Verify the custom not-found page offers a direct home route.
 */
function rendersCustomNotFoundPage(): void {
  render(<NotFound />);

  expect(screen.getByRole('heading', { name: '页面未找到' })).toBeInTheDocument();
  expect(screen.getByRole('link', { name: '返回首页' })).toHaveAttribute('href', '/');
}

/**
 * Verify protected routes wait for restoration, preserve deep links, and reveal authenticated UI.
 */
async function protectsRenderedRouteContent(): Promise<void> {
  routeBoundaryMocks.auth.loading = true;
  const view = render(
    <ProtectedLayout>
      <div>protected route content</div>
    </ProtectedLayout>,
  );

  expect(screen.getByRole('status')).toHaveTextContent('加载中');
  expect(screen.queryByText('protected route content')).not.toBeInTheDocument();

  routeBoundaryMocks.auth.loading = false;
  routeBoundaryMocks.auth.user = null;
  view.rerender(
    <ProtectedLayout>
      <div>protected route content</div>
    </ProtectedLayout>,
  );
  await waitFor(() =>
    expect(routeBoundaryMocks.replace).toHaveBeenCalledWith(
      '/login?next=%2Ffavorites%3Ffolder%3D4',
    ),
  );
  expect(screen.queryByText('protected route content')).not.toBeInTheDocument();

  routeBoundaryMocks.auth.user = { id: 7, username: 'reader', is_admin: false };
  view.rerender(
    <ProtectedLayout>
      <div>protected route content</div>
    </ProtectedLayout>,
  );
  expect(screen.getByText('protected route content')).toBeInTheDocument();
  expect(screen.getByText('settings center marker')).toBeInTheDocument();
  expect(screen.getByText('admin center marker')).toBeInTheDocument();
  expect(screen.getByText('user menu marker')).toBeInTheDocument();
}

beforeEach(() => {
  routeBoundaryMocks.auth.loading = false;
  routeBoundaryMocks.auth.user = null;
  routeBoundaryMocks.pathname = '/favorites';
  routeBoundaryMocks.searchParams = new URLSearchParams('folder=4');
  routeBoundaryMocks.replace.mockReset();
});

describe('route boundaries', () => {
  test('defines specific metadata for every documented route', exposesRouteMetadata);
  test('announces route errors and resets safely', resetsRouteErrors);
  test('renders a self-contained global failure document', rendersGlobalFailureDocument);
  test('renders the custom not-found page', rendersCustomNotFoundPage);
  test('protects rendered route content and preserves deep links', protectsRenderedRouteContent);
});
