/**
 * Weekly-update summary, cursor pagination, query reset, and failure coverage.
 */

import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse, type HttpResponseResolver } from 'msw';
import { parseAsString, useQueryState } from 'nuqs';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { WeeklyUpdatesView } from '@/components/weekly/weekly-updates-view';
import { SELECTED_DATABASE_KEY, readSelectedDatabase } from '@/lib/api';
import type { WeeklyArticlePage, WeeklyUpdatesSummaryResponse } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery, type QueryRenderResult } from '@/tests/render';

type VisiblePageOptions = {
  loadedPages: number;
  hasNextPage?: boolean;
  isFetchingNextPage?: boolean;
  onFetchNextPage?: () => void;
  scrollContainerId?: string;
};

const weeklyViewMocks = vi.hoisted(() => ({
  onFetchNextPage: null as (() => void) | null,
  useVisiblePageList: vi.fn(),
}));

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({ user: { id: 31, username: 'weekly_user', is_admin: false } }),
}));

vi.mock('@/components/feature/article-dialog-card', () => ({
  ArticleDialogCard: ({ article }: { article: { article_id: string; title?: string | null } }) => (
    <article data-testid="weekly-article" data-article-id={article.article_id}>
      {article.title}
    </article>
  ),
}));

vi.mock('@/components/feature/use-favorite-checks', () => ({
  useFavoriteChecks: () => ({
    favoriteChecksByArticle: {},
    isFavoriteStatePending: false,
  }),
}));

vi.mock('@/components/feature/use-visible-page-list', () => ({
  useVisiblePageList: weeklyViewMocks.useVisiblePageList,
}));

const WEEKLY_SUMMARY_FIXTURE: WeeklyUpdatesSummaryResponse = {
  generated_at: '2026-07-08T23:59:59Z',
  window_start: '2026-07-01T23:59:59Z',
  window_end: '2026-07-08T23:59:59Z',
  databases: [
    {
      db_name: 'fixture.sqlite',
      generated_at: '2026-07-08T23:59:59Z',
      new_article_count: 4,
      journals: [
        {
          journal_id: '101',
          journal_title: 'Fixture Journal',
          new_article_count: 3,
        },
        {
          journal_id: '102',
          journal_title: 'Second Fixture Journal',
          new_article_count: 1,
        },
      ],
    },
    {
      db_name: 'other.sqlite',
      generated_at: '2026-07-08T23:59:59Z',
      new_article_count: 1,
      journals: [
        {
          journal_id: '201',
          journal_title: 'Other Journal',
          new_article_count: 1,
        },
      ],
    },
  ],
};

const weeklyArticleRequestUrls: URL[] = [];
let generalArticleRequestCount = 0;
let weeklySummaryRequestCount = 0;

/**
 * Display one nuqs string value for URL-state assertions.
 *
 * @param props - Query parameter and test identifier.
 * @returns Current query value.
 */
function QueryProbe({ parameter, testId }: { parameter: string; testId: string }) {
  const [value] = useQueryState(parameter, parseAsString);
  return <output data-testid={testId}>{value ?? ''}</output>;
}

/**
 * Build one weekly article page for a test response.
 *
 * @param items - Article rows in response order.
 * @param nextCursor - Cursor for the next page.
 * @param hasMore - Whether another page exists.
 * @returns Weekly article page.
 */
function weeklyPage(
  items: WeeklyArticlePage['items'],
  nextCursor: string | null = null,
  hasMore = false,
): WeeklyArticlePage {
  return {
    items,
    page: {
      total: null,
      limit: 50,
      offset: 0,
      next_cursor: nextCursor,
      has_more: hasMore,
    },
  };
}

/**
 * Install weekly summary/page handlers and an audit handler for the general article route.
 *
 * @param articleResolver - Weekly article endpoint response resolver.
 * @param summary - Optional summary response.
 */
