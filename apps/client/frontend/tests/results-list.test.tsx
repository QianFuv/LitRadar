/**
 * Result-list loading, content, pagination, failure, favorite, and stale-state coverage.
 */

import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vite-plus/test';

const resultsListMocks = vi.hoisted(() => ({
  favoriteChecksByArticle: Object.fromEntries([
    ['9223361829786370983', [{ folder_id: 7, folder_name: 'Reading' }]],
  ]),
  isFavoriteStatePending: true,
  onFetchNextPage: null as (() => void) | null,
  user: { id: 21, username: 'results_user', is_admin: false },
  visiblePages: 100,
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
import { articlePagesQueryKey, type Article, type ArticlePage } from '@/lib/api';
import { apiPathKey } from '@/lib/api/query';
import { setSelectedDatabase } from '@/lib/selected-database';
import { useUpdateWorkspaceSearch } from '@/lib/search-params';
import { createArticlePageScenario } from '@/tests/mocks/scenarios';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const SHARED_ARTICLE: Article = createArticlePageScenario().items[0];

/** Verify obsolete filter queries and unmounted lists abort their network requests. */
async function abortsObsoleteArticleRequests(): Promise<void> {
  const signals: AbortSignal[] = [];
  let releaseRequests: (() => void) | undefined;
  const pending = new Promise<void>((resolve) => {
    releaseRequests = resolve;
  });
  server.use(
    http.get('http://localhost/api/articles', async ({ request }) => {
      signals.push(request.signal);
      await pending;
      return HttpResponse.json(
        createArticlePage([{ ...SHARED_ARTICLE, title: 'Obsolete result' }]),
      );
    }),
  );
  const user = userEvent.setup();
  const view = await renderWithQuery(
    <>
      <ResultsList />
      <RatingChangeControl />
    </>,
    { url: '/?abs_rating=4*' },
  );
  try {
    await waitFor(() => expect(signals).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Change rating' }));
    await waitFor(() => expect(signals).toHaveLength(2));
    expect(signals[0].aborted).toBe(true);
    expect(signals[1].aborted).toBe(false);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    view.unmount();
    expect(signals[1].aborted).toBe(true);
    await act(async () => {
      releaseRequests?.();
      await pending;
    });
    expect(screen.queryByText('Obsolete result')).not.toBeInTheDocument();
    expect(
      view.queryClient
        .getQueryCache()
        .getAll()
        .every((query) => query.state.error === null),
    ).toBe(true);
  } finally {
    releaseRequests?.();
    view.unmount();
  }
}

/** Change only one rating system to exercise the real query-key transition. */
function RatingChangeControl() {
  const updateSearch = useUpdateWorkspaceSearch();
  return (
    <button
      type="button"
      onClick={() => void updateSearch((search) => ({ ...search, abs_rating: ['4'] }))}
    >
      Change rating
    </button>
  );
}

/** Change the query without unmounting article rows already available in cache. */
function HighlightQueryChangeControl() {
  const updateSearch = useUpdateWorkspaceSearch();
  return (
    <button
      type="button"
      onClick={() => void updateSearch((search) => ({ ...search, q: 'Updated' }))}
    >
      Change highlight query
    </button>
  );
}

/** Verify every continuation retains ratings and a changed grade starts a clean first page. */
async function retainsRatingsAcrossPagesAndResetsTheirCache(): Promise<void> {
  const requests: URL[] = [];
  server.use(
    http.get('http://localhost/api/articles', ({ request }) => {
      const url = new URL(request.url);
      requests.push(url);
      if (url.searchParams.get('abs_rating') === '4') {
        return HttpResponse.json(
          createArticlePage([
            { ...SHARED_ARTICLE, article_id: '9223361829786309004', title: 'New grade' },
          ]),
        );
      }
      return url.searchParams.has('cursor')
        ? HttpResponse.json(
            createArticlePage([{ ...SHARED_ARTICLE, article_id: '9223361829786309005' }]),
          )
        : HttpResponse.json(
            createArticlePage([SHARED_ARTICLE], { hasMore: true, nextCursor: 'rating-page-two' }),
          );
    }),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <>
      <ResultsList />
      <RatingChangeControl />
    </>,
    { url: '/?abs_rating=4*&fms_rating=A&area=Medicine&q=Fixture' },
  );
  await waitFor(() => expect(screen.getByTestId('result-9223361829786370983')).toBeVisible());
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });
  await waitFor(() => expect(screen.getByTestId('result-9223361829786309005')).toBeVisible());
  for (const url of requests) {
    expect(url.searchParams.getAll('abs_rating')).toEqual(['4*']);
    expect(url.searchParams.getAll('fms_rating')).toEqual(['A']);
    expect(url.searchParams.getAll('area')).toEqual(['Medicine']);
  }
  expect(requests[1].searchParams.get('cursor')).toBe('rating-page-two');
  await user.click(screen.getByRole('button', { name: 'Change rating' }));
  await waitFor(() => expect(screen.getByTestId('result-9223361829786309004')).toBeVisible());
  expect(screen.queryByTestId('result-9223361829786370983')).not.toBeInTheDocument();
  expect(screen.queryByTestId('result-9223361829786309005')).not.toBeInTheDocument();
  expect(requests.at(-1)?.searchParams.has('cursor')).toBe(false);
  expect(requests.at(-1)?.searchParams.getAll('fms_rating')).toEqual(['A']);
  expect(requests.at(-1)?.searchParams.get('q')).toBe('Fixture');
}

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
      total: options.total === undefined ? items.length : options.total,
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
  searchParams = '?q=Fixture&area=Medicine&journal_id=9223361829786300101&month_range=2024-01..2024-02',
) {
  return renderWithQuery(<ResultsList />, { url: `/${searchParams}` });
}

/**
 * Verify loading resolves into typed content, highlights, filters, and favorite state.
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
  await renderResultsList();

  expect(screen.getByRole('status', { name: '正在加载搜索结果' })).toBeInTheDocument();
  expect(screen.getByTestId('results-state-loading')).toBeInTheDocument();
  expect(within(screen.getByTestId('results-state-loading')).queryByRole('status')).toBeNull();
  resolveRequest?.();

  const article = await screen.findByTestId('result-9223361829786370983');
  expect(screen.getByTestId('results-state-results')).toBeInTheDocument();
  expect(screen.getByRole('status', { name: '已加载 1 篇文章' })).toBeInTheDocument();
  expect(screen.getAllByRole('status')).toHaveLength(1);
  expect(within(article).getAllByText('Fixture')).toHaveLength(2);
  for (const highlight of within(article).getAllByText('Fixture')) {
    expect(highlight).toHaveAttribute('data-slot', 'search-highlight');
  }
  expect(article).toHaveAttribute('data-database', 'scenario.sqlite');
  expect(article).toHaveAttribute('data-folder-ids', '7');
  expect(article).toHaveAttribute('data-favorite-pending', 'false');
  expect(requestUrl?.searchParams.get('db')).toBe('scenario.sqlite');
  expect(requestUrl?.searchParams.get('q')).toBe('Fixture');
  expect(requestUrl?.searchParams.getAll('area')).toEqual(['Medicine']);
  expect(requestUrl?.searchParams.getAll('journal_id')).toEqual(['9223361829786300101']);
  expect(requestUrl?.searchParams.get('date_from')).toBe('2024-01-01');
  expect(requestUrl?.searchParams.get('date_to')).toBe('2024-02-29');
  expect(requestUrl?.searchParams.get('include_total')).toBe('false');
  expect(screen.queryByText(/共找到 .* 条结果/)).not.toBeInTheDocument();
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
  const { queryClient } = await renderResultsList('?q=missing');

  expect(await screen.findByRole('alert')).toHaveTextContent('article index unavailable');
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  expect(await screen.findByTestId('results-state-error')).toHaveTextContent('无法加载文章');
  shouldFail = false;
  await queryClient.invalidateQueries({ queryKey: apiPathKey('get', '/api/articles') });

  expect(await screen.findByText('未找到文章。')).toBeInTheDocument();
  expect(screen.getByTestId('results-state-empty')).toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent('请尝试调整搜索词、数据库或筛选条件。');
  expect(screen.getAllByRole('status')).toHaveLength(1);
  expect(screen.queryByText(/共找到 .* 条结果/)).not.toBeInTheDocument();
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
}

/** Verify the result list renders no legacy sticky filter-summary wrapper. */
async function omitsEmptyFilterSummarySlot(): Promise<void> {
  server.use(
    http.get('http://localhost/api/articles', () =>
      HttpResponse.json(createArticlePage([SHARED_ARTICLE], { total: 1 })),
    ),
  );
  await renderResultsList('');

  expect(await screen.findByTestId('result-9223361829786370983')).toBeInTheDocument();
  expect(screen.queryByTestId('filter-summary-slot')).not.toBeInTheDocument();
}

/**
 * Verify cursor pages append progressively and a database change discards stale articles.
 */
async function appendsPagesAndClearsStaleDatabaseResults(): Promise<void> {
  const requestUrls: URL[] = [];
  const secondArticle: Article = {
    ...SHARED_ARTICLE,
    article_id: '9223361829786309002',
    title: 'Second page article',
    abstract: 'Second page abstract',
  };
  const otherDatabaseArticle: Article = {
    ...SHARED_ARTICLE,
    article_id: '9223361829786309003',
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
  await renderResultsList('');

  expect(await screen.findByTestId('result-9223361829786370983')).toBeInTheDocument();
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });
  expect(await screen.findByTestId('result-9223361829786309002')).toBeInTheDocument();
  expect(requestUrls).toHaveLength(2);
  expect(requestUrls[0].searchParams.get('include_total')).toBe('false');
  expect(requestUrls[1].searchParams.get('cursor')).toBe('page-two');
  expect(requestUrls[1].searchParams.get('include_total')).toBe('false');
  expect(screen.queryByText(/共找到 .* 条结果/)).not.toBeInTheDocument();

  act(() => setSelectedDatabase('other.sqlite'));
  expect(await screen.findByTestId('result-9223361829786309003')).toBeInTheDocument();
  await waitFor(() =>
    expect(screen.queryByTestId('result-9223361829786370983')).not.toBeInTheDocument(),
  );
  expect(screen.queryByTestId('result-9223361829786309002')).not.toBeInTheDocument();
  expect(requestUrls[2].searchParams.get('include_total')).toBe('false');
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
  await renderResultsList('');

  expect(await screen.findByTestId('result-9223361829786370983')).toBeInTheDocument();
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });

  expect(await screen.findByRole('alert')).toHaveTextContent('second page unavailable');
  await waitFor(() =>
    expect(screen.queryByTestId('result-9223361829786370983')).not.toBeInTheDocument(),
  );
}

