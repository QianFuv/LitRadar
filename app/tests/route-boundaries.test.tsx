/**
 * Route metadata, error boundary, global failure, and not-found coverage.
 */

import { renderToStaticMarkup } from 'react-dom/server';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

vi.mock('next/font/google', () => ({
  Geist: () => ({ variable: '--font-geist-sans' }),
  Geist_Mono: () => ({ variable: '--font-geist-mono' }),
}));

import { metadata as rootMetadata } from '@/app/layout';
import { metadata as loginMetadata } from '@/app/login/page';
import { metadata as adminMetadata } from '@/app/(protected)/admin/layout';
import { metadata as favoritesMetadata } from '@/app/(protected)/favorites/layout';
import { metadata as settingsMetadata } from '@/app/(protected)/settings/layout';
import { metadata as trackingMetadata } from '@/app/(protected)/tracking/layout';
import { metadata as weeklyMetadata } from '@/app/(protected)/weekly-updates/layout';
import RouteError from '@/app/error';
import GlobalError from '@/app/global-error';
import NotFound, { metadata as notFoundMetadata } from '@/app/not-found';

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
  expect(adminMetadata).toMatchObject({
    title: '管理面板',
    description: '管理 LitRadar 用户、邀请码、运行设置、计划任务和公告。',
  });
  expect(favoritesMetadata).toMatchObject({
    title: '我的收藏',
    description: '整理、移动和导出已收藏的文献。',
  });
  expect(settingsMetadata).toMatchObject({
    title: '账号设置',
    description: '管理 LitRadar 账号、安全设置、访问令牌和知网会话。',
  });
  expect(trackingMetadata).toMatchObject({
    title: '文献追踪',
    description: '配置文献推荐、通知和每周追踪推送。',
  });
  expect(weeklyMetadata).toMatchObject({
    title: '每周更新',
    description: '按数据库和期刊浏览每周新增文献。',
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
  expect(report).toHaveBeenCalledWith('LitRadar route error', {
    digest: 'route-digest',
    name: 'Error',
  });
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

describe('route boundaries', () => {
  test('defines specific metadata for every documented route', exposesRouteMetadata);
  test('announces route errors and resets safely', resetsRouteErrors);
  test('renders a self-contained global failure document', rendersGlobalFailureDocument);
  test('renders the custom not-found page', rendersCustomNotFoundPage);
});