function installWeeklyHandlers(
  articleResolver: HttpResponseResolver,
  summary: WeeklyUpdatesSummaryResponse = WEEKLY_SUMMARY_FIXTURE,
): void {
  server.use(
    http.get('http://localhost/api/weekly-updates/summary', () => {
      weeklySummaryRequestCount += 1;
      return HttpResponse.json(summary);
    }),
    http.get('http://localhost/api/meta/databases', () =>
      HttpResponse.json(['fixture.sqlite', 'other.sqlite']),
    ),
    http.get('http://localhost/api/weekly-updates/articles', (context) => {
      weeklyArticleRequestUrls.push(new URL(context.request.url));
      return articleResolver(context);
    }),
    http.get('http://localhost/api/articles', () => {
      generalArticleRequestCount += 1;
      return HttpResponse.json(weeklyPage([]));
    }),
  );
}

/**
 * Install deterministic article pages for every summary selection.
 */
function installDefaultWeeklyHandlers(): void {
  installWeeklyHandlers(({ request }) => {
    const url = new URL(request.url);
    const dbName = url.searchParams.get('db');
    const journalId = url.searchParams.get('journal_id');
    const query = url.searchParams.get('q');
    if (query === 'needle') {
      return HttpResponse.json(
        weeklyPage([{ article_id: 'search-1', journal_id: '101', title: 'Needle result' }]),
      );
    }
    if (dbName === 'other.sqlite') {
      return HttpResponse.json(
        weeklyPage([{ article_id: 'other-1', journal_id: '201', title: 'Other weekly article' }]),
      );
    }
    if (journalId === '102') {
      return HttpResponse.json(
        weeklyPage([
          { article_id: 'journal-2', journal_id: '102', title: 'Second journal article' },
        ]),
      );
    }
    return HttpResponse.json(
      weeklyPage([
        { article_id: 'weekly-1', journal_id: '101', title: 'Weekly first' },
        { article_id: 'weekly-2', journal_id: '101', title: 'Weekly second' },
        { article_id: 'weekly-3', journal_id: '101', title: 'Weekly third' },
      ]),
    );
  });
}

/**
 * Render the weekly page with matching Next and nuqs query snapshots.
 *
 * @param searchParams - Initial URL query string.
 * @returns Render result and query client.
 */
function renderWeeklyPage(searchParams: string): QueryRenderResult {
  return renderWithQuery(
    <NuqsTestingAdapter searchParams={searchParams} hasMemory>
      <WeeklyUpdatesView />
      <QueryProbe parameter="db" testId="weekly-db" />
    </NuqsTestingAdapter>,
  );
}

/**
 * Return rendered weekly article identifiers in visual order.
 *
 * @returns Article identifiers.
 */
function renderedArticleIds(): string[] {
  return screen
    .queryAllByTestId('weekly-article')
    .map((element) => element.getAttribute('data-article-id') ?? '');
}

/**
 * Verify the initial view performs only one summary and one bounded article request.
 */