/**
 * Verify a repeated pagination cursor fails without rendering partial results.
 */
async function rejectsRepeatedCursor(): Promise<void> {
  let requestCount = 0;
  const secondArticle: Article = {
    ...SHARED_ARTICLE,
    article_id: '9223361829786309002',
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
  await renderResultsList('');

  expect(await screen.findByTestId('result-9223361829786370983')).toBeInTheDocument();
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });
  await waitFor(() => expect(requestCount).toBe(2));

  expect(await screen.findByRole('alert')).toHaveTextContent('分页游标重复');
  await waitFor(() =>
    expect(screen.queryByTestId('result-9223361829786370983')).not.toBeInTheDocument(),
  );
  expect(screen.queryByTestId('result-9223361829786309002')).not.toBeInTheDocument();
}

beforeEach(() => {
  setSelectedDatabase('scenario.sqlite');
  resultsListMocks.favoriteChecksByArticle = {
    '9223361829786370983': [{ folder_id: 7, folder_name: 'Reading' }],
  };
  resultsListMocks.isFavoriteStatePending = true;
  resultsListMocks.onFetchNextPage = null;
  resultsListMocks.visiblePages = 100;
});

/** Verify favorite updates reuse highlights while article edits invalidate the projection. */
async function reusesHighlightsForFavoriteChanges(): Promise<void> {
  server.use(
    http.get('http://localhost/api/articles', () =>
      HttpResponse.json(createArticlePage([SHARED_ARTICLE])),
    ),
  );
  const view = await renderResultsList('?q=Fixture');
  await screen.findByTestId('result-9223361829786370983');
  const split = vi.spyOn(String.prototype, 'split');
  resultsListMocks.favoriteChecksByArticle = { '9223361829786370983': [] };
  resultsListMocks.isFavoriteStatePending = false;
  view.rerender(<ResultsList />);
  expect(screen.getByTestId('result-9223361829786370983')).toHaveAttribute('data-folder-ids', '');
  expect(screen.getByTestId('result-9223361829786370983')).toHaveAttribute(
    'data-favorite-pending',
    'false',
  );
  expect(
    split.mock.calls.filter(
      ([pattern]) => pattern instanceof RegExp && pattern.source.includes('Fixture'),
    ),
  ).toHaveLength(0);
  await act(async () => {
    const queryKey = view.queryClient
      .getQueryCache()
      .find({ queryKey: apiPathKey('get', '/api/articles'), exact: false })!.queryKey;
    view.queryClient.setQueryData(queryKey, {
      pages: [createArticlePage([{ ...SHARED_ARTICLE, title: 'Updated Fixture title' }])],
      pageParams: [null],
    });
  });
  expect(await screen.findByRole('heading', { name: 'Updated Fixture title' })).toBeInTheDocument();
  expect(
    split.mock.calls.some(
      ([pattern]) => pattern instanceof RegExp && pattern.source.includes('Fixture'),
    ),
  ).toBe(true);
}

