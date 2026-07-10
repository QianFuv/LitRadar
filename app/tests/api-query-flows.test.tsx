/**
 * Query serialization and infinite pagination behavior coverage.
 */

import { useInfiniteQuery } from '@tanstack/react-query';
import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vitest';

import { getNextArticlePageParam } from '@/components/feature/results-list';
import { getArticles, type ArticlePage } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let capturedArticleUrl = '';
let paginationRequestCount = 0;

/**
 * Capture one article request and return an empty page.
 *
 * @param context - MSW request context.
 * @returns Empty article page response.
 */
function captureArticleRequest(context: { request: Request }): Response {
  capturedArticleUrl = context.request.url;
  return HttpResponse.json({
    items: [],
    page: { total: 0, limit: 20, offset: 0, next_cursor: null, has_more: false },
  });
}

/**
 * Return two deterministic cursor pages.
 *
 * @param context - MSW request context.
 * @returns First or second article page.
 */
function paginatedArticleResponse(context: { request: Request }): Response {
  paginationRequestCount += 1;
  const cursor = new URL(context.request.url).searchParams.get('cursor');
  if (cursor === 'page-two') {
    return HttpResponse.json({
      items: [{ article_id: 'article-2', title: 'Second page' }],
      page: { total: null, limit: 20, offset: 0, next_cursor: null, has_more: false },
    });
  }
  return HttpResponse.json({
    items: [{ article_id: 'article-1', title: 'First page' }],
    page: { total: 2, limit: 20, offset: 0, next_cursor: 'page-two', has_more: true },
  });
}

/**
 * Render the production article fetcher through an infinite-query harness.
 *
 * @returns Pagination test UI.
 */
function PaginationHarness() {
  const query = useInfiniteQuery<ArticlePage, Error>({
    queryKey: ['pagination-harness'],
    queryFn: ({ pageParam }) =>
      getArticles(
        new URLSearchParams('q=systems'),
        pageParam as string | null,
        true,
        'fixture.sqlite',
      ),
    initialPageParam: null,
    getNextPageParam: getNextArticlePageParam,
  });

  return (
    <div>
      {query.data?.pages
        .flatMap((page) => page.items)
        .map((article) => (
          <span key={article.article_id}>{article.title}</span>
        ))}
      <button
        type="button"
        disabled={!query.hasNextPage || query.isFetchingNextPage}
        onClick={() => void query.fetchNextPage()}
      >
        Load more
      </button>
    </div>
  );
}

/**
 * Verify repeated filters, cursor, database, and total flags serialize correctly.
 */
async function serializesArticleQuery(): Promise<void> {
  server.use(http.get('http://localhost/api/articles', captureArticleRequest));
  const filters = new URLSearchParams();
  filters.append('area', 'systems');
  filters.append('area', 'security');
  filters.set('q', 'rust async');

  await getArticles(filters, 'cursor-token', true, 'fixture.sqlite');

  const url = new URL(capturedArticleUrl);
  expect(url.searchParams.getAll('area')).toEqual(['systems', 'security']);
  expect(url.searchParams.get('q')).toBe('rust async');
  expect(url.searchParams.get('cursor')).toBe('cursor-token');
  expect(url.searchParams.get('include_total')).toBe('1');
  expect(url.searchParams.get('db')).toBe('fixture.sqlite');
}

/**
 * Verify cursor pagination appends the next page and then stops.
 */
async function loadsInfinitePages(): Promise<void> {
  paginationRequestCount = 0;
  server.use(http.get('http://localhost/api/articles', paginatedArticleResponse));
  const user = userEvent.setup();

  renderWithQuery(<PaginationHarness />);

  expect(await screen.findByText('First page')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Load more' }));
  expect(await screen.findByText('Second page')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Load more' })).toBeDisabled();
  expect(paginationRequestCount).toBe(2);
}

describe('article query flows', () => {
  test('serializes filters and cursor parameters', serializesArticleQuery);
  test('loads cursor pages through an infinite query', loadsInfinitePages);
});