async function loadsOneInitialBoundedPage(): Promise<void> {
  installDefaultWeeklyHandlers();
  renderWeeklyPage('?q=homepage-only&db=fixture.sqlite&journal=101');

  expect(await screen.findByText('Weekly first')).toBeInTheDocument();
  expect(weeklySummaryRequestCount).toBe(1);
  expect(weeklyArticleRequestUrls).toHaveLength(1);
  expect(generalArticleRequestCount).toBe(0);
  const request = weeklyArticleRequestUrls[0];
  expect(request.searchParams.get('db')).toBe('fixture.sqlite');
  expect(request.searchParams.get('journal_id')).toBe('101');
  expect(request.searchParams.get('window_end')).toBe(WEEKLY_SUMMARY_FIXTURE.window_end);
  expect(request.searchParams.get('limit')).toBe('50');
  expect(request.searchParams.get('q')).toBeNull();
  expect(request.searchParams.get('cursor')).toBeNull();
  expect(screen.getByRole('combobox', { name: '搜索文章' })).toHaveValue('');
  expect(screen.getByRole('complementary')).toBeInTheDocument();
  expect(screen.getByRole('main')).toHaveAttribute('id', 'main-content');
  expect(document.getElementById('results-scroll-container')).toBeInTheDocument();
  expect(document.querySelector('[data-weekly-state="ready"]')).not.toBeNull();
  expect(document.querySelector('[data-weekly-journal="101"]')).not.toBeNull();
  const articleState = document.querySelector('[data-weekly-article-state="results"]');
  expect(articleState).not.toBeNull();
  expect(screen.getByTestId('weekly-state-announcement')).toHaveAttribute('role', 'status');
  expect(screen.getAllByTestId('weekly-state-announcement')).toHaveLength(1);
  expect(within(articleState as HTMLElement).queryByRole('status')).toBeNull();
  expect(screen.getAllByTestId('weekly-article')[0].parentElement).toHaveAttribute(
    'data-weekly-article-state',
    'results',
  );
  expect(weeklyViewMocks.useVisiblePageList).toHaveBeenLastCalledWith(
    expect.objectContaining({
      loadedPages: 1,
      scrollContainerId: 'results-scroll-container',
    }),
  );
}

/**
 * Verify selecting another database resets the article request without changing homepage storage.
 */
async function resetsForDatabaseSelection(): Promise<void> {
  window.localStorage.setItem(SELECTED_DATABASE_KEY, 'homepage.sqlite');
  installDefaultWeeklyHandlers();
  const user = userEvent.setup();
  renderWeeklyPage('?db=fixture.sqlite&journal=101');

  expect(await screen.findByText('Weekly first')).toBeInTheDocument();
  const databaseSelect = screen
    .getAllByRole('combobox')
    .find((element) => element.getAttribute('data-slot') === 'select-trigger');
  expect(databaseSelect).toBeDefined();
  (databaseSelect as HTMLElement).focus();
  await user.keyboard('{ArrowDown}');
  expect(await screen.findByRole('option', { name: 'other.sqlite' })).toBeInTheDocument();
  await user.keyboard('{ArrowDown}{Enter}');

  await waitFor(() => expect(screen.getByTestId('weekly-db')).toHaveTextContent('other.sqlite'));
  expect(await screen.findByText('Other weekly article')).toBeInTheDocument();
  expect(document.querySelector('[data-weekly-journal="201"]')).not.toBeNull();
  expect(weeklyArticleRequestUrls).toHaveLength(2);
  const request = weeklyArticleRequestUrls[1];
  expect(request.searchParams.get('db')).toBe('other.sqlite');
  expect(request.searchParams.get('journal_id')).toBe('201');
  expect(request.searchParams.get('cursor')).toBeNull();
  expect(readSelectedDatabase()).toBe('homepage.sqlite');
}

/**
 * Verify journal selection starts a fresh first page.
 */
async function resetsForJournalSelection(): Promise<void> {
  installDefaultWeeklyHandlers();
  const user = userEvent.setup();
  renderWeeklyPage('?db=fixture.sqlite&journal=101');

  expect(await screen.findByText('Weekly first')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: /Second Fixture Journal/ }));

  expect(await screen.findByText('Second journal article')).toBeInTheDocument();
  expect(document.querySelector('[data-weekly-journal="102"]')).not.toBeNull();
  expect(weeklyArticleRequestUrls).toHaveLength(2);
  const request = weeklyArticleRequestUrls[1];
  expect(request.searchParams.get('journal_id')).toBe('102');
  expect(request.searchParams.get('cursor')).toBeNull();
  expect(screen.queryByText('Weekly first')).not.toBeInTheDocument();
}

/**
 * Verify a submitted weekly search starts a bounded server-side page chain.
 */