/** Verify appending a page highlights only new articles without rescanning earlier text. */
async function reusesHighlightsAcrossPages(): Promise<void> {
  const firstArticle = {
    ...SHARED_ARTICLE,
    title: 'First Fixture title',
    abstract: 'First Fixture abstract',
  };
  const secondArticle = {
    ...SHARED_ARTICLE,
    article_id: '9223361829786309002',
    title: 'Second Fixture title',
    abstract: 'Second Fixture abstract',
  };
  server.use(
    http.get('http://localhost/api/articles', ({ request }) =>
      HttpResponse.json(
        new URL(request.url).searchParams.has('cursor')
          ? createArticlePage([secondArticle])
          : createArticlePage([firstArticle], { hasMore: true, nextCursor: 'page-two' }),
      ),
    ),
  );
  await renderResultsList('?q=Fixture');
  await screen.findByTestId('result-9223361829786370983');
  const split = vi.spyOn(String.prototype, 'split');
  await act(async () => {
    resultsListMocks.onFetchNextPage?.();
  });
  const secondResult = await screen.findByTestId('result-9223361829786309002');
  expect(within(secondResult).getAllByText('Fixture')).toHaveLength(2);
  const highlightedTexts = split.mock.calls.flatMap(([pattern], index) =>
    pattern instanceof RegExp && pattern.source.includes('Fixture')
      ? [String(split.mock.contexts[index])]
      : [],
  );
  expect(highlightedTexts).not.toContain(firstArticle.title);
  expect(highlightedTexts).not.toContain(firstArticle.abstract);
  expect(highlightedTexts).toContain(secondArticle.title);
  expect(highlightedTexts).toContain(secondArticle.abstract);
}

