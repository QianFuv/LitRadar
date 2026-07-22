/**
 * Serial browser journeys through the real Rust listener and disposable SQLite state.
 */

import { expect, test, type Page, type TestInfo } from '@playwright/test';

import type { AdminUserInfo, FavoriteArticleItem, Folder } from '@/lib/api';

const ADMIN_USERNAME = 'fullstack_admin';
const ADMIN_PASSWORD = 'FullStackAdmin!2026';
const MEMBER_USERNAME = 'fullstack_member';
const MEMBER_PASSWORD = 'FullStackMember!2026';
const ARTICLE_TITLE = 'Evidence Graphs for Living Literature Reviews';
const CREATED_ANNOUNCEMENT_TITLE_PREFIX = 'Browser-persisted release notice';

test.describe.configure({ mode: 'serial' });

/**
 * Authenticate one seeded account through the public login form.
 *
 * @param page - Playwright page.
 * @param username - Seeded username.
 * @param password - Seeded password.
 * @returns Promise resolved after the protected workspace loads.
 */
async function login(page: Page, username: string, password: string): Promise<void> {
  await page.goto('/login');
  await page.getByLabel('用户名').fill(username);
  await page.getByLabel('密码', { exact: true }).fill(password);
  const loginResponse = page.waitForResponse(
    (response) =>
      response.request().method() === 'POST' &&
      new URL(response.url()).pathname === '/api/auth/login',
  );
  await page.getByRole('button', { name: '登录', exact: true }).click();
  expect((await loginResponse).ok()).toBe(true);
  await dismissVisibleAnnouncements(page);
  await expect(page.getByRole('button', { name: `打开账号菜单：${username}` })).toBeVisible();
}

/**
 * Permanently dismiss seeded announcements when the search workspace presents them.
 *
 * @param page - Playwright page.
 * @returns Promise resolved after any visible announcement closes.
 */
async function dismissVisibleAnnouncements(page: Page): Promise<void> {
  const announcementDialog = page.getByRole('dialog', { name: '系统公告' });
  const didAppear = await announcementDialog
    .waitFor({ state: 'visible', timeout: 5_000 })
    .then(() => true)
    .catch(() => false);
  if (didAppear) {
    await announcementDialog.getByRole('button', { name: '永久关闭' }).click();
    await expect(announcementDialog).toHaveCount(0);
  }
}

/**
 * Assert that the browser owns a server-created HttpOnly session cookie.
 *
 * @param page - Playwright page.
 * @returns Promise resolved after cookie validation.
 */
async function expectHttpOnlySession(page: Page): Promise<void> {
  const cookies = await page.context().cookies();
  const sessionCookie = cookies.find((cookie) => cookie.name === 'litradar_session');
  expect(sessionCookie).toBeDefined();
  expect(sessionCookie?.httpOnly).toBe(true);
}

/**
 * Restore the seeded Reading folder to its empty precondition.
 *
 * @param page - Authenticated Playwright page.
 * @returns Promise resolved after any persisted fixture favorites are removed.
 */
async function emptyReadingFolder(page: Page): Promise<void> {
  const foldersResponse = await page.request.get('/api/favorites/folders');
  expect(foldersResponse.ok()).toBe(true);
  const folders = (await foldersResponse.json()) as Folder[];
  const readingFolder = folders.find((folder) => folder.name === 'Reading');
  if (!readingFolder) {
    throw new Error('seeded Reading folder is missing');
  }

  const articlesResponse = await page.request.get(
    `/api/favorites/folders/${readingFolder.id}/articles?limit=100&offset=0`,
  );
  expect(articlesResponse.ok()).toBe(true);
  const articles = (await articlesResponse.json()) as FavoriteArticleItem[];
  if (articles.length === 0) {
    return;
  }

  const removeResponse = await page.request.post(
    `/api/favorites/folders/${readingFolder.id}/articles/bulk-remove`,
    {
      data: {
        articles: articles.map((article) => ({
          article_id: article.article_id,
          db_name: article.db_name,
        })),
      },
    },
  );
  expect(removeResponse.ok()).toBe(true);
  const removalResult = (await removeResponse.json()) as { count: number };
  expect(removalResult.count).toBe(articles.length);
}

/**
 * Restore the seeded member to the non-administrator precondition.
 *
 * @param page - Authenticated administrator page.
 * @returns Promise resolved after any stale administrator role is revoked.
 */