async function resetsForSearchQuery(): Promise<void> {
  installDefaultWeeklyHandlers();
  const user = userEvent.setup();
  renderWeeklyPage('?db=fixture.sqlite&journal=101');

  expect(await screen.findByText('Weekly first')).toBeInTheDocument();
  const input = screen.getByRole('combobox', { name: '搜索文章' });
  await user.type(input, 'needle');
  await user.click(screen.getByRole('button', { name: '搜索' }));

  expect(await screen.findByText('Needle result')).toBeInTheDocument();
  expect(document.querySelector('[data-weekly-article-state="results"]')).not.toBeNull();
  expect(weeklyArticleRequestUrls).toHaveLength(2);
  const request = weeklyArticleRequestUrls[1];
  expect(request.searchParams.get('q')).toBe('needle');
  expect(request.searchParams.get('limit')).toBe('50');
  expect(request.searchParams.get('cursor')).toBeNull();
  expect(generalArticleRequestCount).toBe(0);
  expect(screen.queryByText('Weekly first')).not.toBeInTheDocument();
}

/**
 * Verify each requested cursor page is loaded once and duplicate boundary rows are hidden.
 */
async function loadsCursorPagesWithoutDuplicates(): Promise<void> {
  installWeeklyHandlers(({ request }) => {
    const cursor = new URL(request.url).searchParams.get('cursor');
    if (cursor === 'page-two') {
      return HttpResponse.json(
        weeklyPage(
          [
            { article_id: 'weekly-2', journal_id: '101', title: 'Weekly second duplicate' },
            { article_id: 'weekly-3', journal_id: '101', title: 'Weekly third' },
          ],
          null,
          false,
        ),
      );
    }
    return HttpResponse.json(
      weeklyPage(
        [
          { article_id: 'weekly-1', journal_id: '101', title: 'Weekly first' },
          { article_id: 'weekly-2', journal_id: '101', title: 'Weekly second' },
        ],
        'page-two',
        true,
      ),
    );
  });
  renderWeeklyPage('?db=fixture.sqlite&journal=101');

  expect(await screen.findByText('Weekly first')).toBeInTheDocument();
  await waitFor(() => expect(weeklyViewMocks.onFetchNextPage).not.toBeNull());
  act(() => weeklyViewMocks.onFetchNextPage?.());

  await waitFor(() => expect(weeklyArticleRequestUrls).toHaveLength(2));
  await waitFor(() => expect(renderedArticleIds()).toEqual(['weekly-1', 'weekly-2', 'weekly-3']));
  expect(weeklyArticleRequestUrls[0].searchParams.get('cursor')).toBeNull();
  expect(weeklyArticleRequestUrls[1].searchParams.get('cursor')).toBe('page-two');

  act(() => weeklyViewMocks.onFetchNextPage?.());
  expect(weeklyArticleRequestUrls).toHaveLength(2);
}

/**
 * Verify a changed summary window starts a fresh first page with the new boundary.
 */
async function resetsForSummaryWindow(): Promise<void> {
  installWeeklyHandlers(({ request }) => {
    const windowEnd = new URL(request.url).searchParams.get('window_end');
    const isUpdatedWindow = windowEnd === '2026-07-09T23:59:59Z';
    return HttpResponse.json(
      weeklyPage([
        {
          article_id: isUpdatedWindow ? 'updated-window' : 'initial-window',
          journal_id: '101',
          title: isUpdatedWindow ? 'Updated window article' : 'Initial window article',
        },
      ]),
    );
  });
  const { queryClient } = renderWeeklyPage('?db=fixture.sqlite&journal=101');

  expect(await screen.findByText('Initial window article')).toBeInTheDocument();
  act(() => {
    queryClient.setQueryData<WeeklyUpdatesSummaryResponse>(['weekly-updates-summary'], {
      ...WEEKLY_SUMMARY_FIXTURE,
      generated_at: '2026-07-09T23:59:59Z',
      window_end: '2026-07-09T23:59:59Z',
    });
  });

  expect(await screen.findByText('Updated window article')).toBeInTheDocument();
  expect(weeklyArticleRequestUrls).toHaveLength(2);
  expect(weeklyArticleRequestUrls[1].searchParams.get('window_end')).toBe('2026-07-09T23:59:59Z');
  expect(weeklyArticleRequestUrls[1].searchParams.get('cursor')).toBeNull();
  expect(screen.queryByText('Initial window article')).not.toBeInTheDocument();
}

