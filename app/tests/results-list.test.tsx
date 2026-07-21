/**
 * Result-list loading, content, pagination, failure, favorite, and stale-state coverage.
 */

import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { act, screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

const resultsListMocks = vi.hoisted(() => ({
  favoriteChecksByArticle: {
    '9001': [{ folder_id: 7, folder_name: 'Reading' }],
  } as Record<string, { folder_id: number; folder_name: string }[]>,
  isFavoriteStatePending: true,
  onFetchNextPage: null as (() => void) | null,
  searchParams: '',
  user: { id: 21, username: 'results_user', is_admin: false },
  visiblePages: 100,
}));

vi.mock('next/navigation', () => ({
  useSearchParams: () => new URLSearchParams(resultsListMocks.searchParams),
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({ user: resultsListMocks.user }),
}));

vi.mock('@/components/feature/use-visible-page-list', () => ({
  useVisiblePageList: (options: { onFetchNextPage?: () => void }) => {
    resultsListMocks.onFetchNextPage = options.onFetchNextPage ?? null;
    return {
      visiblePages: resultsListMocks.visiblePages,
      prefetchRef: vi.fn(),
      loadMoreRef: vi.fn(),
    };
  },
}));

vi.mock('@/components/feature/use-favorite-checks', () => ({
  useFavoriteChecks: () => ({
    favoriteChecksByArticle: resultsListMocks.favoriteChecksByArticle,
    isFavoriteStatePending: resultsListMocks.isFavoriteStatePending,
  }),
}));

vi.mock('@/components/feature/article-dialog-card', () => ({
  ArticleDialogCard: ({
    article,
    dbName,
    initialFolderIds,
    isFavoriteStatePending,
    preview,
    title,
  }: {
    article: { article_id: string };
    dbName: string;
    initialFolderIds: number[];
    isFavoriteStatePending: boolean;
    preview: React.ReactNode;
    title: React.ReactNode;
  }) => (
    <article
      data-testid={`result-${article.article_id}`}
      data-database={dbName}
      data-folder-ids={initialFolderIds.join(',')}
      data-favorite-pending={String(isFavoriteStatePending)}
    >
      <h2>{title}</h2>
      <p>{preview}</p>
    </article>
  ),
}));

import { ResultsList } from '@/components/feature/results-list';
import type { Article, ArticlePage } from '@/lib/api';
import { setSelectedDatabase } from '@/lib/selected-database';
import { createArticlePageScenario } from '@/tests/mocks/scenarios';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const SHARED_ARTICLE: Article = createArticlePageScenario().items[0];

/**
 * Build one deterministic article page.
 *
 * @param items - Article rows in the page.
 * @param options - Pagination metadata overrides.
 * @returns Article page response.
 */
function createArticlePage(
  items: Article[],
  options: {
    hasMore?: boolean;
    nextCursor?: string | null;
    total?: number | null;
  } = {},
): ArticlePage {
  return {
    items,
    page: {
      total: options.total ?? items.length,
      limit: 20,
      offset: 0,
      next_cursor: options.nextCursor ?? null,
      has_more: options.hasMore ?? false,
    },
  };
}

/**
 * Render the production result list with URL-backed filters.
 *
 * @param searchParams - Initial query string.
 * @returns Render result and query client.
 */
function renderResultsList(
  searchParams = '?q=Fixture&area=Medicine&journal_id=101&month_range=2024-01..2024-02',
) {
  resultsListMocks.searchParams = searchParams;
  return renderWithQuery(
    <NuqsTestingAdapter searchParams={searchParams} hasMemory>
      <ResultsList />
    </NuqsTestingAdapter>,
  );
}

/**
 * Verify loading resolves into typed content, totals, highlights, filters, and favorite state.
 */
async function rendersTypedResultContent(): Promise<void> {
  let resolveRequest: (() => void) | undefined;
  let requestUrl: URL | undefined;
  const requestGate = new Promise<void>((resolve) => {
    resolveRequest = resolve;
  });
  server.use(
    http.get('http://localhost/api/articles', async ({ request }) => {
      requestUrl = new URL(request.url);
      await requestGate;
      return HttpResponse.json(createArticlePage([SHARED_ARTICLE], { total: 12 }));
    }),
  );
  renderResultsList();

  expect(screen.getByRole('status', { name: '正在加载搜索结果' })).toBeInTheDocument();
  resolveRequest?.();

  const article = await screen.findByTestId('result-9001');
  expect(screen.getByText('共找到 12 条结果')).toBeInTheDocument();
  expect(within(article).getAllByText('Fixture')).toHaveLength(2);
  for (const highlight of within(article).getAllByText('Fixture')) {
    expect(highlight).toHaveClass('font-bold');
  }
  expect(article).toHaveAttribute('data-database', 'scenario.sqlite');
  expect(article).toHaveAttribute('data-folder-ids', '7');
  expect(article).toHaveAttribute('data-favorite-pending', 'true');
  expect(requestUrl?.searchParams.get('db')).toBe('scenario.sqlite');
  expect(requestUrl?.searchParams.get('q')).toBe('Fixture');
  expect(requestUrl?.searchParams.getAll('area')).toEqual(['Medicine']);
  expect(requestUrl?.searchParams.getAll('journal_id')).toEqual(['101']);
  expect(requestUrl?.searchParams.get('date_from')).toBe('2024-01-01');
  expect(requestUrl?.searchParams.get('date_to')).toBe('2024-02-29');
  expect(requestUrl?.searchParams.get('include_total')).toBe('1');
}

/**
 * Verify a first-page failure is visible and an explicit query retry can recover to empty data.
 */
async function recoversFirstPageFailureToEmptyState(): Promise<void> {
  let shouldFail = true;
  server.use(
    http.get('http://localhost/api/articles', () =>
      shouldFail
        ? HttpResponse.json({ detail: 'article index unavailable' }, { status: 503 })
        : HttpResponse.json(createArticlePage([], { total: 0 })),
    ),
  );
  const { queryClient } = renderResultsList('?q=missing');

  expect(await screen.findByRole('alert')).toHaveTextContent('article index unavailable');
  shouldFail = false;
  await queryClient.invalidateQueries({ queryKey: ['articles'] });

  expect(await screen.findByText('未找到文章。')).toBeInTheDocument();
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
}

/**
 * Verify cursor pages append progressively and a database change discards stale articles.
 */
async function appendsPagesAndClearsStaleDatabaseResults(): Promise<void> {
  const requestUrls: URL[] = [];
  const secondArticle: Article = {
    ...SHARED_ARTICLE,
    article_id: '9002',
    title: 'Second page article',
    abstract: 'Second page abstract',
  };
  const otherDatabaseArticle: Article = {
    ...SHARED_ARTICLE,
    article_id: 'other-1',
    title: 'Other database article',
    abstract: 'Other database abstract',
  };
  server.use(
    http.get('http://localhost/api/articles', ({ request }) => {
      const url = new URL(request.url);
      requestUrls.push(url);
      if (url.searchParams.get('db') === 'other.sqlite') {
        return HttpResponse.json(createArticlePage([otherDatabaseArticle], { total: 1 }));
      }
      if (url.searchParams.get('cursor') === 'page-two') {
        return HttpResponse.json(createArticlePage([secondArticle], { total: null }));
      }
      return HttpResponse.json(
        createArticlePage([SHARED_ARTICLE], {
          hasMore: true,
          nextCursor: 'page-two',
          total: 2,
        }),
      );
    }),
  );
  renderResultsList('');

  expect(await screen.findByTestId('result-9001')).toBeInTheDocument();
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });
  expect(await screen.findByTestId('result-9002')).toBeInTheDocument();
  expect(requestUrls).toHaveLength(2);
  expect(requestUrls[1].searchParams.get('cursor')).toBe('page-two');

  act(() => setSelectedDatabase('other.sqlite'));
  expect(await screen.findByTestId('result-other-1')).toBeInTheDocument();
  await waitFor(() => expect(screen.queryByTestId('result-9001')).not.toBeInTheDocument());
  expect(screen.queryByTestId('result-9002')).not.toBeInTheDocument();
}