async function ensureMemberIsNotAdministrator(page: Page): Promise<void> {
  const usersResponse = await page.request.get('/api/admin/users');
  expect(usersResponse.ok()).toBe(true);
  const users = (await usersResponse.json()) as AdminUserInfo[];
  const member = users.find((user) => user.username === MEMBER_USERNAME);
  if (!member) {
    throw new Error(`seeded member ${MEMBER_USERNAME} is missing`);
  }
  if (!member.is_admin) {
    return;
  }

  const revokeResponse = await page.request.put(`/api/admin/users/${member.id}/admin`, {
    data: { is_admin: false },
  });
  expect(revokeResponse.ok()).toBe(true);
}

/**
 * Exercise search, article detail, and persisted favorites as the seeded member.
 *
 * @param fixtures - Playwright fixtures.
 * @returns Promise resolved after the journey.
 */
async function searchAndFavoriteJourney({ page }: { page: Page }): Promise<void> {
  await login(page, MEMBER_USERNAME, MEMBER_PASSWORD);
  await expectHttpOnlySession(page);
  await emptyReadingFolder(page);
  await page.reload();
  await expect(
    page.getByRole('button', { name: `打开账号菜单：${MEMBER_USERNAME}` }),
  ).toBeVisible();

  const searchInput = page.getByRole('combobox', { name: '搜索文章' });
  await searchInput.fill(ARTICLE_TITLE);
  await searchInput.press('Enter');
  await expect(page).toHaveURL(/\?q=Evidence(?:%20|\+)Graphs/);
  await expect(page.getByText(ARTICLE_TITLE, { exact: true }).first()).toBeVisible();

  await page.getByRole('button', { name: '查看详情' }).first().click();
  let articleDialog = page.getByRole('dialog', { name: ARTICLE_TITLE });
  await expect(articleDialog).toBeVisible();
  await articleDialog.getByRole('button', { name: '收藏', exact: true }).click();
  const favoriteResponse = page.waitForResponse(
    (response) =>
      response.request().method() === 'POST' &&
      /\/api\/favorites\/folders\/\d+\/articles$/.test(new URL(response.url()).pathname),
  );
  await page.getByRole('button', { name: 'Reading', exact: true }).click();
  expect((await favoriteResponse).ok()).toBe(true);
  await expect(articleDialog.getByRole('button', { name: '已收藏', exact: true })).toBeVisible();

  await page.reload();
  await expect(page.getByText(ARTICLE_TITLE, { exact: true }).first()).toBeVisible();
  await page.getByRole('button', { name: '查看详情' }).first().click();
  articleDialog = page.getByRole('dialog', { name: ARTICLE_TITLE });
  await expect(articleDialog.getByRole('button', { name: '已收藏', exact: true })).toBeVisible();
}

/**
 * Persist administrator user, invite, and announcement mutations across refetches.
 *
 * @param fixtures - Playwright fixtures.
 * @param testInfo - Retry and repeat identity for attempt-unique persisted data.
 * @returns Promise resolved after all mutations are reloaded and verified.
 */