/** Verify a new query invalidates highlights even when article identity and text are unchanged. */
async function refreshesHighlightsForQueryChanges(): Promise<void> {
  const article = {
    ...SHARED_ARTICLE,
    title: 'Fixture Updated title',
    abstract: 'Fixture Updated abstract',
  };
  server.use(
    http.get('http://localhost/api/articles', () =>
      HttpResponse.json(createArticlePage([article])),
    ),
  );
  const view = await renderWithQuery(
    <>
      <ResultsList />
      <HighlightQueryChangeControl />
    </>,
    { url: '/?q=Fixture' },
  );
  const result = await screen.findByTestId('result-9223361829786370983');
  expect(Array.from(result.querySelectorAll('span'), (span) => span.textContent)).toEqual([
    'Fixture',
    'Fixture',
  ]);
  view.queryClient.setQueryData(articlePagesQueryKey({ q: 'Updated' }, 'scenario.sqlite'), {
    pages: [createArticlePage([article])],
    pageParams: [null],
  });
  await userEvent.setup().click(screen.getByRole('button', { name: 'Change highlight query' }));
  await waitFor(() =>
    expect(Array.from(result.querySelectorAll('span'), (span) => span.textContent)).toEqual([
      'Updated',
      'Updated',
    ]),
  );
  expect(screen.getByTestId('result-9223361829786370983')).toBe(result);
}

describe('results list', () => {
  test(
    'refreshes mounted article highlights when the query changes',
    refreshesHighlightsForQueryChanges,
  );
  test('reuses earlier article highlights when appending pages', reusesHighlightsAcrossPages);
  test(
    'reuses highlights for favorite updates and refreshes changed articles',
    reusesHighlightsForFavoriteChanges,
  );
  test('aborts obsolete filter and unmounted article requests', abortsObsoleteArticleRequests);
  test(
    'keeps ratings on later pages and resets results when a grade changes',
    retainsRatingsAcrossPagesAndResetsTheirCache,
  );
  test('renders typed result content and favorite state', rendersTypedResultContent);
  test('recovers a first-page failure to an empty state', recoversFirstPageFailureToEmptyState);
  test('omits an empty filter summary slot', omitsEmptyFilterSummarySlot);
  test(
    'appends pages and clears stale database results',
    appendsPagesAndClearsStaleDatabaseResults,
  );
  test('rejects a later-page failure without partial results', rejectsLaterPageFailure);
  test('rejects a repeated cursor without partial results', rejectsRepeatedCursor);
});
