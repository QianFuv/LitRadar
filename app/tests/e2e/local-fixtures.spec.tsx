/**
 * Browser flows backed exclusively by Playwright route fixtures.
 */

import { expect, test, type Page, type Route } from '@playwright/test';

/**
 * Fulfill one route with a JSON response.
 *
 * @param route - Intercepted browser route.
 * @param payload - JSON-serializable response body.
 * @param status - HTTP status code.
 */
async function fulfillJson(route: Route, payload: unknown, status = 200): Promise<void> {
  await route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(payload),
  });
}

/**
 * Serve unauthenticated bootstrap-state API fixtures.
 *
 * @param route - Intercepted API route.
 */
async function serveBootstrapApi(route: Route): Promise<void> {
  const pathname = new URL(route.request().url()).pathname;
  if (pathname === '/api/auth/me') {
    await fulfillJson(route, { detail: 'Authentication required' }, 401);
    return;
  }
  if (pathname === '/api/auth/invite-required') {
    await fulfillJson(route, { required: true, bootstrap_required: true });
    return;
  }
  await fulfillJson(route, { detail: `Unhandled fixture route: ${pathname}` }, 404);
}

/**
 * Serve authenticated tracking-page API fixtures.
 *
 * @param route - Intercepted API route.
 */
async function serveTrackingApi(route: Route): Promise<void> {
  const request = route.request();
  const pathname = new URL(request.url()).pathname;
  if (pathname === '/api/auth/me') {
    await fulfillJson(route, { id: 41, username: 'browser_user', is_admin: false });
    return;
  }
  if (pathname === '/api/tracking/status') {
    await fulfillJson(route, {
      tracking_folder: { id: 4, name: 'Tracking' },
      total_folders: 1,
      weekly_articles_available: 2,
      notification_configured: false,
    });
    return;
  }
  if (pathname === '/api/meta/databases') {
    await fulfillJson(route, ['fixture.sqlite']);
    return;
  }
  if (pathname === '/api/favorites/folders') {
    await fulfillJson(route, [
      { id: 4, name: 'Tracking', is_tracking: true, article_count: 0, created_at: 1 },
    ]);
    return;
  }
  if (pathname === '/api/tracking/notification-settings') {
    await fulfillJson(route, null);
    return;
  }
  if (pathname === '/api/tracking/push-weekly' && request.method() === 'POST') {
    await fulfillJson(route, {
      job_id: 'browser-job',
      status: 'completed',
      message: '本地 fixture 推送完成',
      started_at: 1,
      finished_at: 2,
      pushed: 2,
      selected: 2,
      total_candidates: 2,
      summary: 'fixture summary',
      folder_id: 4,
      folder_name: 'Tracking',
    });
    return;
  }
  await fulfillJson(route, { detail: `Unhandled fixture route: ${pathname}` }, 404);
}

/**
 * Verify an uninitialized deployment disables public registration.
 *
 * @param page - Playwright browser page.
 */
async function showsBootstrapBoundary(page: Page): Promise<void> {
  await page.route('**/api/**', serveBootstrapApi);
  await page.goto('/login');
  await page.getByRole('button', { name: '注册' }).last().click();

  await expect(page.getByRole('status')).toContainText('系统管理员尚未完成本机初始化');
  await expect(page.getByLabel('密码')).toHaveAttribute('minlength', '12');
  await expect(page.getByLabel('邀请码')).toBeVisible();
  await expect(page.getByRole('button', { name: '注册' }).first()).toBeDisabled();
}

/**
 * Verify an authenticated tracking flow can complete with local API fixtures.
 *
 * @param page - Playwright browser page.
 */
async function completesFixtureTrackingPush(page: Page): Promise<void> {
  await page.route('**/api/**', serveTrackingApi);
  await page.goto('/tracking');

  await expect(page.getByRole('heading', { name: '文献追踪', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '推送到追踪文件夹' }).click();
  await expect(page.getByRole('status')).toContainText('本地 fixture 推送完成');
}

/**
 * Run the bootstrap-boundary browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function bootstrapBoundaryTest({ page }: { page: Page }): Promise<void> {
  await showsBootstrapBoundary(page);
}

/**
 * Run the authenticated tracking browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function fixtureTrackingTest({ page }: { page: Page }): Promise<void> {
  await completesFixtureTrackingPush(page);
}

test('shows the local administrator bootstrap boundary', bootstrapBoundaryTest);
test('completes an authenticated tracking push with local fixtures', fixtureTrackingTest);
