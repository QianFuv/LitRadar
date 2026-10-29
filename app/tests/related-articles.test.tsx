/**
 * Related articles: backend-ranked list and in-reader navigation.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import type { Article } from '@/lib/api';
import { AuthProvider, useAuth } from '@/lib/auth-context';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

vi.mock('next/navigation', () => ({
  usePathname: () => '/',
  useSearchParams: () => new URLSearchParams(),
}));

vi.mock('react-intersection-observer', () => ({
  useInView: () => ({ ref: () => undefined, inView: true }),
}));

const SOURCE: Article = {
  article_id: 'source',
  journal_id: 'journal-1',
  title: 'Graph Neural Networks for Traffic Forecasting',
  abstract: 'Spatiotemporal traffic prediction.',
  authors: ['Ada Lovelace'],
  journal_title: 'Journal of Tests',
  date: '2024-05-17',
};

/**
 * Build a related article.
 *
 * @param id - Article identifier.
 * @param title - Article title.
 * @returns Related article.
 */
function relatedArticle(id: string, title: string): Article {
  return {
    article_id: id,
    journal_id: 'journal-2',
    title,
    abstract: '',
    authors: ['Someone Else'],
    journal_title: 'Other Journal',
    date: '2024-01-01',
  };
}

/** Mount the article card once the server session is known. */
function AuthenticatedCard() {
  const { user, loading } = useAuth();
  return loading || !user ? null : <ArticleDialogCard article={SOURCE} dbName="fixture.sqlite" />;
}

/** Verify one ranked request, backend order, and opening a related article in place with a way back. */
async function opensRelatedArticleInPlace(): Promise<void> {
  const relatedRequests: URL[] = [];
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ id: 21, username: 'reader', is_admin: false }),
    ),
    http.get('http://localhost/api/articles/:articleId/access', () =>
      HttpResponse.json({
        abstract_page: {
          available: true,
          label: '查看摘要页',
          requires_login: false,
          message: null,
        },
        fulltext: { available: true, label: '获取全文', requires_login: false, message: null },
      }),
    ),
    http.get('http://localhost/api/articles/:articleId/related', ({ request, params }) => {
      relatedRequests.push(new URL(request.url));
      return HttpResponse.json(
        params.articleId === 'source'
          ? [
              relatedArticle('related-1', 'Traffic forecasting with graph neural networks'),
              relatedArticle('related-2', 'Protein folding dynamics'),
            ]
          : [],
      );
    }),
    http.get('http://localhost/api/favorites/check', () =>
      HttpResponse.json([{ folder_id: 3, folder_name: 'Reading' }]),
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(
    <AuthProvider>
      <AuthenticatedCard />
    </AuthProvider>,
  );

  await user.click(await screen.findByRole('button', { name: /^查看文章详情：/ }));
  const dialog = await screen.findByRole('dialog');
  const related = await within(dialog).findByRole('region', { name: '相关文章' });
  const options = await within(related).findAllByRole('button');
  expect(options.map((option) => option.textContent)).toEqual([
    expect.stringContaining('Traffic forecasting with graph neural networks'),
    expect.stringContaining('Protein folding dynamics'),
  ]);
  expect(related).not.toHaveTextContent('共同关键词');

  expect(relatedRequests).toHaveLength(1);
  expect(relatedRequests[0]?.searchParams.get('db')).toBe('fixture.sqlite');
  expect(relatedRequests[0]?.searchParams.get('limit')).toBe('6');

  await user.click(options[0]!);
  expect(
    await within(dialog).findByRole('heading', {
      name: /Traffic forecasting with graph neural networks/,
    }),
  ).toBeInTheDocument();
  expect(await within(dialog).findByRole('button', { name: '已收藏' })).toBeInTheDocument();
  expect(await within(dialog).findByText('暂未找到相关文章。')).toBeInTheDocument();

  await user.click(within(dialog).getByRole('button', { name: '返回上一篇' }));
  await waitFor(() =>
    expect(
      within(dialog).getByRole('heading', { name: /Graph Neural Networks for Traffic/ }),
    ).toBeInTheDocument(),
  );
  expect(within(dialog).queryByRole('button', { name: '返回上一篇' })).not.toBeInTheDocument();
}

beforeEach(() => {
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: () => undefined,
  });
});

describe('related articles', () => {
  test('opens a related article in place and returns', opensRelatedArticleInPlace, 15000);
});
