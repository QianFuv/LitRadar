/**
 * Favorite citation download behavior through the production page and API client.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { FavoritesPageContent } from '@/components/favorites/favorites-page-content';
import { AuthProvider } from '@/lib/auth-context';
import type { FavoriteArticleItem } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const favoriteExportMocks = vi.hoisted(() => ({
  anchorClick: vi.fn(),
  createObjectUrl: vi.fn(() => 'blob:favorite-export'),
  revokeObjectUrl: vi.fn(),
  useVisiblePageList: vi.fn(() => ({
    loadMoreRef: () => undefined,
    prefetchRef: () => undefined,
    visiblePages: 1,
  })),
}));

vi.mock('@/components/feature/use-visible-page-list', () => ({
  useVisiblePageList: favoriteExportMocks.useVisiblePageList,
}));

vi.mock('@/components/feature/article-dialog-card', () => ({
  ArticleDialogCard: ({ article }: { article: FavoriteArticleItem }) => <div>{article.title}</div>,
}));

const originalCreateObjectUrl = URL.createObjectURL;
const originalRevokeObjectUrl = URL.revokeObjectURL;

/**
 * Install the common authenticated favorites-page handlers.
 */
function installFavoritePageHandlers(): void {
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ id: 21, username: 'favorite_user', is_admin: false }),
    ),
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json([
        { id: 3, name: 'Reading', is_tracking: false, article_count: 0, created_at: 1 },
      ]),
    ),
    http.get('http://localhost/api/favorites/folders/3/articles', () => HttpResponse.json([])),
  );
}

/**
 * Render the favorites page with its authenticated and URL-state providers.
 */
function renderFavoritesPage(): void {
  renderWithQuery(
    <AuthProvider>
      <NuqsTestingAdapter searchParams="?folder=3">
        <FavoritesPageContent userId={21} />
      </NuqsTestingAdapter>
    </AuthProvider>,
  );
}

/**
 * Verify the server filename is used once and all temporary browser resources are released.
 */
async function downloadsWithServerFilename(): Promise<void> {
  installFavoritePageHandlers();
  server.use(
    http.get('http://localhost/api/favorites/folders/3/export', ({ request }) => {
      expect(new URL(request.url).searchParams.get('format')).toBe('bibtex');
      return new HttpResponse('@article{fixture}', {
        headers: {
          'Content-Disposition': 'attachment; filename="Reading.bib"',
          'Content-Type': 'application/x-bibtex; charset=utf-8',
        },
      });
    }),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  await user.click(await screen.findByRole('button', { name: '导出引用' }));

  const feedback = await screen.findByTestId('export-feedback-announcement');
  expect(feedback).toHaveAttribute('role', 'status');
  expect(feedback).toHaveTextContent('已导出 Reading.bib。');
  expect(document.querySelector('[data-motion-feedback-key="export-success"]')).not.toBeNull();
  expect(favoriteExportMocks.createObjectUrl).toHaveBeenCalledTimes(1);
  expect(favoriteExportMocks.anchorClick).toHaveBeenCalledWith({
    download: 'Reading.bib',
    href: 'blob:favorite-export',
  });
  expect(favoriteExportMocks.revokeObjectUrl).toHaveBeenCalledWith('blob:favorite-export');
  expect(document.querySelector('a[download]')).toBeNull();
}

/**
 * Verify a missing attachment filename falls back to the selected format extension.
 */
async function downloadsWithFallbackFilename(): Promise<void> {
  installFavoritePageHandlers();
  server.use(
    http.get(
      'http://localhost/api/favorites/folders/3/export',
      () => new HttpResponse('@article{fixture}'),
    ),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  await user.click(await screen.findByRole('button', { name: '导出引用' }));

  const feedback = await screen.findByTestId('export-feedback-announcement');
  expect(feedback).toHaveAttribute('role', 'status');
  expect(feedback).toHaveTextContent('已导出 favorites.bib。');
  expect(favoriteExportMocks.anchorClick).toHaveBeenCalledWith({
    download: 'favorites.bib',
    href: 'blob:favorite-export',
  });
  expect(favoriteExportMocks.revokeObjectUrl).toHaveBeenCalledTimes(1);
}

/**
 * Verify pending and failed exports remain in place without creating a browser download.
 */
async function surfacesExportFailureInPlace(): Promise<void> {
  installFavoritePageHandlers();
  let releaseExport = (): void => undefined;
  const exportGate = new Promise<void>((resolve) => {
    releaseExport = resolve;
  });
  server.use(
    http.get('http://localhost/api/favorites/folders/3/export', async () => {
      await exportGate;
      return HttpResponse.json(
        { detail: '收藏夹引用数量超过导出上限' },
        { status: 413, headers: { 'X-Request-Id': 'request-export-limit' } },
      );
    }),
  );
  const user = userEvent.setup();
  renderFavoritesPage();

  await user.click(await screen.findByRole('button', { name: '导出引用' }));
  expect(await screen.findByRole('button', { name: '导出中…' })).toBeDisabled();
  releaseExport();

  const feedback = await screen.findByTestId('export-feedback-announcement');
  expect(feedback).toHaveAttribute('role', 'alert');
  expect(feedback).toHaveTextContent('收藏夹引用数量超过导出上限');
  expect(document.querySelector('[data-motion-feedback-key="export-error"]')).not.toBeNull();
  expect(screen.getByRole('heading', { name: '我的收藏' })).toBeInTheDocument();
  expect(favoriteExportMocks.createObjectUrl).not.toHaveBeenCalled();
  expect(favoriteExportMocks.anchorClick).not.toHaveBeenCalled();
  expect(favoriteExportMocks.revokeObjectUrl).not.toHaveBeenCalled();
  await waitFor(() => expect(screen.getByRole('button', { name: '导出引用' })).toBeEnabled());
}

beforeEach(() => {
  favoriteExportMocks.anchorClick.mockReset();
  favoriteExportMocks.createObjectUrl.mockReset().mockReturnValue('blob:favorite-export');
  favoriteExportMocks.revokeObjectUrl.mockReset();
  Object.defineProperty(URL, 'createObjectURL', {
    configurable: true,
    value: favoriteExportMocks.createObjectUrl,
  });
  Object.defineProperty(URL, 'revokeObjectURL', {
    configurable: true,
    value: favoriteExportMocks.revokeObjectUrl,
  });
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: vi.fn().mockReturnValue({ matches: false }),
  });
  vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
    this: HTMLAnchorElement,
  ) {
    favoriteExportMocks.anchorClick({ download: this.download, href: this.href });
  });
});

afterEach(() => {
  Object.defineProperty(URL, 'createObjectURL', {
    configurable: true,
    value: originalCreateObjectUrl,
  });
  Object.defineProperty(URL, 'revokeObjectURL', {
    configurable: true,
    value: originalRevokeObjectUrl,
  });
  vi.restoreAllMocks();
});

describe('favorite citation export', () => {
  test(
    'downloads once with the server filename and releases the object URL',
    downloadsWithServerFilename,
  );
  test('uses a safe fallback filename when the response omits one', downloadsWithFallbackFilename);
  test('disables the action and surfaces backend failures in place', surfacesExportFailureInPlace);
});
