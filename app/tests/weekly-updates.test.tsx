/**
 * Weekly-update query isolation, database state, pagination, ordering, and failure coverage.
 */

import { parseAsString, useQueryState } from 'nuqs';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse, type HttpResponseResolver } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { WeeklyUpdatesView } from '@/components/weekly/weekly-updates-view';
import { SELECTED_DATABASE_KEY, readSelectedDatabase } from '@/lib/api';
import type { WeeklyUpdatesResponse } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const weeklyViewMocks = vi.hoisted(() => ({
  useVisiblePageList: vi.fn(({ loadedPages }: { loadedPages: number }) => ({
    visiblePages: loadedPages,
    prefetchRef: vi.fn(),
    loadMoreRef: vi.fn(),
  })),
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

const WEEKLY_UPDATES_FIXTURE: WeeklyUpdatesResponse = {
  generated_at: '2026-07-08T23:59:59Z',
  window_start: '2026-07-01T10:20:30Z',
  window_end: '2026-07-08T23:59:59Z',
  databases: [
    {
      db_name: 'fixture.sqlite',
      generated_at: '2026-07-08T23:59:59Z',
      new_article_count: 3,
      journals: [
        {
          journal_id: 'journal-1',
          journal_title: 'Fixture Journal',
          new_article_count: 3,
          articles: [
            { article_id: 'weekly-1', journal_id: 'journal-1', title: 'Weekly first' },
            { article_id: 'weekly-2', journal_id: 'journal-1', title: 'Weekly second' },
            { article_id: 'weekly-3', journal_id: 'journal-1', title: 'Weekly third' },
          ],
        },
      ],
    },
    {
      db_name: 'other.sqlite',
      generated_at: '2026-07-08T23:59:59Z',
      new_article_count: 1,
      journals: [
        {
          journal_id: 'journal-2',
          journal_title: 'Other Journal',
          new_article_count: 1,
          articles: [
            { article_id: 'other-1', journal_id: 'journal-2', title: 'Other weekly article' },
          ],
        },
      ],
    },
  ],
};

const articleRequestUrls: URL[] = [];

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
 * Install common weekly metadata handlers and a configurable article search handler.
 *
 * @param articleResolver - Article endpoint response resolver.
 */
function installWeeklyHandlers(articleResolver: HttpResponseResolver): void {
  server.use(
    http.get('http://localhost/api/weekly-updates', () =>
      HttpResponse.json(WEEKLY_UPDATES_FIXTURE),
    ),
    http.get('http://localhost/api/meta/databases', () =>
      HttpResponse.json(['fixture.sqlite', 'other.sqlite']),
    ),
    http.get('http://localhost/api/articles', (context) => {
      articleRequestUrls.push(new URL(context.request.url));
      return articleResolver(context);
    }),
  );
}

/**
 * Install an empty article search response.
 */
function installEmptySearchHandler(): void {
  installWeeklyHandlers(() =>
    HttpResponse.json({
      items: [],
      page: { total: null, limit: 200, offset: 0, next_cursor: null, has_more: false },
    }),
  );
}

/**
 * Render the weekly page with matching Next and nuqs query snapshots.
 *
 * @param searchParams - Initial URL query string.
 */
function renderWeeklyPage(searchParams: string): void {
  renderWithQuery(
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
 * Verify a homepage query neither populates nor filters the weekly search.
 */
async function ignoresHomepageQuery(): Promise<void> {
  installEmptySearchHandler();
  renderWeeklyPage('?q=homepage-only&db=fixture.sqlite&journal=journal-1');

  expect(await screen.findByText('Weekly first')).toBeInTheDocument();
  expect(screen.getByRole('combobox', { name: '搜索文章' })).toHaveValue('');
  expect(articleRequestUrls).toHaveLength(0);
  expect(screen.getByRole('complementary')).toBeInTheDocument();
  expect(screen.getByRole('main')).toHaveAttribute('id', 'main-content');
  expect(document.getElementById('results-scroll-container')).toBeInTheDocument();
  expect(weeklyViewMocks.useVisiblePageList).toHaveBeenCalledWith(
    expect.objectContaining({ scrollContainerId: 'results-scroll-container' }),
  );
}

/**
 * Verify selecting a weekly database updates only weekly URL state.
 */
async function keepsHomepageDatabaseSelection(): Promise<void> {
  window.localStorage.setItem(SELECTED_DATABASE_KEY, 'homepage.sqlite');
  installEmptySearchHandler();
  const user = userEvent.setup();
  renderWeeklyPage('?db=fixture.sqlite&journal=journal-1');

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
  expect(readSelectedDatabase()).toBe('homepage.sqlite');
}

/**
 * Verify all cursor pages are searched with complete filters and weekly ordering wins.
 */
async function searchesEveryCursorPage(): Promise<void> {
  installWeeklyHandlers(({ request }) => {
    const cursor = new URL(request.url).searchParams.get('cursor');
    if (cursor === 'page-two') {
      return HttpResponse.json({
        items: [{ article_id: 'weekly-1', title: 'Search first' }],
        page: { total: null, limit: 200, offset: 0, next_cursor: null, has_more: false },
      });
    }
    return HttpResponse.json({
      items: [
        { article_id: 'weekly-3', title: 'Search third' },
        { article_id: 'outside-week', title: 'Outside weekly payload' },
      ],
      page: {
        total: null,
        limit: 200,
        offset: 0,
        next_cursor: 'page-two',
        has_more: true,
      },
    });
  });
  renderWeeklyPage('?q=needle&weekly_q=needle&db=fixture.sqlite&journal=journal-1');

  await waitFor(() => expect(articleRequestUrls).toHaveLength(2));
  await waitFor(() => expect(renderedArticleIds()).toEqual(['weekly-1', 'weekly-3']));
  for (const url of articleRequestUrls) {
    expect(url.searchParams.get('db')).toBe('fixture.sqlite');
    expect(url.searchParams.get('journal_id')).toBe('journal-1');
    expect(url.searchParams.get('q')).toBe('needle');
    expect(url.searchParams.get('limit')).toBe('200');
    expect(url.searchParams.get('date_from')).toBe('2026-07-01');
    expect(url.searchParams.get('date_to')).toBe('2026-07-08');
    expect(url.searchParams.get('include_total')).toBe('0');
  }
  expect(articleRequestUrls[0].searchParams.get('cursor')).toBeNull();
  expect(articleRequestUrls[1].searchParams.get('cursor')).toBe('page-two');
}

/**
 * Verify a repeated cursor fails without rendering partial matches.
 */
async function rejectsRepeatedCursor(): Promise<void> {
  installWeeklyHandlers(({ request }) => {
    const hasCursor = new URL(request.url).searchParams.has('cursor');
    return HttpResponse.json({
      items: [{ article_id: hasCursor ? 'weekly-1' : 'weekly-3' }],
      page: {
        total: null,
        limit: 200,
        offset: 0,
        next_cursor: 'repeated-cursor',
        has_more: true,
      },
    });
  });
  renderWeeklyPage('?q=needle&weekly_q=needle&db=fixture.sqlite&journal=journal-1');

  expect(await screen.findByRole('alert')).toHaveTextContent('分页游标重复');
  expect(articleRequestUrls).toHaveLength(2);
  expect(renderedArticleIds()).toEqual([]);
  expect(screen.queryByText('该期刊中没有匹配全文检索条件的本周文章。')).not.toBeInTheDocument();
}

/**
 * Verify a later-page failure is not reported as partial success or empty data.
 */
async function rejectsLaterPageFailure(): Promise<void> {
  installWeeklyHandlers(({ request }) => {
    const cursor = new URL(request.url).searchParams.get('cursor');
    if (cursor === 'page-two') {
      return HttpResponse.json({ detail: 'second page failed' }, { status: 500 });
    }
    return HttpResponse.json({
      items: [{ article_id: 'weekly-3' }],
      page: {
        total: null,
        limit: 200,
        offset: 0,
        next_cursor: 'page-two',
        has_more: true,
      },
    });
  });
  renderWeeklyPage('?q=needle&weekly_q=needle&db=fixture.sqlite&journal=journal-1');

  expect(await screen.findByRole('alert')).toHaveTextContent('second page failed');
  expect(articleRequestUrls).toHaveLength(2);
  expect(renderedArticleIds()).toEqual([]);
  expect(screen.queryByText('该期刊中没有匹配全文检索条件的本周文章。')).not.toBeInTheDocument();
}

beforeEach(() => {
  articleRequestUrls.length = 0;
  weeklyViewMocks.useVisiblePageList.mockClear();
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  });
});

describe('weekly updates search state', () => {
  test('ignores a homepage-only query parameter', ignoresHomepageQuery);
  test('keeps homepage database selection unchanged', keepsHomepageDatabaseSelection);
  test('searches every cursor page with weekly filters and ordering', searchesEveryCursorPage);
  test('rejects a repeated search cursor without partial results', rejectsRepeatedCursor);
  test('rejects a later-page failure without partial results', rejectsLaterPageFailure);
});