async function administratorMutationJourney(
  { page }: { page: Page },
  testInfo: TestInfo,
): Promise<void> {
  await login(page, ADMIN_USERNAME, ADMIN_PASSWORD);
  await expectHttpOnlySession(page);
  await ensureMemberIsNotAdministrator(page);
  const createdAnnouncementTitle = `${CREATED_ANNOUNCEMENT_TITLE_PREFIX} ${testInfo.repeatEachIndex}-${testInfo.retry}`;
  await page.goto('/?admin=overview');
  await expect(page).toHaveURL(/\/?\?admin=overview$/);
  const adminDialog = page.getByRole('dialog', { name: '管理面板' });
  const adminCategories = adminDialog.getByRole('navigation', { name: '管理分类' });
  await expect(adminDialog).toBeVisible();
  await adminCategories.getByRole('button', { name: '用户', exact: true }).click();
  await expect(page).toHaveURL(/\/?\?admin=users$/);

  const grantAdminButton = adminDialog.getByRole('button', {
    name: `设为 ${MEMBER_USERNAME} 为管理员`,
  });
  await expect(grantAdminButton).toBeVisible();
  const grantResponse = page.waitForResponse(
    (response) =>
      response.request().method() === 'PUT' &&
      /\/api\/admin\/users\/\d+\/admin$/.test(new URL(response.url()).pathname),
  );
  await grantAdminButton.click();
  expect((await grantResponse).ok()).toBe(true);
  await expect(
    adminDialog.getByRole('button', { name: `取消 ${MEMBER_USERNAME} 的管理员` }),
  ).toBeVisible();
  await page.reload();
  await expect(
    adminDialog.getByRole('button', { name: `取消 ${MEMBER_USERNAME} 的管理员` }),
  ).toBeVisible();

  await adminCategories.getByRole('button', { name: '邀请码', exact: true }).click();
  await expect(page).toHaveURL(/\/?\?admin=invite-codes$/);
  const inviteResponse = page.waitForResponse(
    (response) =>
      response.request().method() === 'POST' &&
      new URL(response.url()).pathname === '/api/admin/invite-codes',
  );
  await adminDialog.getByRole('button', { name: '生成邀请码' }).click();
  const createdInviteResponse = await inviteResponse;
  expect(createdInviteResponse.ok()).toBe(true);
  const createdInvite = (await createdInviteResponse.json()) as { code: string };
  const invitePrefix = `${createdInvite.code.slice(0, 8)}…`;
  await expect(page.getByText(invitePrefix, { exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByText(invitePrefix, { exact: true })).toBeVisible();

  await adminCategories.getByRole('button', { name: '公告', exact: true }).click();
  await expect(page).toHaveURL(/\/?\?admin=announcements$/);
  await page.getByRole('button', { name: '新建公告' }).click();
  const announcementDialog = page.getByRole('dialog', { name: '新建公告' });
  await announcementDialog.getByLabel('公告标题').fill(createdAnnouncementTitle);
  await announcementDialog
    .getByLabel('公告内容')
    .fill('Created through the browser against the disposable auth database.');
  const announcementResponse = page.waitForResponse(
    (response) =>
      response.request().method() === 'POST' &&
      new URL(response.url()).pathname === '/api/admin/announcements',
  );
  await announcementDialog.getByRole('button', { name: '创建', exact: true }).click();
  expect((await announcementResponse).ok()).toBe(true);
  await dismissVisibleAnnouncements(page);
  await expect(adminDialog.getByText(createdAnnouncementTitle, { exact: true })).toBeVisible();
  await page.reload();
  await expect(adminDialog.getByText(createdAnnouncementTitle, { exact: true })).toBeVisible();

  await adminCategories.getByRole('button', { name: '用户', exact: true }).click();
  await expect(page).toHaveURL(/\/?\?admin=users$/);
  const revokeAdminButton = adminDialog.getByRole('button', {
    name: `取消 ${MEMBER_USERNAME} 的管理员`,
  });
  const revokeResponse = page.waitForResponse(
    (response) =>
      response.request().method() === 'PUT' &&
      /\/api\/admin\/users\/\d+\/admin$/.test(new URL(response.url()).pathname),
  );
  await revokeAdminButton.click();
  expect((await revokeResponse).ok()).toBe(true);
  await page.reload();
  await expect(
    adminDialog.getByRole('button', { name: `设为 ${MEMBER_USERNAME} 为管理员` }),
  ).toBeVisible();
}

/**
 * Verify the removed administrator route, API authorization, and logout boundary.
 *
 * @param fixtures - Playwright fixtures.
 * @returns Promise resolved after permission boundaries are verified.
 */
async function protectedRouteJourney({ page }: { page: Page }): Promise<void> {
  await login(page, MEMBER_USERNAME, MEMBER_PASSWORD);
  const authenticatedAdminRoute = await page.goto('/admin');
  expect(authenticatedAdminRoute?.status()).toBe(404);
  await expect(page.getByRole('heading', { name: '页面未找到' })).toBeVisible();
  expect((await page.request.get('/api/admin/users')).status()).toBe(403);

  await page.goto('/');
  await page.getByRole('button', { name: `打开账号菜单：${MEMBER_USERNAME}` }).click();
  await page.getByRole('menuitem', { name: '退出登录' }).click();
  await expect(page).toHaveURL(/\/login\?next=%2F$/);
  expect(
    (await page.context().cookies()).some((cookie) => cookie.name === 'litradar_session'),
  ).toBe(false);

  const anonymousAdminRoute = await page.goto('/admin');
  expect(anonymousAdminRoute?.status()).toBe(404);
  await expect(page.getByRole('heading', { name: '页面未找到' })).toBeVisible();
  expect((await page.request.get('/api/admin/users')).status()).toBe(401);
}

test('searches and persists a favorite through the real backend', searchAndFavoriteJourney);
test('persists administrator mutations through the real backend', administratorMutationJourney);
test('enforces authenticated and administrator route boundaries', protectedRouteJourney);