/**
 * Verify a later-page transport failure is not presented as a successful partial result.
 */
async function rejectsLaterPageFailure(): Promise<void> {
  server.use(
    http.get('http://localhost/api/articles', ({ request }) => {
      if (new URL(request.url).searchParams.has('cursor')) {
        return HttpResponse.json({ detail: 'second page unavailable' }, { status: 503 });
      }
      return HttpResponse.json(
        createArticlePage([SHARED_ARTICLE], {
          hasMore: true,
          nextCursor: 'page-two',
          total: 2,
        }),
      );
    }),
  );
  renderResultsList('');

  expect(await screen.findByTestId('result-9001')).toBeInTheDocument();
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });

  expect(await screen.findByRole('alert')).toHaveTextContent('second page unavailable');
  expect(screen.queryByTestId('result-9001')).not.toBeInTheDocument();
}

/**
 * Verify a repeated pagination cursor fails without rendering partial results.
 */
async function rejectsRepeatedCursor(): Promise<void> {
  let requestCount = 0;
  const secondArticle: Article = {
    ...SHARED_ARTICLE,
    article_id: '9002',
    title: 'Repeated cursor article',
  };
  server.use(
    http.get('http://localhost/api/articles', ({ request }) => {
      requestCount += 1;
      const hasCursor = new URL(request.url).searchParams.has('cursor');
      return HttpResponse.json(
        createArticlePage(hasCursor ? [secondArticle] : [SHARED_ARTICLE], {
          hasMore: true,
          nextCursor: 'repeated-cursor',
          total: 2,
        }),
      );
    }),
  );
  renderResultsList('');

  expect(await screen.findByTestId('result-9001')).toBeInTheDocument();
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });
  await waitFor(() => expect(requestCount).toBe(2));

  expect(await screen.findByRole('alert')).toHaveTextContent('分页游标重复');
  expect(screen.queryByTestId('result-9001')).not.toBeInTheDocument();
  expect(screen.queryByTestId('result-9002')).not.toBeInTheDocument();
}

beforeEach(() => {
  setSelectedDatabase('scenario.sqlite');
  resultsListMocks.favoriteChecksByArticle = {
    '9001': [{ folder_id: 7, folder_name: 'Reading' }],
  };
  resultsListMocks.isFavoriteStatePending = true;
  resultsListMocks.onFetchNextPage = null;
  resultsListMocks.searchParams = '';
  resultsListMocks.visiblePages = 100;
});

describe('results list', () => {
  test('renders typed result content and favorite state', rendersTypedResultContent);
  test('recovers a first-page failure to an empty state', recoversFirstPageFailureToEmptyState);
  test(
    'appends pages and clears stale database results',
    appendsPagesAndClearsStaleDatabaseResults,
  );
  test('rejects a later-page failure without partial results', rejectsLaterPageFailure);
  test('rejects a repeated cursor without partial results', rejectsRepeatedCursor);
});
