/**
 * Browser flows backed exclusively by Playwright route fixtures.
 */

import { expect, test, type Browser, type Locator, type Page, type Route } from '@playwright/test';

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
 * Verify exact totals stay omitted while the filtered summary remains sticky.
 *
 * @param page - Playwright browser page.
 */
async function expectActiveFilterSummaryToStick(page: Page): Promise<void> {
  const resultCount = page.getByText(/共找到 \d+ 条结果/);
  const filterSummary = page.getByRole('region', { name: '已应用筛选' });
  const filterSummarySlot = page.getByTestId('filter-summary-slot');
  const scrollContainer = page.locator('#results-scroll-container');

  await expect(resultCount).toHaveCount(0);
  await expect(filterSummary).toBeVisible();
  await expect(filterSummarySlot).toHaveCSS('position', 'sticky');

  await scrollContainer.evaluate((element) => {
    element.scrollTop = 400;
  });
  const pinnedTop = Math.round((await filterSummarySlot.boundingBox())?.y ?? -1);
  expect(pinnedTop).toBeGreaterThan(0);

  await scrollContainer.evaluate((element) => {
    element.scrollTop = 700;
  });
  await expect
    .poll(async () => Math.round((await filterSummarySlot.boundingBox())?.y ?? -1))
    .toBe(pinnedTop);
  await expect(resultCount).toHaveCount(0);
  await page.screenshot({ path: '../output/ui/active-filter-sticky.png', fullPage: true });

  await scrollContainer.evaluate((element) => {
    element.scrollTop = 0;
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
  const requestUrl = new URL(request.url());
  const pathname = requestUrl.pathname;
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
  if (pathname === '/api/weekly-updates/summary') {
    await fulfillJson(route, {
      generated_at: '2026-07-17T09:00:00Z',
      window_start: '2026-07-10T00:00:00Z',
      window_end: '2026-07-17T23:59:59Z',
      databases: [
        {
          db_name: 'fixture.sqlite',
          run_id: 'weekly-fixture-run',
          generated_at: '2026-07-17T09:00:00Z',
          new_article_count: 2,
          journals: [
            {
              journal_id: 'fixture-journal',
              journal_title: 'Journal of Reproducible Literature',
              new_article_count: 2,
            },
          ],
        },
      ],
    });
    return;
  }
  if (pathname === '/api/weekly-updates/articles') {
    await fulfillJson(route, {
      items: [
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
      page: {
        total: null,
        limit: 50,
        offset: 0,
        next_cursor: null,
        has_more: false,
      },
    });
    return;
  }
  if (pathname === '/api/articles') {
    const items = requestUrl.searchParams.has('q')
      ? Array.from({ length: 30 }, (_, index) => ({
          article_id: `search-fixture-${index + 1}`,
          journal_id: 'fixture-journal',
          journal_title: 'Journal of Reproducible Literature',
          title: `Graph evidence fixture ${index + 1}`,
          authors: ['Browser Fixture'],
          date: '2026-07-15',
          abstract: `Graph result ${index + 1} provides enough content for sticky result scrolling.`,
        }))
      : [];
    await fulfillJson(route, {
      items,
      page: {
        total: items.length,
        limit: 50,
        offset: 0,
        next_cursor: null,
        has_more: false,
      },
    });
    return;
  }
  if (pathname === '/api/favorites/folders') {
    await fulfillJson(route, [
      { id: 4, name: 'Tracking', is_tracking: true, article_count: 1, created_at: 1 },
    ]);
    return;
  }
  if (pathname === '/api/favorites/folders/4/articles/page') {
    await fulfillJson(route, {
      items: [
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
      ],
      page: { total: null, limit: 50, offset: 0, next_cursor: null, has_more: false },
    });
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
  if (pathname === '/api/tracking/ai-endpoints') {
    await fulfillJson(route, []);
    return;
  }
  if (pathname === '/api/tracking/push-weekly/status' && request.method() === 'GET') {
    await fulfillJson(route, {
      job_id: null,
      status: 'idle',
      message: 'No manual push task is available',
      started_at: null,
      finished_at: null,
      deadline_at: null,
      cancellation_requested: false,
      can_cancel: false,
      can_retry: false,
      pushed: 0,
      selected: 0,
      total_candidates: null,
      summary: '',
      folder_id: null,
      folder_name: null,
    });
    return;
  }
  if (pathname === '/api/tracking/push-weekly' && request.method() === 'POST') {
    await fulfillJson(route, {
      job_id: 'browser-job',
      status: 'completed',
      message: '本地 fixture 推送完成',
      started_at: 1,
      finished_at: 2,
      deadline_at: 600,
      cancellation_requested: false,
      can_cancel: false,
      can_retry: true,
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
 * Serve an authenticated administrator together with the existing workspace fixtures.
 *
 * @param route - Intercepted API route.
 */
async function serveAdministratorApi(route: Route): Promise<void> {
  const pathname = new URL(route.request().url()).pathname;
  if (pathname === '/api/auth/me') {
    await fulfillJson(route, { id: 42, username: 'browser_admin', is_admin: true });
    return;
  }
  if (pathname === '/api/admin/stats') {
    await fulfillJson(route, {
      auth: {
        total_users: 2,
        admin_count: 1,
        total_folders: 1,
        total_favorites: 1,
        total_invite_codes: 1,
        used_invite_codes: 0,
        unused_invite_codes: 1,
        active_tokens: 0,
        notification_subscribers: 0,
        scheduled_tasks: 0,
        active_announcements: 0,
      },
      index: {
        databases: [],
        total_articles: 30,
        total_journals: 1,
      },
      push: [],
    });
    return;
  }
  if (pathname === '/api/admin/scheduled-tasks') {
    await fulfillJson(route, [
      {
        id: 8,
        name: 'Weekly index',
        job: { kind: 'index', notify: false, push: false },
        legacy_command: null,
        cron: '0 8 * * *',
        timezone: 'UTC',
        timeout_seconds: 3600,
        coalesce: true,
        enabled: true,
        last_run_at: null,
        last_status: 'idle',
        created_at: 1,
        updated_at: 2,
      },
    ]);
    return;
  }
  if (
    pathname === '/api/admin/users' ||
    pathname === '/api/admin/invite-codes' ||
    pathname === '/api/admin/runtime-settings' ||
    pathname === '/api/admin/announcements'
  ) {
    await fulfillJson(route, []);
    return;
  }
  if (pathname === '/api/admin/provider-catalog') {
    await fulfillJson(route, { catalogs: [], providers: [] });
    return;
  }
  if (pathname === '/api/admin/scheduler/status') {
    await fulfillJson(route, {
      last_checked_at: 1_700_000_000,
      recent_runs: [
        {
          id: 12,
          task_id: 8,
          task_name: 'Weekly index',
          scheduled_for: 1_699_999_800,
          status: 'success',
          worker_id: 'worker-fixture',
          claimed_at: 1_699_999_801,
          started_at: 1_699_999_802,
          finished_at: 1_699_999_803,
        },
      ],
      workers: [
        {
          worker_id: 'worker-fixture',
          started_at: 1_699_999_000,
          heartbeat_at: 1_700_000_000,
          is_healthy: true,
        },
      ],
    });
    return;
  }
  await serveTrackingApi(route);
}

/**
 * Verify an uninitialized deployment disables public registration.
 *
 * @param page - Playwright browser page.
 */
async function showsBootstrapBoundary(page: Page): Promise<void> {
  await page.route('**/api/**', serveBootstrapApi);
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/login');
  await hideDevelopmentIndicator(page);

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
  await expect(page.locator('[data-auth-header-mode="register"]')).toBeVisible();
  await page.screenshot({ path: '../output/ui/login-desktop.png', fullPage: true });

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator('[data-auth-state="form"]')).toBeVisible();
  expect(
    await page
      .locator('#main-content')
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: '../output/ui/login-mobile.png', fullPage: true });
}

/**
 * Verify an authenticated login visit redirects without exposing the editable form.
 *
 * @param page - Playwright browser page.
 */
async function redirectsAuthenticatedLogin(page: Page): Promise<void> {
  const maliciousRequests: string[] = [];
  page.on('request', (request) => {
    if (new URL(request.url()).hostname === 'malicious.example') {
      maliciousRequests.push(request.url());
    }
  });
  await page.route('**/api/**', serveTrackingApi);
  await page.goto('/login?next=%2F%5Cmalicious.example');

  await expect(page).toHaveURL(/\/$/);
  expect(new URL(page.url()).hostname).not.toBe('malicious.example');
  expect(maliciousRequests).toEqual([]);

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
  await expect(page.locator('[data-motion-feedback="manual-push"]')).toContainText(
    '本地 fixture 推送完成',
  );
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
  await expect(settingsDialog.locator('[data-motion-section-header="general"]')).toBeVisible();
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
  const navigationInsets = await mobileCategories.evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    return { left: Math.round(bounds.left), right: Math.round(innerWidth - bounds.right) };
  });
  expect(navigationInsets.right).toBe(navigationInsets.left);
  await expect(mobileDialog.locator('[data-mobile-overflow-cue="true"]')).toBeVisible();
  await expect(mobileCategories.getByRole('button', { name: '常规' })).toHaveAttribute(
    'data-section-active',
    'true',
  );
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
 * Verify reduced-motion delivery toggles remain immediate, non-focusable, and state preserving.
 *
 * @param page - Playwright browser page.
 */
async function verifiesReducedMotionDynamicSettings(page: Page): Promise<void> {
  await page.route('**/api/**', serveTrackingApi);
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/?view=favorites&folder=4&settings=notifications');
  await hideDevelopmentIndicator(page);

  const settingsDialog = page.getByRole('dialog', { name: '设置中心' });
  const deliverySelect = settingsDialog.getByRole('combobox', { name: '推送方式' });
  const deliveryPanel = settingsDialog.locator('[data-motion-delivery-panel="pushplus"]');
  await expect(deliveryPanel).toHaveAttribute('inert', '');
  await expect(deliveryPanel).toHaveAttribute('aria-hidden', 'true');

  await deliverySelect.click();
  await page.getByRole('option', { name: 'PushPlus 外部推送' }).click();
  await expect(deliveryPanel).not.toHaveAttribute('inert');
  const pushplusToken = settingsDialog.getByLabel('PushPlus 令牌');
  await pushplusToken.fill('browser-pushplus-token');

  await deliverySelect.click();
  await page.getByRole('option', { name: '追踪文件夹推送' }).click();
  await expect(deliveryPanel).toHaveAttribute('inert', '');
  await pushplusToken.evaluate((element) => (element as HTMLInputElement).focus());
  await expect(pushplusToken).not.toBeFocused();
  await expect(pushplusToken).toHaveValue('browser-pushplus-token');

  await deliverySelect.click();
  await page.getByRole('option', { name: 'PushPlus 外部推送' }).click();
  await expect(deliveryPanel).not.toHaveAttribute('inert');
  await expect(pushplusToken).toHaveValue('browser-pushplus-token');
  const animationDurations = await deliveryPanel.evaluate((element) =>
    element.getAnimations({ subtree: true }).flatMap((animation) => {
      const duration = animation.effect?.getTiming().duration;
      return typeof duration === 'number' ? [duration] : [];
    }),
  );
  expect(Math.max(0, ...animationDurations)).toBeLessThanOrEqual(1);
  expect(
    await settingsDialog.evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: '../output/ui/settings-dynamic-dark-desktop.png', fullPage: true });

  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(settingsDialog).toHaveCSS('width', '390px');
  expect(
    await settingsDialog.evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: '../output/ui/settings-dynamic-light-mobile.png', fullPage: true });
}

/**
 * Verify administrator menu entry, responsive center, query preservation, and focus return.
 *
 * @param page - Playwright browser page.
 */
async function verifiesAdministratorCenter(page: Page): Promise<void> {
  await page.route('**/api/**', serveAdministratorApi);
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/?q=graph');
  await hideDevelopmentIndicator(page);

  const accountTrigger = page.getByRole('button', {
    name: '打开账号菜单：browser_admin',
  });
  await accountTrigger.click();
  const adminEntry = page.getByRole('menuitem', { name: '管理面板' });
  await expect(adminEntry).toHaveAttribute('href', '/?q=graph&admin=overview');
  await adminEntry.click();

  await expect(page).toHaveURL('/?q=graph&admin=overview');
  const adminDialog = page.getByRole('dialog', { name: '管理面板' });
  await expect(adminDialog).toBeVisible();
  await expectConsistentDialogClose(adminDialog, 40);
  await expect(adminDialog).toHaveCSS('max-width', '1152px');
  await expect(page.getByRole('heading', { name: '概览', exact: true })).toBeVisible();
  await expect(adminDialog.locator('[data-motion-section-header="overview"]')).toBeVisible();
  const desktopCategories = adminDialog.locator('aside').getByRole('navigation', {
    name: '管理分类',
  });
  await expect(desktopCategories.getByRole('button')).toHaveCount(6);
  await page.screenshot({ path: '../output/ui/admin-center-desktop.png', fullPage: true });

  await desktopCategories.getByRole('button', { name: '用户' }).click();
  await expect(page).toHaveURL('/?q=graph&admin=users');
  await expect(page.getByRole('heading', { name: '用户', exact: true })).toBeVisible();

  await desktopCategories.getByRole('button', { name: '计划任务' }).click();
  await expect(page).toHaveURL('/?q=graph&admin=scheduled-tasks');
  await expect(adminDialog.locator('[data-motion-scheduled-task-key="8"]')).toBeVisible();
  await expect(adminDialog.locator('[data-motion-scheduler-state="workers-1-1"]')).toBeVisible();
  await adminDialog.getByRole('button', { name: '新建任务' }).click();
  const taskDialog = page.getByRole('dialog', { name: '新建定时任务' });
  await expectConsistentDialogClose(taskDialog, 40);
  const indexFields = taskDialog.locator('[data-motion-scheduled-fields="index"]');
  const deliveryFields = taskDialog.locator('[data-motion-scheduled-fields="delivery"]');
  await expect(indexFields).not.toHaveAttribute('inert');
  await expect(deliveryFields).toHaveAttribute('inert', '');
  await taskDialog.getByLabel('元数据 CSV 文件名（可选）').fill('journals.csv');
  const presetSelect = taskDialog.getByRole('combobox', { name: '任务预设' });
  await presetSelect.click();
  await page.getByRole('option', { name: '仅文件夹推送' }).click();
  await expect(indexFields).toHaveAttribute('inert', '');
  await expect(deliveryFields).not.toHaveAttribute('inert');
  await taskDialog.getByLabel('索引数据库（可选）').fill('journals.sqlite');
  await presetSelect.click();
  await page.getByRole('option', { name: '索引更新', exact: true }).click();
  await expect(deliveryFields).toHaveAttribute('inert', '');
  await expect(taskDialog.getByLabel('索引数据库（可选）')).toHaveValue('journals.sqlite');
  await expect(taskDialog.getByLabel('元数据 CSV 文件名（可选）')).toHaveValue('journals.csv');
  await taskDialog.getByRole('button', { name: '取消' }).click();
  await expect(taskDialog).toHaveCount(0);
  await page.screenshot({ path: '../output/ui/admin-scheduled-desktop.png', fullPage: true });
  await adminDialog.getByRole('button', { name: '关闭' }).click();
  await expect(page).toHaveURL('/?q=graph');
  await expect(adminDialog).toHaveCount(0);
  await expect(accountTrigger).toBeFocused();

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?q=graph&admin=overview');
  const mobileDialog = page.getByRole('dialog', { name: '管理面板' });
  await expect(mobileDialog).toBeVisible();
  await expectConsistentDialogClose(mobileDialog, 44);
  await hideDevelopmentIndicator(page);
  await expect(mobileDialog).toHaveCSS('width', '390px');
  await expect(mobileDialog).toHaveCSS('height', '844px');
  const mobileCategories = mobileDialog
    .locator('header')
    .getByRole('navigation', { name: '管理分类' });
  const navigationInsets = await mobileCategories.evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    return { left: Math.round(bounds.left), right: Math.round(innerWidth - bounds.right) };
  });
  expect(navigationInsets.right).toBe(navigationInsets.left);
  await expect(mobileDialog.locator('[data-mobile-overflow-cue="true"]')).toBeVisible();
  await expect(mobileCategories.getByRole('button', { name: '概览' })).toHaveAttribute(
    'data-section-active',
    'true',
  );
  expect(
    await mobileCategories.evaluate((element) => element.scrollWidth > element.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: '../output/ui/admin-center-mobile.png', fullPage: true });
  const mobileScheduledButton = mobileCategories.getByRole('button', { name: '计划任务' });
  await mobileScheduledButton.scrollIntoViewIfNeeded();
  await mobileScheduledButton.click();
  await expect(mobileDialog.locator('[data-motion-scheduled-task-key="8"]')).toBeVisible();
  expect(await mobileDialog.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
    true,
  );
  await page.screenshot({ path: '../output/ui/admin-scheduled-mobile.png', fullPage: true });
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
  await expectActiveFilterSummaryToStick(page);

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

  await expect(
    page.getByRole('group', { name: '外观主题' }).getByRole('menuitemradio'),
  ).toHaveCount(3);
  await expect(page.getByRole('menu')).toHaveCount(1);
  await page.getByRole('menuitemradio', { name: '深色' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('dark');
  await expect(page.locator('html')).toHaveClass(/dark/);

  await trigger.click();
  await expect(page.getByRole('menuitemradio', { name: '深色' })).toHaveAttribute(
    'aria-checked',
    'true',
  );
  await page.getByRole('menuitemradio', { name: '跟随系统' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('system');

  await trigger.click();
  await page.getByRole('menuitem', { name: '打开设置中心' }).click();
  await expect(page).toHaveURL('/?q=graph&settings=general');
  const settingsDialog = page.getByRole('dialog', { name: '设置中心' });
  await expect(settingsDialog).toBeVisible();
  await expectConsistentDialogClose(settingsDialog, 40);
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
  await page.getByRole('menuitemradio', { name: '浅色' }).click();
  await expect.poll(() => page.evaluate(() => window.localStorage.getItem('theme'))).toBe('light');
  await expect(page.locator('html')).not.toHaveClass(/dark/);
  await expectElementChromeToBeGrayscale(currentNavigationLink, [
    'backgroundColor',
    'borderColor',
    'color',
  ]);
  await expectElementChromeToBeGrayscale(trigger, ['backgroundColor', 'borderColor', 'color']);
  await expectThemeChromeTokensToBeGrayscale(page);
  await page.screenshot({ path: '../output/ui/default-chrome-light.png', fullPage: true });

  await trigger.click();
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
  await expect(filterDialog.getByRole('button', { name: '关闭' })).toHaveCount(0);
  const mobileNavigation = filterDialog.getByRole('navigation', { name: '页面导航' });
  await expect(mobileNavigation.getByRole('link')).toHaveCount(3);
  await expect(mobileNavigation.getByRole('link', { name: '文献检索' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await page.screenshot({ path: '../output/ui/navigation-mobile.png', fullPage: true });
  await page.mouse.click(382, 400);
  await expect(filterDialog).toHaveCount(0);

  const firstArticleAction = page.getByRole('button', { name: /^查看文章详情：/ }).first();
  const firstArticleCard = firstArticleAction.locator('[data-slot="card"]');
  const firstArticleTitle = firstArticleCard.locator('[data-slot="card-title"]');
  await expect(firstArticleCard).toBeVisible();
  await expect(firstArticleTitle).toBeVisible();
  await expect(firstArticleAction).toBeVisible();
  const articleCardBox = await firstArticleCard.boundingBox();
  const articleTitleBox = await firstArticleTitle.boundingBox();
  const articleActionBox = await firstArticleAction.boundingBox();
  expect(articleCardBox).not.toBeNull();
  expect(articleTitleBox).not.toBeNull();
  expect(articleActionBox).not.toBeNull();
  await expect(firstArticleCard.locator('[data-slot="card-footer"]')).toHaveCount(0);
  expect(articleActionBox?.width).toBe(articleCardBox?.width);
  expect(articleActionBox?.height).toBe(articleCardBox?.height);
  expect(articleCardBox?.x ?? -1).toBeGreaterThanOrEqual(0);
  expect((articleCardBox?.x ?? 390) + (articleCardBox?.width ?? 1)).toBeLessThanOrEqual(390);
  expect((articleTitleBox?.y ?? 844) + (articleTitleBox?.height ?? 1)).toBeLessThanOrEqual(
    (articleCardBox?.y ?? 0) + (articleCardBox?.height ?? 0),
  );
  await page.screenshot({ path: '../output/ui/search-results-mobile.png', fullPage: true });

  const mobileTrigger = page.getByRole('button', { name: '打开账号菜单：browser_user' });
  const resultsPaddingBottom = await page
    .locator('#results-scroll-container')
    .evaluate((element) => Number.parseFloat(window.getComputedStyle(element).paddingBottom));
  const triggerBox = await mobileTrigger.boundingBox();
  expect(resultsPaddingBottom).toBeGreaterThanOrEqual(128);
  expect(triggerBox).not.toBeNull();
  expect((triggerBox?.y ?? 844) + (triggerBox?.height ?? 0)).toBeLessThanOrEqual(796);

  const lastInteractive = page
    .locator('#main-content :is(button:not([disabled]), [role="button"])')
    .last();
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

  await page.setViewportSize({ width: 320, height: 740 });
  await mobileTrigger.click();
  const mobileMenu = page.getByRole('menu', { name: '账号菜单' });
  await expect(page.getByRole('menu')).toHaveCount(1);
  await expect(
    mobileMenu.getByRole('group', { name: '外观主题' }).getByRole('menuitemradio'),
  ).toHaveCount(3);
  expect(await mobileMenu.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
    true,
  );
  await page.screenshot({ path: '../output/ui/theme-controls-mobile.png', animations: 'disabled' });
  await mobileMenu.getByRole('menuitemradio', { name: '深色' }).click();
  await expect(mobileMenu).toHaveCount(0);
  await expect(page.locator('html')).toHaveClass(/dark/);

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
 * Run the reduced-motion dynamic settings browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function reducedMotionDynamicSettingsTest({ page }: { page: Page }): Promise<void> {
  await verifiesReducedMotionDynamicSettings(page);
}

/**
 * Run the administrator-center browser test.
 *
 * @param fixtures - Playwright page fixture.
 */
async function administratorCenterTest({ page }: { page: Page }): Promise<void> {
  await verifiesAdministratorCenter(page);
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

/**
 * Serve a long, badged article and a successful favorite mutation for polish checks.
 *
 * @param route - Intercepted API route.
 */
async function serveInterfacePolishApi(route: Route): Promise<void> {
  const pathname = new URL(route.request().url()).pathname;
  if (pathname === '/api/articles') {
    await fulfillJson(route, {
      items: [
        {
          article_id: 'polish-fixture',
          title:
            'Graph Evidence Synthesis for Transparent and Reproducible Living Literature Reviews',
          journal_title: 'Journal of Reproducible Literature',
          authors: ['Browser Fixture'],
          date: '2026-07-15',
          abstract:
            'A readable summary with clear metadata and explicit actions on a narrow screen.',
          open_access: 1,
          in_press: 1,
        },
      ],
      page: { total: null, limit: 50, offset: 0, next_cursor: null, has_more: false },
    });
    return;
  }
  if (pathname === '/api/favorites/check') {
    await fulfillJson(route, []);
    return;
  }
  if (pathname === '/api/articles/polish-fixture/access') {
    await fulfillJson(route, {
      abstract_page: { available: true, label: '查看摘要页', requires_login: false, message: null },
      fulltext: { available: true, label: '获取全文', requires_login: false, message: null },
    });
    return;
  }
  if (pathname === '/api/favorites/folders/4/articles' && route.request().method() === 'POST') {
    await fulfillJson(route, {
      id: 2,
      folder_id: 4,
      article_id: 'polish-fixture',
      db_name: 'fixture.sqlite',
      note: '',
      created_at: 2,
    });
    return;
  }
  await serveTrackingApi(route);
}

/**
 * Verify an actual control box is large enough without relying on overlapping pseudo-elements.
 *
 * @param control - Interactive element to measure.
 * @param minimumSize - Minimum width and height in CSS pixels.
 */
async function expectComfortableTarget(control: Locator, minimumSize: number): Promise<void> {
  await expect(control).toBeVisible();
  await expect
    .poll(async () => {
      const box = await control.boundingBox();
      return Math.min(box?.width ?? 0, box?.height ?? 0);
    })
    .toBeGreaterThanOrEqual(minimumSize);
}

/**
 * Verify every dialog uses the same corner inset and the shared button radius.
 *
 * @param dialog - Visible dialog to inspect.
 * @param size - Expected touch target size for the active viewport.
 */
async function expectConsistentDialogClose(dialog: Locator, size: number): Promise<void> {
  await expect(dialog).toBeVisible();
  await expect
    .poll(() =>
      dialog.evaluate((element) => {
        const close = element.querySelector<HTMLElement>(':scope > [data-slot="dialog-close"]');
        if (!close) return null;
        const dialogBox = element.getBoundingClientRect();
        const closeBox = close.getBoundingClientRect();
        const dialogStyle = getComputedStyle(element);
        const standardButton = document.querySelector('[data-slot="button"][type="submit"]');
        return {
          top: Math.round(
            closeBox.top - dialogBox.top - Number.parseFloat(dialogStyle.borderTopWidth),
          ),
          right: Math.round(
            dialogBox.right - closeBox.right - Number.parseFloat(dialogStyle.borderRightWidth),
          ),
          width: Math.round(closeBox.width),
          height: Math.round(closeBox.height),
          hasSharedRadius:
            standardButton !== null &&
            getComputedStyle(close).borderTopLeftRadius ===
              getComputedStyle(standardButton).borderTopLeftRadius,
        };
      }),
    )
    .toEqual({ top: 16, right: 16, width: size, height: size, hasSharedRadius: true });
}

/**
 * Measure rendered text contrast after compositing a control over the active theme surface.
 *
 * @param control - Control whose text must remain readable in either theme.
 */
async function expectReadableControlText(control: Locator): Promise<void> {
  const contrast = await control.evaluate((element) => {
    const canvas = document.createElement('canvas');
    canvas.width = 1;
    canvas.height = 1;
    const context = canvas.getContext('2d');
    if (!context) throw new Error('Canvas is required to resolve computed CSS colors');
    const styles = getComputedStyle(element);
    context.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--background');
    context.fillRect(0, 0, 1, 1);
    context.fillStyle = styles.backgroundColor;
    context.fillRect(0, 0, 1, 1);
    const background = Array.from(context.getImageData(0, 0, 1, 1).data).slice(0, 3);
    context.fillStyle = styles.color;
    context.fillRect(0, 0, 1, 1);
    const foreground = Array.from(context.getImageData(0, 0, 1, 1).data).slice(0, 3);
    const luminances = [background, foreground].map((channels) =>
      channels.reduce((luminance, channel, index) => {
        const value = channel / 255;
        const linearValue = value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
        return luminance + linearValue * [0.2126, 0.7152, 0.0722][index];
      }, 0),
    );
    return (Math.max(...luminances) + 0.05) / (Math.min(...luminances) + 0.05);
  });
  expect(contrast).toBeGreaterThanOrEqual(4.5);
}

/**
 * Verify touch targets, readable mobile titles, keyboard focus, and restrained press feedback.
 *
 * @param fixtures - Playwright page fixture.
 */
async function interfacePolishControlsTest({ page }: { page: Page }): Promise<void> {
  await page.route('**/api/**', serveInterfacePolishApi);
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'no-preference' });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?q=graph');
  await hideDevelopmentIndicator(page);

  const input = page.getByRole('combobox', { name: '搜索文章' });
  const clear = page.getByRole('button', { name: '清空搜索输入' });
  const submit = page.getByRole('button', { name: '搜索', exact: true });
  await expectComfortableTarget(clear, 44);
  await expectComfortableTarget(submit, 44);
  await expectComfortableTarget(page.getByRole('button', { name: '搜索语法帮助' }), 44);
  await expectComfortableTarget(page.getByRole('button', { name: '打开筛选器' }), 44);
  await expectComfortableTarget(page.getByRole('button', { name: '移除搜索 graph' }), 44);

  const card = page.locator('[data-slot="card"]').filter({ hasText: 'Graph Evidence' });
  const title = card.locator('[data-slot="card-title"]');
  const titleBox = await title.boundingBox();
  const badgeBox = await card.getByText('开放获取', { exact: true }).boundingBox();
  expect(badgeBox?.y ?? 0).toBeGreaterThanOrEqual((titleBox?.y ?? 0) + (titleBox?.height ?? 0));
  await expect(title).toHaveCSS('text-wrap-style', 'balance');
  await page.screenshot({ path: '../output/ui/polish-search-mobile.png', fullPage: true });

  const details = page.getByRole('button', { name: /^查看文章详情：/ });
  await expectComfortableTarget(details, 44);
  await expect(card.locator('[data-slot="card-footer"]')).toHaveCount(0);
  await title.click();
  const dialog = page.getByRole('dialog', { name: /Graph Evidence/ });
  await expectConsistentDialogClose(dialog, 44);
  const articleActions = dialog.getByRole('group', { name: '文章操作' });
  await expect(articleActions.getByRole('link', { name: '查看摘要页' })).toBeVisible();
  await expect(articleActions.getByRole('link', { name: '获取全文' })).toBeVisible();
  await expect(articleActions.getByRole('button', { name: '复制信息' })).toBeVisible();
  await expect(articleActions.getByRole('button', { name: '收藏', exact: true })).toBeVisible();
  const actionControls = articleActions.locator('button, a');
  await expect(actionControls).toHaveCount(4);
  for (const control of await actionControls.all()) {
    await expectComfortableTarget(control, 44);
    await expect.poll(() => control.innerText()).toBe('');
  }
  await page.screenshot({
    path: '../output/ui/article-dialog-mobile.png',
    fullPage: true,
    animations: 'disabled',
  });
  await page.setViewportSize({ width: 320, height: 740 });
  expect(
    await articleActions.evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.screenshot({
    path: '../output/ui/article-dialog-narrow.png',
    fullPage: true,
    animations: 'disabled',
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.keyboard.press('Escape');
  await expect(details).toBeFocused();
  for (const key of ['Enter', 'Space']) {
    await details.press(key);
    await expect(dialog).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(details).toBeFocused();
  }

  await page.setViewportSize({ width: 1280, height: 900 });
  await expectComfortableTarget(clear, 40);
  const animationSession = await page.context().newCDPSession(page);
  await animationSession.send('Animation.enable');
  await animationSession.send('Animation.setPlaybackRate', { playbackRate: 0.1 });
  expect(await animationSession.send('Animation.getPlaybackRate')).toEqual({ playbackRate: 0.1 });
  await submit.hover();
  await page.mouse.down();
  await expect
    .poll(() =>
      submit.evaluate((element) =>
        element
          .getAnimations()
          .some(
            (animation) =>
              animation instanceof CSSTransition && animation.transitionProperty === 'scale',
          ),
      ),
    )
    .toBe(true);
  await expect(submit).toHaveCSS('scale', '0.96');
  await page.screenshot({ path: '../output/ui/polish-press-desktop.png', fullPage: true });
  await page.mouse.up();
  await expect(submit).toHaveCSS('scale', 'none');
  await animationSession.send('Animation.setPlaybackRate', { playbackRate: 1 });
  await animationSession.detach();
  await clear.hover();
  await page.mouse.down();
  await expect(clear).toHaveCSS('scale', 'none');
  await expect(clear).not.toHaveAttribute('static');
  await page.mouse.up();
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('');
  await expect(page).toHaveURL('/?q=graph');

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await submit.hover();
  await page.mouse.down();
  await expect(submit).toHaveCSS('scale', 'none');
  await page.mouse.move(1, 1);
  await page.mouse.up();
  await page.setViewportSize({ width: 320, height: 740 });
  expect(await page.locator('body').evaluate((element) => element.scrollWidth <= innerWidth)).toBe(
    true,
  );
}

/**
 * Verify favorite state changes preserve control geometry and provide readable, semantic feedback.
 *
 * @param fixtures - Playwright page fixture.
 */
async function interfacePolishFavoriteTest({ page }: { page: Page }): Promise<void> {
  await page.route('**/api/**', serveInterfacePolishApi);
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/?q=graph');
  await hideDevelopmentIndicator(page);
  await page.getByRole('button', { name: /^查看文章详情：/ }).click();
  const trigger = page.getByRole('button', { name: '收藏', exact: true });
  await expect(trigger).toBeVisible();
  await expect(trigger.getByText('收藏', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('link', { name: '查看摘要页' }).getByText('查看摘要页'),
  ).toBeVisible();
  await expect(page.getByRole('dialog', { name: /Graph Evidence/ })).toHaveCSS('scale', '1');
  await page.evaluate(() => document.fonts.ready);
  const initialWidth = (await trigger.boundingBox())?.width ?? 0;
  await trigger.click();
  const folder = page.getByRole('button', { name: 'Tracking', exact: true });
  await folder.click();
  const selectedTrigger = page.getByRole('button', { name: '已收藏', exact: true });
  await expect(selectedTrigger).toBeVisible();
  await expect.poll(async () => (await selectedTrigger.boundingBox())?.width).toBe(initialWidth);
  await expect(folder).toHaveAttribute('aria-pressed', 'true');
  await expectComfortableTarget(folder, 40);
  expect(
    await selectedTrigger.locator('svg').evaluate((element) => {
      const style = getComputedStyle(element);
      return style.fill === style.color;
    }),
  ).toBe(true);
  await expect(page.getByRole('link', { name: '查看摘要页' })).toBeVisible();
  await page.screenshot({
    path: '../output/ui/polish-favorite-light.png',
    fullPage: true,
    animations: 'disabled',
  });
  await expectReadableControlText(selectedTrigger);
  await expectReadableControlText(folder);
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveClass(/dark/);
  await page.screenshot({
    path: '../output/ui/polish-favorite-dark.png',
    fullPage: true,
    animations: 'disabled',
  });
  await expectReadableControlText(selectedTrigger);
  await expectReadableControlText(folder);
  await page.keyboard.press('Escape');
  await expect(page.locator('[data-slot="popover-content"]')).toHaveCount(0);
  await expect(selectedTrigger).toBeFocused();
}

/**
 * Verify the server document opts out of extension theming before any client script can run.
 *
 * @param fixtures - Playwright browser and configured application URL.
 */
async function nativeThemeDocumentTest({
  browser,
  baseURL,
}: {
  browser: Browser;
  baseURL?: string;
}): Promise<void> {
  const context = await browser.newContext({ baseURL, javaScriptEnabled: false });
  try {
    const page = await context.newPage();
    await page.goto('/login');
    await expect(page.locator('head meta[name="darkreader-lock"]')).toHaveCount(1);
  } finally {
    await context.close();
  }
}

/**
 * Serve enough area filters to require scrolling inside a mobile sidebar.
 *
 * @param route - Intercepted fixture API request.
 */
async function serveLongSidebarApi(route: Route): Promise<void> {
  if (new URL(route.request().url()).pathname === '/api/meta/areas') {
    await fulfillJson(
      route,
      Array.from({ length: 24 }, (unusedValue, index) => ({
        value: `field_${index + 1}`,
        count: index + 1,
      })),
    );
    return;
  }
  await serveTrackingApi(route);
}

/**
 * Verify long drawers scroll with real wheel and touch input and dismiss without a close button.
 *
 * @param fixtures - Playwright page fixture.
 */
async function mobileSidebarScrollTest({ page }: { page: Page }): Promise<void> {
  await page.route('**/api/**', serveLongSidebarApi);
  await page.setViewportSize({ width: 616, height: 751 });
  await page.goto('/?q=graph');
  await hideDevelopmentIndicator(page);
  const trigger = page.getByRole('button', { name: '打开筛选器' });
  await trigger.click();
  const drawer = page.getByRole('dialog', { name: '筛选器' });
  await expect(drawer.getByText('field_24', { exact: true })).toBeAttached();
  const scrollContainer = drawer.locator('aside > div').first();
  const lastSection = drawer.getByText('暂无可用发表年份');
  await expect(lastSection).not.toBeInViewport();
  await page.screenshot({ path: '../output/ui/mobile-sidebar-top.png', animations: 'disabled' });

  await page.mouse.move(160, 500);
  await page.mouse.wheel(0, 2000);
  await expect
    .poll(() => scrollContainer.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
  await expect(lastSection).toBeInViewport();
  await expect(drawer.getByRole('button', { name: '关闭', exact: true })).toHaveCount(0);
  await page.screenshot({ path: '../output/ui/mobile-sidebar-bottom.png', animations: 'disabled' });

  const touchSession = await page.context().newCDPSession(page);
  try {
    await touchSession.send('Emulation.setTouchEmulationEnabled', {
      enabled: true,
      maxTouchPoints: 1,
    });
    /**
     * Dispatch a trusted touch drag through successive browser frames.
     *
     * @param startY - Starting vertical position in viewport CSS pixels.
     * @param distance - Signed vertical distance traveled by the finger.
     */
    async function swipeSidebar(startY: number, distance: number): Promise<void> {
      await touchSession.send('Input.dispatchTouchEvent', {
        type: 'touchStart',
        touchPoints: [{ x: 160, y: startY }],
      });
      for (let step = 1; step <= 8; step += 1) {
        await touchSession.send('Input.dispatchTouchEvent', {
          type: 'touchMove',
          touchPoints: [{ x: 160, y: startY + (distance * step) / 8 }],
        });
        await page.evaluate(() => new Promise(requestAnimationFrame));
      }
      await touchSession.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    }
    const initialScrollTop = await scrollContainer.evaluate((element) => element.scrollTop);
    await swipeSidebar(120, 500);
    await expect
      .poll(() => scrollContainer.evaluate((element) => element.scrollTop))
      .toBeLessThan(initialScrollTop);
    const upperScrollTop = await scrollContainer.evaluate((element) => element.scrollTop);
    await swipeSidebar(640, -400);
    await expect
      .poll(() => scrollContainer.evaluate((element) => element.scrollTop))
      .toBeGreaterThan(upperScrollTop);
  } finally {
    await touchSession.detach();
  }

  await page.mouse.click(600, 100);
  await expect(drawer).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.keyboard.press('Escape');
  await expect(drawer).toHaveCount(0);
  await expect(trigger).toBeFocused();
}

/**
 * Verify logout and a new login propagate through native storage events in one cookie context.
 *
 * @param fixtures - Browser page and configured application origin.
 */
async function crossTabSessionTest({
  page,
  baseURL,
}: {
  page: Page;
  baseURL?: string;
}): Promise<void> {
  if (!baseURL) {
    throw new Error('The browser fixture requires an application base URL');
  }
  const context = page.context();
  await context.addCookies([{ name: 'litradar_review_session', value: 'first', url: baseURL }]);
  await context.route('**/api/**', async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    if (pathname === '/api/auth/me') {
      const cookie = route.request().headers().cookie ?? '';
      if (cookie.includes('litradar_review_session=second')) {
        await fulfillJson(route, { id: 55, username: 'browser_second', is_admin: false });
      } else if (cookie.includes('litradar_review_session=first')) {
        await fulfillJson(route, { id: 41, username: 'browser_user', is_admin: false });
      } else {
        await fulfillJson(route, { detail: 'Authentication required' }, 401);
      }
      return;
    }
    if (pathname === '/api/auth/invite-required') {
      await fulfillJson(route, { required: false, bootstrap_required: false });
      return;
    }
    if (pathname === '/api/auth/logout' || pathname === '/api/auth/login') {
      const isLogin = pathname.endsWith('/login');
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        headers: {
          'Set-Cookie': isLogin
            ? 'litradar_review_session=second; Path=/; HttpOnly'
            : 'litradar_review_session=; Path=/; Max-Age=0; HttpOnly',
        },
        body: JSON.stringify(
          isLogin
            ? {
                expires_at: Date.now() / 1000 + 3600,
                user: { id: 55, username: 'browser_second', is_admin: false },
              }
            : { ok: true },
        ),
      });
      return;
    }
    await serveTrackingApi(route);
  });
  const secondPage = await context.newPage();
  await page.goto('/');
  await secondPage.goto('/');
  await expect(page.getByRole('button', { name: '打开账号菜单：browser_user' })).toBeVisible();
  await secondPage.getByRole('button', { name: '打开账号菜单：browser_user' }).click();
  await secondPage.getByRole('menuitem', { name: '退出登录' }).click();
  await expect(page).toHaveURL(/\/login/);
  await expect(secondPage).toHaveURL(/\/login/);
  await secondPage.getByLabel('用户名').fill('browser_second');
  await secondPage.getByLabel('密码', { exact: true }).fill('browser-second-password');
  await secondPage.getByRole('button', { name: '登录', exact: true }).first().click();
  await expect(page.getByRole('button', { name: '打开账号菜单：browser_second' })).toBeVisible();
  await expect(
    secondPage.getByRole('button', { name: '打开账号菜单：browser_second' }),
  ).toBeVisible();
  expect(
    (await context.cookies()).find((cookie) => cookie.name === 'litradar_review_session')?.value,
  ).toBe('second');
  await secondPage.close();
}

/**
 * Serve article access that requires opening the data-source settings center.
 *
 * @param route - Intercepted fixture API request.
 */
async function serveArticleSettingsApi(route: Route): Promise<void> {
  const pathname = new URL(route.request().url()).pathname;
  if (pathname.endsWith('/access')) {
    await fulfillJson(route, {
      abstract_page: {
        available: true,
        label: '查看摘要页',
        requires_login: false,
        message: null,
      },
      fulltext: {
        available: false,
        label: '获取全文',
        requires_login: true,
        message: '需要登录',
      },
    });
    return;
  }
  if (pathname === '/api/cnki/session') {
    await fulfillJson(route, {
      configured: false,
      status: 'empty',
      expires_at: null,
      cookie_names: [],
    });
    return;
  }
  await serveTrackingApi(route);
}

/**
 * Keep data-source settings usable after navigation unmounts the article dialog.
 *
 * @param fixtures - Playwright page fixture.
 */
async function articleDataSourceSettingsTest({ page }: { page: Page }): Promise<void> {
  await page.route('**/api/**', serveArticleSettingsApi);
  for (const workspace of ['/?q=graph', '/?view=favorites&folder=4', '/?view=weekly-updates']) {
    await page.goto(workspace);
    const articleTrigger = page.getByRole('button', { name: /^查看文章详情：/ }).first();
    await articleTrigger.click();
    const articleDialog = page.getByRole('dialog');
    await articleDialog.getByRole('link', { name: '去设置登录' }).click();

    await expect(page).toHaveURL(
      new RegExp(`${workspace.replace('?', '\\?')}&settings=data-sources$`),
    );
    const settingsDialog = page.getByRole('dialog', { name: '设置中心' });
    await expect(settingsDialog.getByText('未配置', { exact: true })).toBeVisible();
    await settingsDialog.getByRole('button', { name: '刷新 CNKI 登录状态' }).click();
    await expect(settingsDialog).toBeVisible();
    await expect(page.getByRole('dialog')).toHaveCount(1);
    await settingsDialog.getByRole('button', { name: '关闭', exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`${workspace.replace('?', '\\?')}$`));
    await expect(settingsDialog).toHaveCount(0);
  }
}

test('opens usable data-source settings from article details', articleDataSourceSettingsTest);
test('synchronizes logout and account changes across browser tabs', crossTabSessionTest);

test('scrolls a long mobile sidebar and dismisses without a close button', mobileSidebarScrollTest);
test('declares native theme ownership before hydration', nativeThemeDocumentTest);
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
test('preserves dynamic settings under reduced motion', reducedMotionDynamicSettingsTest);
test('supports the administrator center across desktop and mobile', administratorCenterTest);
test('supports three deep-linkable root workspaces', unifiedRootWorkspacesTest);
test('supports accessible navigation and theme selection', userMenuNavigationTest);
test('polishes search targets and restrained control feedback', interfacePolishControlsTest);
test('polishes stable and semantic favorite feedback', interfacePolishFavoriteTest);
