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

  const usernameInput = page.getByLabel('用户名');
  const passwordInput = page.getByLabel('密码', { exact: true });
  await expect(usernameInput).toBeFocused();
  await passwordInput.fill('browser-password');
  await page.getByRole('button', { name: '显示密码' }).click();
  await expect(passwordInput).toHaveAttribute('type', 'text');
  await expect(passwordInput).toHaveValue('browser-password');

  await page.getByRole('button', { name: '注册' }).last().click();

  await expect(page.getByRole('status')).toContainText('系统管理员尚未完成本机初始化');
  await expect(passwordInput).toHaveAttribute('minlength', '12');
  await expect(passwordInput).toHaveAttribute('autocomplete', 'new-password');
  await expect(page.getByLabel('邀请码')).toBeVisible();
  await expect(page.getByRole('button', { name: '注册' }).first()).toBeDisabled();
}

/**
 * Verify an authenticated login visit redirects without exposing the editable form.
 *
 * @param page - Playwright browser page.
 */
async function redirectsAuthenticatedLogin(page: Page): Promise<void> {
  await page.route('**/api/**', serveTrackingApi);
  await page.goto('/login?next=/tracking');

  await expect(page).toHaveURL(/\/tracking$/);
  await expect(page.getByRole('heading', { name: '文献追踪', exact: true })).toBeVisible();
  await expect(page.getByLabel('用户名')).toHaveCount(0);
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
 * Verify protected navigation, theme persistence, dismiss behavior, and mobile safe-area spacing.
 *
 * @param page - Playwright browser page.
 */
async function verifiesUserMenuNavigationAndTheme(page: Page): Promise<void> {
  const hydrationDiagnostics: string[] = [];

  page.on('console', (message) => {
    const text = message.text();
    if (message.type() === 'error' && /hydration|did not match|server rendered html/i.test(text)) {
      hydrationDiagnostics.push(text);
    }
  });
  page.on('pageerror', (error) => {
    if (/hydration|did not match|server rendered html/i.test(error.message)) {
      hydrationDiagnostics.push(error.message);
    }
  });

  await page.setViewportSize({ width: 360, height: 800 });
  await page.route('**/api/**', serveTrackingApi);
  await page.goto('/tracking');
  await page.evaluate(() => {
    document.documentElement.style.setProperty('--safe-area-inset-bottom', '32px');
  });

  const trigger = page.getByRole('button', { name: '打开用户菜单' });
  await expect(trigger).toBeVisible();
  await trigger.click();
  await expect(page.getByRole('menuitem', { name: '文献追踪' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(page.getByRole('menuitem', { name: '管理面板' })).toHaveCount(0);

  await page.getByRole('menuitemradio', { name: '深色' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('dark');
  await expect(page.locator('html')).toHaveClass(/dark/);

  await trigger.click();
  await page.getByRole('menuitemradio', { name: '跟随系统' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('system');

  await trigger.click();
  await page.mouse.click(8, 8);
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expect(trigger).toBeFocused();

  await trigger.click();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expect(trigger).toBeFocused();

  const mainPaddingBottom = await page
    .locator('#main-content')
    .evaluate((element) => Number.parseFloat(window.getComputedStyle(element).paddingBottom));
  const triggerBox = await trigger.boundingBox();
  expect(mainPaddingBottom).toBeGreaterThanOrEqual(128);
  expect(triggerBox).not.toBeNull();
  expect(triggerBox?.y ?? 800).toBeLessThan(752);

  const lastInteractive = page.locator('#main-content button:not([disabled])').last();
  await lastInteractive.scrollIntoViewIfNeeded();
  const lastInteractiveBox = await lastInteractive.boundingBox();
  const updatedTriggerBox = await trigger.boundingBox();
  expect(lastInteractiveBox).not.toBeNull();
  expect(updatedTriggerBox).not.toBeNull();
  const doesOverlap =
    (lastInteractiveBox?.x ?? 0) < (updatedTriggerBox?.x ?? 0) + (updatedTriggerBox?.width ?? 0) &&
    (lastInteractiveBox?.x ?? 0) + (lastInteractiveBox?.width ?? 0) > (updatedTriggerBox?.x ?? 0) &&
    (lastInteractiveBox?.y ?? 0) < (updatedTriggerBox?.y ?? 0) + (updatedTriggerBox?.height ?? 0) &&
    (lastInteractiveBox?.y ?? 0) + (lastInteractiveBox?.height ?? 0) > (updatedTriggerBox?.y ?? 0);
  expect(doesOverlap).toBe(false);

  await trigger.click();
  await page.getByRole('menuitem', { name: '账号设置' }).click();
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.getByRole('heading', { name: '账号设置' })).toBeVisible();
  expect(hydrationDiagnostics).toEqual([]);
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
 * Run the authenticated login redirect browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function authenticatedLoginRedirectTest({ page }: { page: Page }): Promise<void> {
  await redirectsAuthenticatedLogin(page);
}

/**
 * Run the authenticated tracking browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function fixtureTrackingTest({ page }: { page: Page }): Promise<void> {
  await completesFixtureTrackingPush(page);
}

/**
 * Run the authenticated user-menu browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function userMenuNavigationTest({ page }: { page: Page }): Promise<void> {
  await verifiesUserMenuNavigationAndTheme(page);
}

test('shows the local administrator bootstrap boundary', bootstrapBoundaryTest);
test(
  'redirects an authenticated login visit without showing the form',
  authenticatedLoginRedirectTest,
);
test('completes an authenticated tracking push with local fixtures', fixtureTrackingTest);
test('supports accessible navigation and theme selection', userMenuNavigationTest);
