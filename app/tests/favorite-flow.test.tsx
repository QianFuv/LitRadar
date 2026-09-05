/**
 * Favorite cache update coverage using the production button and API client.
 */

import type { ReactNode } from 'react';
import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { FavoriteButton } from '@/components/feature/favorite-button';
import { FavoritesPageContent } from '@/components/favorites/favorites-page-content';
import { AuthProvider, useAuth } from '@/lib/auth-context';
import type { FavoriteArticleItem, FavoriteCheck } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { favoriteArticlePageResponse } from '@/tests/mocks/handlers/favorites';
import { renderWithQuery } from '@/tests/render';

const favoriteFlowMocks = vi.hoisted(() => ({
  useVisiblePageList: vi.fn(() => ({
    loadMoreRef: () => undefined,
    prefetchRef: () => undefined,
    visiblePages: 1,
  })),
}));

vi.mock('@/components/feature/use-visible-page-list', () => ({
  useVisiblePageList: favoriteFlowMocks.useVisiblePageList,
}));

vi.mock('@/components/feature/article-dialog-card', () => ({
  ArticleDialogCard: ({
    article,
    extraActions,
    leading,
    preview,
  }: {
    article: FavoriteArticleItem;
    extraActions?: ReactNode;
    leading?: ReactNode;
    preview?: ReactNode;
  }) => (
    <div>
      {leading}
      <span>{article.title}</span>
      {preview}
      {extraActions}
    </div>
  ),
}));

let favoriteRequestBody: unknown = null;
let removeRequestCount = 0;

/**
 * Return an authenticated fixture user.
 *
 * @returns Current-user response.
 */
function currentUserResponse(): Response {
  return HttpResponse.json({ id: 21, username: 'favorite_user', is_admin: false });
}

/**
 * Return one available favorite folder.
 *
 * @returns Favorite folder list response.
 */
function foldersResponse(): Response {
  return HttpResponse.json([
    { id: 3, name: 'Reading', is_tracking: false, article_count: 0, created_at: 1 },
  ]);
}

/**
 * Return an initially empty favorite check.
 *
 * @returns Empty favorite check response.
 */
function emptyFavoriteResponse(): Response {
  return HttpResponse.json([]);
}

/**
 * Return one existing favorite folder membership.
 *
 * @returns Populated favorite check response.
 */
function existingFavoriteResponse(): Response {
  return HttpResponse.json([{ folder_id: 3, folder_name: 'Reading' }]);
}

/**
 * Record a confirmed favorite removal.
 *
 * @param context - MSW request context.
 * @returns Successful removal response.
 */
function removeFavoriteResponse(context: { request: Request }): Response {
  const requestUrl = new URL(context.request.url);
  expect(requestUrl.searchParams.get('db_name')).toBe('fixture.sqlite');
  removeRequestCount += 1;
  return HttpResponse.json({ ok: true });
}

/**
 * Build a loaded favorite article fixture.
 *
 * @param id - Favorite row identifier.
 * @returns Favorite article record.
 */
function favoriteArticleFixture(id: number): FavoriteArticleItem {
  return {
    id,
    folder_id: 3,
    article_id: `article-${id}`,
    db_name: 'fixture.sqlite',
    note: '',
    created_at: 2,
    metadata_status: 'available',
    title: `Article ${id}`,
    authors: ['Researcher'],
    abstract: `Abstract ${id}`,
  };
}

/**
 * Render the favorites page with a stable selected folder.
 *
 * @returns Rendered query utilities.
 */
function renderFavoritesPage(): ReturnType<typeof renderWithQuery> {
  server.use(http.get('http://localhost/api/auth/me', currentUserResponse));
  return renderWithQuery(
    <AuthProvider>
      <AuthenticatedFavoriteFixture />
    </AuthProvider>,
  );
}

/** Render private fixture content only after its real authentication boundary resolves. */
function AuthenticatedFavoriteFixture() {
  const { user, loading } = useAuth();
  if (loading || !user) return null;
  return (
    <NuqsTestingAdapter searchParams="?folder=3">
      <FavoritesPageContent userId={user.id} />
    </NuqsTestingAdapter>
  );
}

