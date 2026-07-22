/**
 * Browser flows backed exclusively by Playwright route fixtures.
 */

import { expect, test, type Locator, type Page, type Route } from '@playwright/test';

type ChromeColorProperty = 'backgroundColor' | 'borderColor' | 'color';

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
 * Parse the visible RGB channels from a computed CSS color.
 *
 * @param value - Computed color in hex or rgb/rgba notation.
 * @param context - Assertion context for failures.
 * @returns Red, green, and blue channel values.
 */
function parseColorChannels(value: string, context: string): readonly [number, number, number] {
  const hexMatch = /^#([0-9a-f]{6})$/i.exec(value.trim());
  if (hexMatch) {
    return [
      Number.parseInt(hexMatch[1].slice(0, 2), 16),
      Number.parseInt(hexMatch[1].slice(2, 4), 16),
      Number.parseInt(hexMatch[1].slice(4, 6), 16),
    ];
  }

  const rgbMatch = /^rgba?\(\s*([\d.]+)(?:,\s*|\s+)([\d.]+)(?:,\s*|\s+)([\d.]+)/i.exec(
    value.trim(),
  );
  if (rgbMatch) {
    return [Number(rgbMatch[1]), Number(rgbMatch[2]), Number(rgbMatch[3])];
  }

  throw new Error(`${context} is not a supported computed color: ${value}`);
}

/**
 * Assert that one computed color has no hue.
 *
 * @param value - Computed CSS color.
 * @param context - Assertion context for failures.
 */
function expectColorToBeGrayscale(value: string, context: string): void {
  const oklabMatch = /^oklab\(\s*[\d.]+\s+(-?[\d.]+)\s+(-?[\d.]+)(?:\s*\/[^)]+)?\)$/i.exec(
    value.trim(),
  );
  if (oklabMatch) {
    expect(Math.abs(Number(oklabMatch[1]))).toBeLessThan(0.001);
    expect(Math.abs(Number(oklabMatch[2]))).toBeLessThan(0.001);
    return;
  }

  const channels = parseColorChannels(value, context);
  expect(new Set(channels).size).toBe(1);
}

/**
 * Assert selected computed chrome colors are grayscale for one element.
 *
 * @param locator - Element whose computed styles are inspected.
 * @param properties - Computed color properties to inspect.
 */
async function expectElementChromeToBeGrayscale(
  locator: Locator,
  properties: readonly ChromeColorProperty[],
): Promise<void> {
  const values = await locator.evaluate((element, colorProperties) => {
    const styles = window.getComputedStyle(element);
    return colorProperties.map((property) => styles[property]);
  }, properties);

  for (const [index, value] of values.entries()) {
    expectColorToBeGrayscale(value, properties[index]);
  }
}

/**
 * Assert global focus-ring tokens are grayscale in the active theme.
 *
 * @param page - Playwright browser page.
 */
async function expectThemeChromeTokensToBeGrayscale(page: Page): Promise<void> {
  const values = await page.locator('html').evaluate((element) => {
    const styles = window.getComputedStyle(element);
    return ['--ring', '--sidebar-ring'].map((token) => styles.getPropertyValue(token).trim());
  });

  for (const value of values) {
    expectColorToBeGrayscale(value, 'focus ring token');
  }
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
  if (pathname === '/api/weekly-updates') {
    await fulfillJson(route, {
      generated_at: '2026-07-17T09:00:00Z',
      window_start: '2026-07-10T00:00:00Z',
      window_end: '2026-07-17T23:59:59Z',
      databases: [
        {
          db_name: 'fixture.sqlite',
          generated_at: '2026-07-17T09:00:00Z',
          new_article_count: 2,
          journals: [
            {
              journal_id: 'fixture-journal',
              journal_title: 'Journal of Reproducible Literature',
              new_article_count: 2,
              articles: [
                {
                  article_id: 'weekly-fixture-1',
                  journal_id: 'fixture-journal',
                  journal_title: 'Journal of Reproducible Literature',
                  title: 'Reliable Evidence Synthesis for Living Reviews',
                  authors: ['Lin Chen', 'Maya Patel'],
                  date: '2026-07-16',
                  abstract:
                    'A fixture article demonstrating the shared weekly workspace and article detail surface.',
                },
                {
                  article_id: 'weekly-fixture-2',
                  journal_id: 'fixture-journal',
                  journal_title: 'Journal of Reproducible Literature',
                  title: 'Transparent Search Strategies in Rapid Reviews',
                  authors: ['Noah Williams', 'Rui Zhang'],
                  date: '2026-07-14',
                  abstract:
                    'A second fixture article used to verify stable ordering and responsive layout.',
                },
              ],
            },
          ],
        },
      ],
    });
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
      { id: 4, name: 'Tracking', is_tracking: true, article_count: 1, created_at: 1 },
    ]);
    return;
  }
  if (pathname === '/api/favorites/folders/4/articles') {
    await fulfillJson(route, [
      {
        id: 1,
        folder_id: 4,
        article_id: 'favorite-fixture-1',
        db_name: 'fixture.sqlite',
        note: '',
        created_at: 1,
        journal_id: 'fixture-journal',
        journal_title: 'Journal of Reproducible Literature',
        title: 'A Unified Workspace for Literature Monitoring',
        authors: ['Jia Liu', 'Alex Morgan'],
        date: '2026-07-15',
        abstract:
          'A browser fixture illustrating folder management, citation export, and shared article presentation.',
      },
    ]);
    return;
  }
  if (pathname === '/api/favorites/check/batch' && request.method() === 'POST') {
    await fulfillJson(route, [
      { article_id: 'weekly-fixture-1', folders: [{ folder_id: 4, folder_name: 'Tracking' }] },
      { article_id: 'weekly-fixture-2', folders: [] },
    ]);
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
  await page.goto('/login?next=%2F%3Fview%3Dfavorites%26settings%3Dtracking');

  await expect(page).toHaveURL(/\/\?view=favorites&settings=tracking$/);
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
  for (const missingPath of [
    '/missing-browser-fixture',
    '/favorites',
    '/weekly-updates',
  ] as const) {
    const response = await page.goto(missingPath);

    expect(response?.status()).toBe(404);
    await expect(page).toHaveTitle('页面未找到 | LitRadar');
    await expect(page.getByRole('heading', { name: '页面未找到' })).toBeVisible();
    await expect(page.getByRole('link', { name: '返回首页' })).toHaveAttribute('href', '/');
  }
}

