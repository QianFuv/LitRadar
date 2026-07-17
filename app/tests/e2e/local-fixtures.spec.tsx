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
 * Hide the Next.js development indicator from visual evidence screenshots.
 *
 * @param page - Playwright browser page.
 */
async function hideDevelopmentIndicator(page: Page): Promise<void> {
  await page.addStyleTag({ content: 'nextjs-portal { display: none !important; }' });
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
  if (pathname === '/api/meta/areas' || pathname === '/api/meta/journals') {
    await fulfillJson(route, []);
    return;
  }
  if (pathname === '/api/years') {
    await fulfillJson(route, []);
    return;
  }
  if (pathname === '/api/articles') {
    await fulfillJson(route, {
      items: [],
      page: { total: 0, limit: 20, offset: 0, next_cursor: null, has_more: false },
    });
    return;
  }
  if (pathname === '/api/favorites/folders') {
    await fulfillJson(route, [
      { id: 4, name: 'Tracking', is_tracking: true, article_count: 0, created_at: 1 },
    ]);
    return;
  }
  if (pathname === '/api/favorites/folders/4/articles') {
    await fulfillJson(route, []);
    return;
  }
  if (pathname === '/api/auth/invite-code') {
    await fulfillJson(route, null);
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
  await page.goto('/login?next=%2Ffavorites%3Fsettings%3Dtracking');

  await expect(page).toHaveURL(/\/favorites\?settings=tracking$/);
  await expect(page.getByRole('dialog', { name: '设置中心' })).toBeVisible();
  await expect(page.getByRole('heading', { name: '文献追踪', exact: true })).toBeVisible();
  await expect(page.getByLabel('用户名')).toHaveCount(0);
}

/**
 * Verify an unknown route renders the exported custom not-found page.
 *
 * @param page - Playwright browser page.
 */
async function showsCustomNotFoundPage(page: Page): Promise<void> {
  await page.route('**/api/**', serveBootstrapApi);
  const response = await page.goto('/missing-browser-fixture');

  expect(response?.status()).toBe(404);
  await expect(page).toHaveTitle('页面未找到 | LitRadar');
  await expect(page.getByRole('heading', { name: '页面未找到' })).toBeVisible();
  await expect(page.getByRole('link', { name: '返回首页' })).toHaveAttribute('href', '/');
}

/**
 * Verify an authenticated tracking flow can complete with local API fixtures.
 *
 * @param page - Playwright browser page.
 */
async function completesFixtureTrackingPush(page: Page): Promise<void> {
  await page.route('**/api/**', serveTrackingApi);
  await page.goto('/favorites?settings=notifications');

  await expect(page.getByRole('dialog', { name: '设置中心' })).toBeVisible();
  await expect(page.getByRole('heading', { name: '通知与推送', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '推送到追踪文件夹' }).click();
  await expect(page.getByRole('status')).toContainText('本地 fixture 推送完成');
}

/**
 * Verify desktop and mobile settings layouts, guarded history, and query preservation.
 *
 * @param page - Playwright browser page.
 */
async function verifiesAggregatedSettingsCenter(page: Page): Promise<void> {
  await page.route('**/api/**', serveTrackingApi);
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/favorites?q=graph');
  const settingsInitiator = page.getByRole('button', { name: '新建收藏夹' });
  await settingsInitiator.focus();
  await page.evaluate(() => {
    window.history.pushState(null, '', '/favorites?q=graph&settings=general');
  });

  const settingsDialog = page.getByRole('dialog', { name: '设置中心' });
  await expect(settingsDialog).toBeVisible();
  await hideDevelopmentIndicator(page);
  await expect(page.getByRole('heading', { name: '常规', exact: true })).toBeVisible();
  await expect(settingsDialog).toHaveCSS('max-width', '1152px');
  await page.screenshot({
    path: '../output/ui/settings-center-desktop.png',
    fullPage: true,
  });

  const desktopCategories = settingsDialog.locator('aside');
  await desktopCategories.getByRole('button', { name: '文献追踪' }).click();
  await expect(page).toHaveURL('/favorites?q=graph&settings=tracking');
  await page.getByRole('switch', { name: '启用推荐' }).click();

  await page.goBack();
  await expect(page.getByRole('alertdialog', { name: '放弃未保存的配置？' })).toBeVisible();
  await page.getByRole('button', { name: '继续编辑' }).click();
  await expect(page).toHaveURL('/favorites?q=graph&settings=tracking');
  await expect(page.getByRole('switch', { name: '启用推荐' })).not.toBeChecked();

  await desktopCategories.getByRole('button', { name: '账号与安全' }).click();
  await page.getByRole('button', { name: '放弃更改' }).click();
  await expect(page).toHaveURL('/favorites?q=graph&settings=account');
  await expect(page.getByRole('heading', { name: '账号与安全', exact: true })).toBeVisible();

  await settingsDialog.getByRole('button', { name: '关闭' }).click();
  await expect(page).toHaveURL('/favorites?q=graph');
  await expect(settingsDialog).toHaveCount(0);
  await expect(settingsInitiator).toBeFocused();

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/favorites?settings=general');
  const mobileDialog = page.getByRole('dialog', { name: '设置中心' });
  await expect(mobileDialog).toBeVisible();
  await hideDevelopmentIndicator(page);
  const mobileBox = await mobileDialog.boundingBox();
  expect(mobileBox).not.toBeNull();
  expect(mobileBox?.width).toBe(390);
  expect(mobileBox?.height).toBe(844);
  const mobileCategories = mobileDialog
    .locator('header')
    .getByRole('navigation', { name: '设置分类' });
  expect(
    await mobileCategories.evaluate((element) => element.scrollWidth > element.clientWidth),
  ).toBe(true);
  await page.screenshot({
    path: '../output/ui/settings-center-mobile.png',
    fullPage: true,
  });
  const mobileTokensButton = mobileCategories.getByRole('button', { name: '访问令牌' });
  await mobileTokensButton.scrollIntoViewIfNeeded();
  await expect(mobileTokensButton).toBeInViewport();
}

/**
 * Verify compact navigation, account actions, theme persistence, focus, and safe-area spacing.
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

  await page.route('**/api/**', serveTrackingApi);
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/?q=graph');
  await hideDevelopmentIndicator(page);

  const pageNavigation = page.getByRole('navigation', { name: '页面导航' });
  await expect(pageNavigation.getByRole('link')).toHaveCount(3);
  await expect(pageNavigation.getByRole('link', { name: '文献检索' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(pageNavigation.getByRole('link', { name: '我的收藏' })).toHaveAttribute(
    'title',
    '我的收藏',
  );
  await expect(pageNavigation.getByRole('link', { name: '每周更新' })).toHaveAttribute(
    'href',
    '/weekly-updates',
  );

  const trigger = page.getByRole('button', { name: '打开账号菜单：browser_user' });
  await expect(trigger).toContainText('browser_user');
  await trigger.click();
  await expect(page.getByRole('menuitem', { name: '打开设置中心' })).toHaveAttribute(
    'href',
    '/?q=graph&settings=general',
  );
  await expect(page.getByRole('menuitem', { name: '管理面板' })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: '我的收藏' })).toHaveCount(0);
  await page.screenshot({
    path: '../output/ui/navigation-account-desktop.png',
    fullPage: true,
  });

  await page.getByRole('menuitem', { name: '外观主题' }).hover();
  await page.getByRole('menuitemradio', { name: '深色' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('dark');
  await expect(page.locator('html')).toHaveClass(/dark/);

  await trigger.click();
  await page.getByRole('menuitem', { name: '外观主题' }).hover();
  await page.getByRole('menuitemradio', { name: '跟随系统' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('system');

  await trigger.click();
  await page.getByRole('menuitem', { name: '打开设置中心' }).click();
  await expect(page).toHaveURL('/?q=graph&settings=general');
  const settingsDialog = page.getByRole('dialog', { name: '设置中心' });
  await expect(settingsDialog).toBeVisible();
  await settingsDialog.getByRole('button', { name: '关闭' }).click();
  await expect(page).toHaveURL('/?q=graph');
  await expect(trigger).toBeFocused();

  await trigger.click();
  await page.mouse.click(8, 8);
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expect(trigger).toBeFocused();

  await trigger.click();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await expect(trigger).toBeFocused();

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?q=graph');
  await hideDevelopmentIndicator(page);
  await page.evaluate(() => {
    document.documentElement.style.setProperty('--safe-area-inset-bottom', '32px');
  });

  await page.getByRole('button', { name: '打开筛选器' }).click();
  const filterDialog = page.getByRole('dialog', { name: '筛选器' });
  const mobileNavigation = filterDialog.getByRole('navigation', { name: '页面导航' });
  await expect(mobileNavigation.getByRole('link')).toHaveCount(3);
  await expect(mobileNavigation.getByRole('link', { name: '文献检索' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.screenshot({ path: '../output/ui/navigation-mobile.png', fullPage: true });
  await filterDialog.getByRole('button', { name: '关闭' }).click();

  const mobileTrigger = page.getByRole('button', { name: '打开账号菜单：browser_user' });
  const resultsPaddingBottom = await page
    .locator('#results-scroll-container')
    .evaluate((element) => Number.parseFloat(window.getComputedStyle(element).paddingBottom));
  const triggerBox = await mobileTrigger.boundingBox();
  expect(resultsPaddingBottom).toBeGreaterThanOrEqual(128);
  expect(triggerBox).not.toBeNull();
  expect((triggerBox?.y ?? 844) + (triggerBox?.height ?? 0)).toBeLessThanOrEqual(796);

  const lastInteractive = page.locator('#main-content button:not([disabled])').last();
  await lastInteractive.scrollIntoViewIfNeeded();
  const lastInteractiveBox = await lastInteractive.boundingBox();
  const updatedTriggerBox = await mobileTrigger.boundingBox();
  expect(lastInteractiveBox).not.toBeNull();
  expect(updatedTriggerBox).not.toBeNull();
  const doesOverlap =
    (lastInteractiveBox?.x ?? 0) < (updatedTriggerBox?.x ?? 0) + (updatedTriggerBox?.width ?? 0) &&
    (lastInteractiveBox?.x ?? 0) + (lastInteractiveBox?.width ?? 0) > (updatedTriggerBox?.x ?? 0) &&
    (lastInteractiveBox?.y ?? 0) < (updatedTriggerBox?.y ?? 0) + (updatedTriggerBox?.height ?? 0) &&
    (lastInteractiveBox?.y ?? 0) + (lastInteractiveBox?.height ?? 0) > (updatedTriggerBox?.y ?? 0);
  expect(doesOverlap).toBe(false);

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
 * Run the custom not-found browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function customNotFoundTest({ page }: { page: Page }): Promise<void> {
  await showsCustomNotFoundPage(page);
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
 * Run the aggregated settings-center browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function aggregatedSettingsCenterTest({ page }: { page: Page }): Promise<void> {
  await verifiesAggregatedSettingsCenter(page);
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
test('renders the custom not-found page for an unknown route', customNotFoundTest);
test('completes an authenticated tracking push with local fixtures', fixtureTrackingTest);
test(
  'supports the aggregated settings center across desktop and mobile',
  aggregatedSettingsCenterTest,
);
test('supports accessible navigation and theme selection', userMenuNavigationTest);