/**
 * Capture the add-favorite request and return the inserted row.
 *
 * @param context - MSW request context.
 * @returns Inserted favorite response.
 */
async function addFavoriteResponse(context: { request: Request }): Promise<Response> {
  favoriteRequestBody = await context.request.json();
  return HttpResponse.json({
    id: 9,
    folder_id: 3,
    article_id: 'article-1',
    db_name: 'fixture.sqlite',
    note: '',
    created_at: 2,
  });
}

/**
 * Verify favorites uses the shared sidebar, main landmark, and inner scroll container.
 */
async function rendersFavoritesWorkspace(): Promise<void> {
  favoriteFlowMocks.useVisiblePageList.mockClear();
  const user = userEvent.setup();
  server.use(
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/favorites/folders/3/articles/page', () =>
      favoriteArticlePageResponse([]),
    ),
  );

  renderFavoritesPage();

  expect(await screen.findByRole('heading', { name: '我的收藏' })).toBeInTheDocument();
  expect(screen.getByRole('complementary')).toBeInTheDocument();
  expect(screen.getByRole('main')).toHaveAttribute('id', 'main-content');
  expect(document.getElementById('results-scroll-container')).toBeInTheDocument();
  await waitFor(() =>
    expect(document.querySelector('[data-motion-folder-key="3"]')).not.toBeNull(),
  );
  expect(favoriteFlowMocks.useVisiblePageList).toHaveBeenCalledWith(
    expect.objectContaining({ scrollContainerId: 'results-scroll-container' }),
  );

  await user.click(screen.getByRole('button', { name: '打开收藏夹' }));
  const mobileSidebar = screen.getByRole('dialog', { name: '收藏夹' });
  await user.click(within(mobileSidebar).getByRole('button', { name: '新建收藏夹' }));
  expect(screen.getAllByRole('dialog', { name: '新建收藏夹' })).toHaveLength(1);
}

/**
 * Verify a successful mutation updates the immediate favorite state and query cache.
 */
async function updatesFavoriteCache(): Promise<void> {
  favoriteRequestBody = null;
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/favorites/check', emptyFavoriteResponse),
    http.post('http://localhost/api/favorites/folders/3/articles', addFavoriteResponse),
  );
  const user = userEvent.setup();
  const { queryClient } = renderWithQuery(
    <AuthProvider>
      <FavoriteButton articleId="article-1" dbName="fixture.sqlite" />
    </AuthProvider>,
  );

  await user.click(await screen.findByRole('button', { name: '收藏' }));
  await user.click(await screen.findByRole('button', { name: 'Reading' }));

  expect(await screen.findByRole('button', { name: '已收藏' })).toBeInTheDocument();
  expect(favoriteRequestBody).toEqual({
    article_id: 'article-1',
    db_name: 'fixture.sqlite',
    note: '',
  });
  await waitFor(() => {
    expect(
      queryClient.getQueryData<FavoriteCheck[]>(['fav-check', 'article-1', 'fixture.sqlite']),
    ).toEqual([{ folder_id: 3, folder_name: 'Reading' }]);
  });
}

/**
 * Verify removal preserves its target until the user confirms it.
 */