/**
 * Verify an authenticated tracking flow can complete with local API fixtures.
 *
 * @param page - Playwright browser page.
 */
async function completesFixtureTrackingPush(page: Page): Promise<void> {
  await page.route('**/api/**', serveTrackingApi);
  await page.goto('/?view=favorites&settings=notifications');

  await expect(page.getByRole('dialog', { name: '设置中心' })).toBeVisible();
  await expect(
    page.getByLabel('通知与推送设置内容').getByRole('heading', { name: '通知与推送', exact: true }),
  ).toBeVisible();
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
  await page.goto('/?view=favorites&folder=4');
  const settingsInitiator = page.getByRole('button', { name: '新建收藏夹' });
  await settingsInitiator.focus();
  await page.evaluate(() => {
    window.history.pushState(null, '', '/?view=favorites&folder=4&settings=general');
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
  await expect(page).toHaveURL('/?view=favorites&folder=4&settings=tracking');
  await page.getByRole('switch', { name: '启用推荐' }).click();

  await page.goBack();
  await expect(page.getByRole('alertdialog', { name: '放弃未保存的配置？' })).toBeVisible();
  await page.getByRole('button', { name: '继续编辑' }).click();
  await expect(page).toHaveURL('/?view=favorites&folder=4&settings=tracking');
  await expect(page.getByRole('switch', { name: '启用推荐' })).not.toBeChecked();

  await desktopCategories.getByRole('button', { name: '账号与安全' }).click();
  await page.getByRole('button', { name: '放弃更改' }).click();
  await expect(page).toHaveURL('/?view=favorites&folder=4&settings=account');
  await expect(page.getByRole('heading', { name: '账号与安全', exact: true })).toBeVisible();

  await settingsDialog.getByRole('button', { name: '关闭' }).click();
  await expect(page).toHaveURL('/?view=favorites&folder=4');
  await expect(settingsDialog).toHaveCount(0);
  await expect(settingsInitiator).toBeFocused();

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?view=favorites&folder=4&settings=general');
  const mobileDialog = page.getByRole('dialog', { name: '设置中心' });
  await expect(mobileDialog).toBeVisible();
  await hideDevelopmentIndicator(page);
  await expect(mobileDialog).toHaveCSS('width', '390px');
  await expect(mobileDialog).toHaveCSS('height', '844px');
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
 * Verify the three root workspaces support direct links, history, canonical switches, and mobile drawers.
 *
 * @param page - Playwright browser page.
 */
async function verifiesUnifiedRootWorkspaces(page: Page): Promise<void> {
  await page.route('**/api/**', serveTrackingApi);
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/?view=favorites');
  await hideDevelopmentIndicator(page);

  const desktopNavigation = page.getByRole('navigation', { name: '页面导航' });
  await expect(page.getByRole('heading', { name: '我的收藏', exact: true })).toBeVisible();
  await expect(page.getByText('A Unified Workspace for Literature Monitoring')).toBeVisible();
  await expect(desktopNavigation.getByRole('link', { name: '我的收藏' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.screenshot({
    path: '../output/ui/workspace-favorites-desktop.png',
    fullPage: true,
  });

  await page.reload();
  await hideDevelopmentIndicator(page);
  await expect(page.getByRole('heading', { name: '我的收藏', exact: true })).toBeVisible();
  await desktopNavigation.getByRole('link', { name: '每周更新' }).click();
  await expect(page).toHaveURL('/?view=weekly-updates');
  await expect(page.getByRole('heading', { name: /期刊每周更新/ })).toBeVisible();
  await expect(page.getByText('Reliable Evidence Synthesis for Living Reviews')).toBeVisible();
  await expect(desktopNavigation.getByRole('link', { name: '每周更新' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.screenshot({
    path: '../output/ui/workspace-weekly-desktop.png',
    fullPage: true,
  });

  await page.goto('/?view=favorites&folder=4');
  await desktopNavigation.getByRole('link', { name: '每周更新' }).click();
  await expect(page).toHaveURL('/?view=weekly-updates');
  await page.goBack();
  await expect(page).toHaveURL('/?view=favorites&folder=4');
  await expect(page.getByRole('heading', { name: '我的收藏', exact: true })).toBeVisible();

  await page.goto('/?view=unsupported');
  await expect(page.getByRole('combobox', { name: '搜索文章' })).toBeVisible();
  await expect(desktopNavigation.getByRole('link', { name: '文献检索' })).toHaveAttribute(
    'aria-current',
    'page',
  );

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?view=favorites&folder=4');
  await hideDevelopmentIndicator(page);
  await page.getByRole('button', { name: '打开收藏夹' }).click();
  const favoritesDialog = page.getByRole('dialog', { name: '收藏夹' });
  const favoritesMobileNavigation = favoritesDialog.getByRole('navigation', {
    name: '页面导航',
  });
  await expect(favoritesMobileNavigation.getByRole('link', { name: '我的收藏' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.screenshot({
    path: '../output/ui/workspace-favorites-mobile.png',
    fullPage: true,
  });

  await favoritesMobileNavigation.getByRole('link', { name: '每周更新' }).click();
  await expect(page).toHaveURL('/?view=weekly-updates');
  await expect(page.getByRole('heading', { name: /期刊每周更新/ })).toBeVisible();
  await page.getByRole('button', { name: '打开期刊筛选' }).click();
  const weeklyDialog = page.getByRole('dialog', { name: '期刊筛选' });
  await expect(weeklyDialog.getByRole('link', { name: '每周更新' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.screenshot({
    path: '../output/ui/workspace-weekly-mobile.png',
    fullPage: true,
  });
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
  await expect(page.locator('html')).toHaveClass(/dark/);

  const pageNavigation = page.getByRole('navigation', { name: '页面导航' });
  const currentNavigationLink = pageNavigation.getByRole('link', { name: '文献检索' });
  await expect(pageNavigation.getByRole('link')).toHaveCount(3);
  await expect(currentNavigationLink).toHaveAttribute('aria-current', 'page');
  await expect(pageNavigation.getByRole('link', { name: '我的收藏' })).toHaveAttribute(
    'title',
    '我的收藏',
  );
  await expect(pageNavigation.getByRole('link', { name: '每周更新' })).toHaveAttribute(
    'href',
    '/?view=weekly-updates',
  );

  const trigger = page.getByRole('button', { name: '打开账号菜单：browser_user' });
  await expect(trigger).toContainText('browser_user');
  await expect(
    page.getByRole('complementary').getByRole('button', { name: '重置筛选' }),
  ).toHaveCount(0);
  await expectElementChromeToBeGrayscale(currentNavigationLink, [
    'backgroundColor',
    'borderColor',
    'color',
  ]);
  await expectElementChromeToBeGrayscale(trigger, ['backgroundColor', 'borderColor', 'color']);
  await expectThemeChromeTokensToBeGrayscale(page);
  await page.screenshot({ path: '../output/ui/default-chrome-dark.png', fullPage: true });

  await trigger.click();
  await expect(page.getByRole('menuitem', { name: '打开设置中心' })).toHaveAttribute(
    'href',
    '/?q=graph&settings=general',
  );
  await expect(page.getByRole('menuitem', { name: '管理面板' })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: '我的收藏' })).toHaveCount(0);
  await expectElementChromeToBeGrayscale(page.getByRole('menu'), [
    'backgroundColor',
    'borderColor',
    'color',
  ]);
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
  await expectElementChromeToBeGrayscale(settingsDialog, [
    'backgroundColor',
    'borderColor',
    'color',
  ]);
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

  await trigger.click();
  await page.getByRole('menuitem', { name: '外观主题' }).hover();
  await page.getByRole('menuitemradio', { name: '浅色' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('light');
  await expect(page.locator('html')).not.toHaveClass(/dark/);
  await expectElementChromeToBeGrayscale(currentNavigationLink, [
    'backgroundColor',
    'borderColor',
    'color',
  ]);
  await expectElementChromeToBeGrayscale(trigger, ['backgroundColor', 'borderColor', 'color']);
  await expectElementChromeToBeGrayscale(resetButton, ['backgroundColor', 'color']);
  await expectThemeChromeTokensToBeGrayscale(page);
  await page.screenshot({ path: '../output/ui/default-chrome-light.png', fullPage: true });

  await trigger.click();
  await page.getByRole('menuitem', { name: '外观主题' }).hover();
  await page.getByRole('menuitemradio', { name: '跟随系统' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('system');
  await expect(page.locator('html')).toHaveClass(/dark/);

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
 * Run the unified root-workspace browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function unifiedRootWorkspacesTest({ page }: { page: Page }): Promise<void> {
  await verifiesUnifiedRootWorkspaces(page);
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
test('supports three deep-linkable root workspaces', unifiedRootWorkspacesTest);
test('supports accessible navigation and theme selection', userMenuNavigationTest);