/**
 * Verify summary and article failures remain visible in the weekly workspace.
 */
async function rendersWeeklyFailures(): Promise<void> {
  server.use(
    http.get('http://localhost/api/weekly-updates/summary', () =>
      HttpResponse.json({ detail: 'weekly storage unavailable' }, { status: 503 }),
    ),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json([])),
  );
  const summaryRender = renderWeeklyPage('');

  expect(await screen.findByText('加载每周更新失败')).toBeInTheDocument();
  expect(screen.getByText('weekly storage unavailable')).toBeInTheDocument();
  expect(document.querySelector('[data-weekly-state="error"]')).not.toBeNull();
  expect(screen.getByTestId('weekly-state-announcement')).toHaveAttribute('role', 'alert');
  summaryRender.unmount();

  installWeeklyHandlers(() =>
    HttpResponse.json({ detail: 'weekly page unavailable' }, { status: 503 }),
  );
  renderWeeklyPage('?db=fixture.sqlite&journal=101');

  expect(await screen.findByRole('alert')).toHaveTextContent('weekly page unavailable');
  expect(document.querySelector('[data-weekly-article-state="error"]')).not.toBeNull();
  expect(screen.getAllByRole('alert')).toHaveLength(1);
}

/**
 * Verify an empty summary keeps zero counts and accessible selection guidance.
 */
async function rendersEmptyWeeklySummary(): Promise<void> {
  const emptySummary: WeeklyUpdatesSummaryResponse = {
    ...WEEKLY_SUMMARY_FIXTURE,
    databases: [],
  };
  installWeeklyHandlers(() => HttpResponse.json(weeklyPage([])), emptySummary);
  server.use(http.get('http://localhost/api/meta/databases', () => HttpResponse.json([])));
  renderWeeklyPage('');

  expect(await screen.findByText('0 个数据库')).toBeInTheDocument();
  expect(screen.getByText('0 篇新文章')).toBeInTheDocument();
  expect(screen.getByText('请选择一个期刊以查看新收录文章。')).toBeInTheDocument();
  expect(document.querySelector('[data-weekly-article-state="no-journal"]')).not.toBeNull();
  expect(weeklyArticleRequestUrls).toHaveLength(0);
}

beforeEach(() => {
  weeklyArticleRequestUrls.length = 0;
  generalArticleRequestCount = 0;
  weeklySummaryRequestCount = 0;
  weeklyViewMocks.onFetchNextPage = null;
  weeklyViewMocks.useVisiblePageList.mockReset();
  weeklyViewMocks.useVisiblePageList.mockImplementation((options: VisiblePageOptions) => {
    weeklyViewMocks.onFetchNextPage = options.onFetchNextPage ?? null;
    return {
      visiblePages: options.loadedPages,
      prefetchRef: vi.fn(),
      loadMoreRef: vi.fn(),
    };
  });
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  });
});

describe('weekly updates bounded pagination', () => {
  test('loads one summary and one initial bounded article page', loadsOneInitialBoundedPage);
  test('resets to the first page for a database change', resetsForDatabaseSelection);
  test('resets to the first page for a journal change', resetsForJournalSelection);
  test('resets to bounded server search for a query change', resetsForSearchQuery);
  test('loads one cursor page at a time without duplicates', loadsCursorPagesWithoutDuplicates);
  test('resets to the first page for a summary window change', resetsForSummaryWindow);
  test('renders summary and article failures', rendersWeeklyFailures);
  test('renders an empty weekly summary', rendersEmptyWeeklySummary);
});