async function confirmsFavoriteRemoval(): Promise<void> {
  removeRequestCount = 0;
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/favorites/check', existingFavoriteResponse),
    http.delete(
      'http://localhost/api/favorites/folders/3/articles/article-1',
      removeFavoriteResponse,
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(
    <AuthProvider>
      <FavoriteButton articleId="article-1" dbName="fixture.sqlite" initialFolderIds={[3]} />
    </AuthProvider>,
  );

  await user.click(await screen.findByRole('button', { name: '已收藏' }));
  const folderButton = await screen.findByRole('button', { name: 'Reading' });
  await user.click(folderButton);
  expect(removeRequestCount).toBe(0);
  expect(screen.getByRole('alertdialog', { name: '移除收藏？' })).toHaveTextContent('Reading');

  await user.click(screen.getByRole('button', { name: '取消' }));
  expect(removeRequestCount).toBe(0);
  const favoriteTrigger = screen.getByRole('button', { name: '已收藏' });
  await waitFor(() => expect(favoriteTrigger).toHaveFocus());

  await user.click(favoriteTrigger);
  await user.click(await screen.findByRole('button', { name: 'Reading' }));
  await user.click(screen.getByRole('button', { name: '确认移除' }));
  await waitFor(() => expect(removeRequestCount).toBe(1));
  expect(await screen.findByRole('button', { name: '收藏' })).toBeInTheDocument();
}

/**
 * Verify a folder is deleted only after its identity is confirmed.
 */
async function confirmsFolderDeletion(): Promise<void> {
  let isFolderDeleted = false;
  let deleteRequestCount = 0;
  let releaseDelete = (): void => undefined;
  const deleteGate = new Promise<void>((resolve) => {
    releaseDelete = resolve;
  });
  server.use(
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json(
        isFolderDeleted
          ? []
          : [{ id: 3, name: 'Reading', is_tracking: false, article_count: 1, created_at: 1 }],
      ),
    ),
    http.get('http://localhost/api/favorites/folders/3/articles/page', () =>
      favoriteArticlePageResponse([favoriteArticleFixture(1)]),
    ),
    http.delete('http://localhost/api/favorites/folders/3', async () => {
      deleteRequestCount += 1;
      await deleteGate;
      isFolderDeleted = true;
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  await user.click(await screen.findByRole('button', { name: '删除收藏夹 Reading' }));
  expect(deleteRequestCount).toBe(0);
  expect(screen.getByRole('alertdialog', { name: '删除收藏夹？' })).toHaveTextContent('Reading');
  await user.click(screen.getByRole('button', { name: '确认删除' }));

  await waitFor(() => expect(deleteRequestCount).toBe(1));
  expect(screen.getAllByText('Reading').length).toBeGreaterThan(0);
  expect(document.querySelector('[data-motion-folder-key="3"]')).not.toBeNull();
  expect(screen.getByRole('alertdialog', { name: '删除收藏夹？' })).toBeInTheDocument();
  releaseDelete();
  expect(await screen.findByText('暂无收藏夹，点击 + 创建')).toBeInTheDocument();
  await waitFor(() => expect(document.querySelector('[data-motion-folder-key="3"]')).toBeNull());
}

/**
 * Verify a single loaded article remains until removal is confirmed.
 */
async function confirmsSingleFavoriteRemoval(): Promise<void> {
  let articles = [favoriteArticleFixture(1)];
  let removeCount = 0;
  let releaseFailedRemoval = (): void => undefined;
  const failedRemovalGate = new Promise<void>((resolve) => {
    releaseFailedRemoval = resolve;
  });
  server.use(
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json([
        {
          id: 3,
          name: 'Reading',
          is_tracking: false,
          article_count: articles.length,
          created_at: 1,
        },
      ]),
    ),
    http.get('http://localhost/api/favorites/folders/3/articles/page', () =>
      favoriteArticlePageResponse(articles),
    ),
    http.delete('http://localhost/api/favorites/folders/3/articles/article-1', async () => {
      removeCount += 1;
      if (removeCount === 1) {
        await failedRemovalGate;
        return HttpResponse.json({ detail: 'temporary removal failure' }, { status: 503 });
      }
      articles = [];
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  expect(await screen.findByText('Article 1')).toBeInTheDocument();
  expect(document.querySelector('[data-motion-favorite-key="1"]')).not.toBeNull();
  await user.click(await screen.findByRole('button', { name: '移除收藏' }));
  expect(removeCount).toBe(0);
  expect(screen.getByRole('alertdialog', { name: '移除收藏？' })).toHaveTextContent('Article 1');
  await user.click(screen.getByRole('button', { name: '确认移除' }));

  await waitFor(() => expect(removeCount).toBe(1));
  expect(screen.getByText('Article 1')).toBeInTheDocument();
  expect(document.querySelector('[data-motion-favorite-key="1"]')).not.toBeNull();
  releaseFailedRemoval();
  expect(await screen.findByRole('alert')).toHaveTextContent('temporary removal failure');
  expect(screen.getByText('Article 1')).toBeInTheDocument();
  expect(document.querySelector('[data-motion-favorite-key="1"]')).not.toBeNull();
  await user.click(screen.getByRole('button', { name: '确认移除' }));

  await waitFor(() => expect(removeCount).toBe(2));
  expect(await screen.findByText('此收藏夹为空')).toBeInTheDocument();
  await waitFor(() => expect(document.querySelector('[data-motion-favorite-key="1"]')).toBeNull());
}

/**
 * Verify bulk removal snapshots and submits the selected article identities.
 */
async function confirmsBulkFavoriteRemoval(): Promise<void> {
  let articles = [favoriteArticleFixture(1), favoriteArticleFixture(2)];
  let requestBody: unknown = null;
  server.use(
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json([
        {
          id: 3,
          name: 'Reading',
          is_tracking: false,
          article_count: articles.length,
          created_at: 1,
        },
      ]),
    ),
    http.get('http://localhost/api/favorites/folders/3/articles/page', () =>
      favoriteArticlePageResponse(articles),
    ),
    http.post(
      'http://localhost/api/favorites/folders/3/articles/bulk-remove',
      async ({ request }) => {
        requestBody = await request.json();
        articles = [];
        return HttpResponse.json({ count: 2 });
      },
    ),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  await user.click(await screen.findByRole('checkbox', { name: '选择当前已加载文章' }));
  await user.click(screen.getByRole('button', { name: '删除所选' }));
  expect(requestBody).toBeNull();
  expect(screen.getByRole('alertdialog', { name: '移除所选收藏？' })).toHaveTextContent('2 篇');
  await user.click(screen.getByRole('button', { name: '确认移除' }));

  await waitFor(() =>
    expect(requestBody).toEqual({
      articles: [
        { article_id: 'article-1', db_name: 'fixture.sqlite' },
        { article_id: 'article-2', db_name: 'fixture.sqlite' },
      ],
    }),
  );
  expect(await screen.findByText('此收藏夹为空')).toBeInTheDocument();
}

/**
 * Verify missing and unavailable metadata stay actionable with distinct explanations.
 */
async function rendersFavoriteMetadataAvailability(): Promise<void> {
  server.use(
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json([
        { id: 3, name: 'Reading', is_tracking: false, article_count: 2, created_at: 1 },
        { id: 4, name: 'Archive', is_tracking: false, article_count: 0, created_at: 2 },
      ]),
    ),
    http.get('http://localhost/api/favorites/folders/3/articles/page', () =>
      favoriteArticlePageResponse([
        {
          ...favoriteArticleFixture(1),
          metadata_status: 'missing',
          title: null,
          abstract: null,
        },
        {
          ...favoriteArticleFixture(2),
          metadata_status: 'unavailable',
          title: null,
          abstract: null,
        },
      ]),
    ),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  expect(await screen.findByText(/来源数据库或文章已不存在。收藏仍保留/)).toBeInTheDocument();
  expect(screen.getByText(/文章元数据暂时无法读取，请稍后重试/)).toBeInTheDocument();
  const missingCheckbox = screen.getByRole('checkbox', { name: '选择文章 article-1' });
  expect(missingCheckbox).toBeEnabled();
  expect(screen.getAllByRole('button', { name: '移除收藏' })).toHaveLength(2);
  expect(screen.getByRole('button', { name: '导出引用' })).toBeEnabled();

  await user.click(missingCheckbox);
  const targetFolderSelect = screen.getByRole('combobox', { name: '选择目标收藏夹' });
  targetFolderSelect.focus();
  await user.keyboard('{ArrowDown}{Enter}');
  expect(screen.getByRole('button', { name: '移动所选' })).toBeEnabled();
}

/**
 * Verify folder creation, rename, tracking selection, and export format controls.
 */
async function managesFoldersAndExportFormats(): Promise<void> {
  const folders = [
    { id: 3, name: 'Reading', is_tracking: false, article_count: 0, created_at: 1 },
    { id: 4, name: 'Archive', is_tracking: false, article_count: 0, created_at: 2 },
  ];
  const createPayloads: unknown[] = [];
  const renamePayloads: unknown[] = [];
  const trackingPayloads: unknown[] = [];
  server.use(
    http.get('http://localhost/api/favorites/folders', () => HttpResponse.json(folders)),
    http.get('http://localhost/api/favorites/folders/:folderId/articles/page', () =>
      favoriteArticlePageResponse([]),
    ),
    http.post('http://localhost/api/favorites/folders', async ({ request }) => {
      const payload = (await request.json()) as { is_tracking: boolean; name: string };
      createPayloads.push(payload);
      folders.push({
        id: 5,
        name: payload.name,
        is_tracking: payload.is_tracking,
        article_count: 0,
        created_at: 3,
      });
      return HttpResponse.json(folders[2]);
    }),
    http.put('http://localhost/api/favorites/folders/3', async ({ request }) => {
      const payload = (await request.json()) as { name: string };
      renamePayloads.push(payload);
      folders[0] = { ...folders[0], name: payload.name };
      return HttpResponse.json({ ok: true });
    }),
    http.put('http://localhost/api/favorites/tracking', async ({ request }) => {
      const payload = (await request.json()) as { folder_id: number };
      trackingPayloads.push(payload);
      for (const folder of folders) {
        folder.is_tracking = folder.id === payload.folder_id;
      }
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  expect(await screen.findByRole('button', { name: '导出引用' })).toBeEnabled();
  const exportSelect = screen.getAllByRole('combobox')[0];
  exportSelect.focus();
  await user.keyboard('{ArrowDown}');
  expect(await screen.findByRole('option', { name: 'RIS' })).toBeInTheDocument();
  await user.keyboard('{ArrowDown}{Enter}');
  expect(exportSelect).toHaveTextContent('RIS');
  exportSelect.focus();
  await user.keyboard('{ArrowDown}');
  expect(await screen.findByRole('option', { name: 'EndNote XML' })).toBeInTheDocument();
  await user.keyboard('{ArrowDown}{Enter}');
  expect(exportSelect).toHaveTextContent('EndNote XML');

  await user.click(screen.getByRole('button', { name: '新建收藏夹' }));
  await user.type(screen.getByLabelText('收藏夹名称'), '  New Folder  ');
  await user.click(screen.getByRole('button', { name: '创建' }));
  expect(await screen.findByText('New Folder')).toBeInTheDocument();
  expect(document.querySelector('[data-motion-folder-key="5"]')).not.toBeNull();
  expect(createPayloads).toEqual([{ name: 'New Folder', is_tracking: false }]);

  await user.click(screen.getByRole('button', { name: '重命名收藏夹 Reading' }));
  const renameInput = await screen.findByRole('textbox', { name: '重命名收藏夹 Reading' });
  expect(renameInput.closest('[data-motion-folder-mode="edit"]')).not.toBeNull();
  await user.clear(renameInput);
  await user.type(renameInput, 'Reviewed');
  await user.keyboard('{Enter}');
  expect(await screen.findAllByText('Reviewed')).not.toHaveLength(0);
  expect(renamePayloads).toEqual([{ name: 'Reviewed' }]);

  await user.click(screen.getByRole('button', { name: '设 Archive 为追踪文件夹' }));
  await waitFor(() => expect(trackingPayloads).toEqual([{ folder_id: 4 }]));
  expect(await screen.findByText('追踪')).toBeInTheDocument();
}

/**
 * Verify a failed bulk move retains its selection and can retry to committed empty state.
 */
async function retriesFailedBulkMove(): Promise<void> {
  let articles = [favoriteArticleFixture(1), favoriteArticleFixture(2)];
  const requestBodies: unknown[] = [];
  server.use(
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json([
        {
          id: 3,
          name: 'Reading',
          is_tracking: false,
          article_count: articles.length,
          created_at: 1,
        },
        { id: 4, name: 'Archive', is_tracking: false, article_count: 0, created_at: 2 },
      ]),
    ),
    http.get('http://localhost/api/favorites/folders/3/articles/page', () =>
      favoriteArticlePageResponse(articles),
    ),
    http.post(
      'http://localhost/api/favorites/folders/3/articles/bulk-move',
      async ({ request }) => {
        requestBodies.push(await request.json());
        if (requestBodies.length === 1) {
          return HttpResponse.json({ detail: 'Move target unavailable' }, { status: 503 });
        }
        articles = [];
        return HttpResponse.json({ count: 2 });
      },
    ),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  await user.click(await screen.findByRole('checkbox', { name: '选择当前已加载文章' }));
  const targetFolderSelect = screen.getByRole('combobox', { name: '选择目标收藏夹' });
  targetFolderSelect.focus();
  await user.keyboard('{ArrowDown}');
  expect(await screen.findByRole('option', { name: 'Archive' })).toBeInTheDocument();
  await user.keyboard('{Enter}');
  await user.click(screen.getByRole('button', { name: '移动所选' }));

  const feedback = await screen.findByTestId('batch-feedback-announcement');
  expect(feedback).toHaveAttribute('role', 'alert');
  expect(feedback).toHaveTextContent('Move target unavailable');
  expect(document.querySelector('[data-motion-feedback-key="batch-error"]')).not.toBeNull();
  expect(screen.getByText('已选 2 篇')).toBeInTheDocument();
  expect(screen.getByRole('group', { name: '批量操作' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '移动所选' }));

  await waitFor(() => expect(requestBodies).toHaveLength(2));
  expect(requestBodies).toEqual([
    {
      target_folder_id: 4,
      articles: [
        { article_id: 'article-1', db_name: 'fixture.sqlite' },
        { article_id: 'article-2', db_name: 'fixture.sqlite' },
      ],
    },
    {
      target_folder_id: 4,
      articles: [
        { article_id: 'article-1', db_name: 'fixture.sqlite' },
        { article_id: 'article-2', db_name: 'fixture.sqlite' },
      ],
    },
  ]);
  expect(await screen.findByText('此收藏夹为空')).toBeInTheDocument();
}

beforeEach(() => {
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  });
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: vi.fn().mockReturnValue({ matches: false }),
  });
});

/** Verify the production hook follows server cursors even for a short first page. */
async function loadsFavoriteCursorPages(): Promise<void> {
  const cursors: Array<string | null> = [];
  server.use(
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/favorites/folders/3/articles/page', ({ request }) => {
      const params = new URL(request.url).searchParams;
      expect(params.has('offset')).toBe(false);
      cursors.push(params.get('cursor'));
      return params.get('cursor') === 'fixture-next'
        ? favoriteArticlePageResponse([favoriteArticleFixture(2)])
        : favoriteArticlePageResponse([favoriteArticleFixture(1)], 'fixture-next');
    }),
  );
  renderFavoritesPage();
  expect(await screen.findByText('Article 1', {}, { timeout: 5000 })).toBeInTheDocument();
  favoriteFlowMocks.useVisiblePageList.mockReturnValue({
    loadMoreRef: () => undefined,
    prefetchRef: () => undefined,
    visiblePages: 2,
  });
  const options = favoriteFlowMocks.useVisiblePageList.mock.lastCall as unknown as [
    { onFetchNextPage: () => void },
  ];
  await act(async () => options[0].onFetchNextPage());
  expect(await screen.findByText('Article 2', {}, { timeout: 5000 })).toBeInTheDocument();
  expect(screen.getByText('Article 1')).toBeInTheDocument();
  expect(cursors).toEqual([null, 'fixture-next']);
  expect(favoriteFlowMocks.useVisiblePageList).toHaveBeenLastCalledWith(
    expect.objectContaining({ hasNextPage: false }),
  );
}

describe('favorite mutation flow', () => {
  test('loads favorite pages using opaque cursors', loadsFavoriteCursorPages, 15000);
  test('renders favorites in the shared workspace', rendersFavoritesWorkspace);
  test('updates visible state and cached folder membership', updatesFavoriteCache);
  test('confirms a removal before mutating folder membership', confirmsFavoriteRemoval);
  test('confirms folder deletion before mutation', confirmsFolderDeletion);
  test('confirms one favorite removal before mutation', confirmsSingleFavoriteRemoval);
  test('confirms bulk favorite removal with a target snapshot', confirmsBulkFavoriteRemoval);
  test(
    'keeps missing and unavailable favorite metadata actionable',
    rendersFavoriteMetadataAvailability,
  );
  test('manages folders and export formats', managesFoldersAndExportFormats, 15_000);
  test('retries a failed bulk move', retriesFailedBulkMove);
});
