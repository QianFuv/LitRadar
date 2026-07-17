/**
 * Search draft, history, active-filter, month-range, and scroll-reset coverage.
 */

import { useQueryState, parseAsString } from 'nuqs';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { ThemeProvider } from 'next-themes';
import { renderHook, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import type { ReactNode } from 'react';
import { describe, expect, test, vi } from 'vitest';

import { ActiveFilterChips } from '@/components/feature/active-filter-chips';
import { SearchBar } from '@/components/feature/search-bar';
import { Sidebar } from '@/components/feature/sidebar';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { AuthProvider } from '@/lib/auth-context';
import {
  buildMonthRange,
  buildRecentMonthRange,
  formatMonthRangeLabel,
  getMonthRangeDateBounds,
  getYearBounds,
  parseMonthRange,
  resolveMonthRangeForYears,
} from '@/lib/article-filters';
import { readSelectedDatabase } from '@/lib/api';
import { setSelectedDatabase } from '@/lib/selected-database';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const navigationMocks = vi.hoisted(() => ({
  refresh: vi.fn(),
  replace: vi.fn(),
}));

vi.mock('next/navigation', () => ({
  usePathname: () => '/',
  useRouter: () => navigationMocks,
}));

vi.mock('next-themes', () => ({
  ThemeProvider: ({ children }: { children: ReactNode }) => children,
  useTheme: () => ({ setTheme: vi.fn(), theme: 'dark' }),
}));

vi.mock('react-intersection-observer', () => ({
  useInView: () => ({ ref: vi.fn() }),
}));

/**
 * Display one live nuqs string value for interaction assertions.
 *
 * @param props - Query parameter name and test identifier.
 * @returns Query-state probe.
 */
function QueryProbe({ parameter, testId }: { parameter: string; testId: string }) {
  const [value] = useQueryState(parameter, parseAsString);
  return <output data-testid={testId}>{value ?? ''}</output>;
}

/**
 * Return an authenticated fixture user.
 *
 * @returns Current-user response.
 */
function currentUserResponse(): Response {
  return HttpResponse.json({ id: 21, username: 'filter_user', is_admin: false });
}

/**
 * Verify month parsing, clamping, date bounds, labels, and recent shortcuts.
 */
function normalizesMonthRanges(): void {
  expect(parseMonthRange('2024-12..2024-02')).toEqual(['2024-02', '2024-12']);
  expect(parseMonthRange('2024-13..2025-01')).toBeNull();
  expect(resolveMonthRangeForYears('2019-06..2028-04', 2020, 2026)).toEqual(['2020-01', '2026-12']);
  expect(getMonthRangeDateBounds('2024-02..2024-02')).toEqual({
    dateFrom: '2024-02-01',
    dateTo: '2024-02-29',
  });
  expect(formatMonthRangeLabel('2024-02..2024-12')).toBe('2024年02月 - 2024年12月');
  expect(buildMonthRange('2024-02', '2024-12')).toBe('2024-02..2024-12');
  expect(buildRecentMonthRange(1, 2020, 2026, new Date(2026, 6, 15))).toEqual([
    '2025-08',
    '2026-07',
  ]);
  expect(buildRecentMonthRange(5, 2024, 2026, new Date(2026, 6, 15))).toEqual([
    '2024-01',
    '2026-07',
  ]);
  expect(getYearBounds([])).toBeNull();
  expect(getYearBounds([{ year: 2025 }, { year: 2021 }, { year: 2023 }])).toEqual({
    max: 2025,
    min: 2021,
  });
}

/**
 * Verify clearing edits only the draft until an explicit empty submission.
 */
async function keepsSearchDraftSeparate(): Promise<void> {
  const user = userEvent.setup();
  renderWithQuery(
    <NuqsTestingAdapter searchParams="?q=applied" hasMemory>
      <SearchBar />
      <QueryProbe parameter="q" testId="query-value" />
    </NuqsTestingAdapter>,
  );
  const input = screen.getByRole('combobox', { name: '搜索文章' });

  expect(input).toHaveValue('applied');
  await user.click(screen.getByRole('button', { name: '清空搜索输入' }));
  expect(input).toHaveValue('');
  expect(screen.getByTestId('query-value')).toHaveTextContent('applied');

  await user.click(screen.getByRole('button', { name: '搜索' }));
  await waitFor(() => expect(screen.getByTestId('query-value')).toHaveTextContent(''));

  await user.type(input, 'new query');
  await user.keyboard('{Enter}');
  await waitFor(() => expect(screen.getByTestId('query-value')).toHaveTextContent('new query'));
}

/**
 * Verify history arrows select entries, Enter applies, and Escape restores input focus.
 */
async function operatesSearchHistoryFromKeyboard(): Promise<void> {
  window.localStorage.setItem(
    'litradar:v1:search_history',
    JSON.stringify(['first query', 'second query']),
  );
  const user = userEvent.setup();
  renderWithQuery(
    <NuqsTestingAdapter hasMemory>
      <SearchBar />
      <QueryProbe parameter="q" testId="history-query" />
    </NuqsTestingAdapter>,
  );
  const input = screen.getByRole('combobox', { name: '搜索文章' });

  await user.click(input);
  expect(await screen.findByRole('listbox', { name: '最近搜索' })).toBeInTheDocument();
  await user.keyboard('{ArrowDown}');
  expect(screen.getByRole('option', { name: 'first query' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await user.keyboard('{ArrowDown}{Enter}');
  await waitFor(() =>
    expect(screen.getByTestId('history-query')).toHaveTextContent('second query'),
  );

  await user.click(input);
  await user.keyboard('{Escape}');
  expect(screen.queryByRole('listbox', { name: '最近搜索' })).not.toBeInTheDocument();
  expect(input).toHaveFocus();
}

/**
 * Verify a custom query parameter does not mutate the homepage query.
 */
async function targetsOptionalQueryParameter(): Promise<void> {
  const user = userEvent.setup();
  renderWithQuery(
    <NuqsTestingAdapter searchParams="?q=home&weekly_q=weekly" hasMemory>
      <SearchBar queryParam="weekly_q" />
      <QueryProbe parameter="q" testId="home-query" />
      <QueryProbe parameter="weekly_q" testId="weekly-query" />
    </NuqsTestingAdapter>,
  );
  const input = screen.getByRole('combobox', { name: '搜索文章' });

  await user.clear(input);
  await user.type(input, 'weekly next');
  await user.keyboard('{Enter}');

  await waitFor(() => expect(screen.getByTestId('weekly-query')).toHaveTextContent('weekly next'));
  expect(screen.getByTestId('home-query')).toHaveTextContent('home');
}

/**
 * Verify every applied filter is visible and reset leaves the database unchanged.
 */
async function resetsVisibleFilterChips(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  server.use(
    http.get('http://localhost/api/meta/journals', () =>
      HttpResponse.json([{ journal_id: 'journal-1', title: 'Journal One' }]),
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(
    <NuqsTestingAdapter
      searchParams="?q=systems&area=Information%20Systems&journal_id=journal-1&month_range=2024-01..2024-12"
      hasMemory
    >
      <ActiveFilterChips />
    </NuqsTestingAdapter>,
  );

  expect(await screen.findByTestId('active-filter-chips')).toHaveTextContent('systems');
  expect(screen.getByTestId('active-filter-chips')).toHaveTextContent('信息系统');
  await waitFor(() =>
    expect(screen.getByTestId('active-filter-chips')).toHaveTextContent('Journal One'),
  );
  expect(screen.getByTestId('active-filter-chips')).toHaveTextContent('2024年01月 - 2024年12月');

  await user.click(screen.getByRole('button', { name: '移除领域 信息系统' }));
  await waitFor(() =>
    expect(screen.queryByRole('button', { name: '移除领域 信息系统' })).not.toBeInTheDocument(),
  );
  expect(screen.getByTestId('active-filter-chips')).toHaveTextContent('Journal One');

  await user.click(screen.getByRole('button', { name: '重置筛选' }));
  await waitFor(() => expect(screen.queryByTestId('active-filter-chips')).not.toBeInTheDocument());
  expect(readSelectedDatabase()).toBe('fixture.sqlite');
}

/**
 * Verify empty year metadata renders no fabricated year selectors.
 */
async function rendersUnavailableYearState(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  const onUrlUpdate = vi.fn();
  const user = userEvent.setup();
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(['fixture.sqlite'])),
    http.get('http://localhost/api/meta/areas', () => HttpResponse.json([])),
    http.get('http://localhost/api/meta/journals', () => HttpResponse.json([])),
    http.get('http://localhost/api/years', () => HttpResponse.json([])),
  );
  renderWithQuery(
    <ThemeProvider attribute="class">
      <AuthProvider>
        <NuqsTestingAdapter
          searchParams="?q=systems&area=Information%20Systems&journal_id=journal-1&month_range=2024-01..2024-12"
          hasMemory
          onUrlUpdate={onUrlUpdate}
        >
          <Sidebar />
        </NuqsTestingAdapter>
      </AuthProvider>
    </ThemeProvider>,
  );

  expect(await screen.findByText('暂无可用发表年份')).toBeInTheDocument();
  expect(screen.queryByLabelText('起始年份')).not.toBeInTheDocument();
  expect(screen.getByRole('link', { name: 'LitRadar 首页' })).toBeInTheDocument();
  const pageNavigation = screen.getByRole('navigation', { name: '页面导航' });
  const pageLinks = within(pageNavigation).getAllByRole('link');
  expect(pageLinks).toHaveLength(3);
  expect(within(pageNavigation).getByRole('link', { name: '文献检索' })).toMatchObject({
    title: '文献检索',
  });
  expect(within(pageNavigation).getByRole('link', { name: '文献检索' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  expect(within(pageNavigation).getByRole('link', { name: '我的收藏' })).toHaveAttribute(
    'href',
    '/?view=favorites',
  );
  expect(within(pageNavigation).getByRole('link', { name: '每周更新' })).toHaveAttribute(
    'href',
    '/?view=weekly-updates',
  );
  expect(pageNavigation.querySelectorAll('.sr-only')).toHaveLength(3);
  const resetButtons = screen.getAllByRole('button', { name: '重置筛选' });
  const publicationTimeHeading = screen.getByRole('heading', { name: '发表时间' });
  expect(resetButtons).toHaveLength(1);
  expect(resetButtons[0]).toHaveClass('bg-sidebar-primary', 'text-sidebar-primary-foreground');
  expect(
    publicationTimeHeading.compareDocumentPosition(resetButtons[0]) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);

  await user.click(resetButtons[0]);
  await waitFor(() => expect(onUrlUpdate).toHaveBeenCalled());
  const lastUpdate = onUrlUpdate.mock.calls.at(-1)?.[0];
  expect(lastUpdate?.searchParams.toString()).toBe('');
  expect(readSelectedDatabase()).toBe('fixture.sqlite');
}

/**
 * Verify year metadata enables the three recent-range shortcuts.
 */
async function rendersRecentRangeShortcuts(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(['fixture.sqlite'])),
    http.get('http://localhost/api/meta/areas', () => HttpResponse.json([])),
    http.get('http://localhost/api/meta/journals', () => HttpResponse.json([])),
    http.get('http://localhost/api/years', () =>
      HttpResponse.json([
        { year: 2020, issue_count: 1, journal_count: 1 },
        { year: new Date().getFullYear(), issue_count: 1, journal_count: 1 },
      ]),
    ),
  );
  renderWithQuery(
    <ThemeProvider attribute="class">
      <AuthProvider>
        <NuqsTestingAdapter hasMemory>
          <Sidebar />
        </NuqsTestingAdapter>
      </AuthProvider>
    </ThemeProvider>,
  );

  expect(await screen.findByRole('button', { name: '近 1 年' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '近 3 年' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '近 5 年' })).toBeInTheDocument();
}

/**
 * Verify list-key resets explicitly disable smooth scrolling.
 */
function resetsResultsScrollWithoutAnimation(): void {
  const scrollContainer = document.createElement('div');
  scrollContainer.id = 'test-results-scroll';
  scrollContainer.scrollTo = vi.fn();
  document.body.append(scrollContainer);

  renderHook(() =>
    useVisiblePageList({
      listKey: 'filters-a',
      loadedPages: 1,
      scrollContainerId: scrollContainer.id,
    }),
  );

  expect(scrollContainer.scrollTo).toHaveBeenCalledWith({ behavior: 'auto', top: 0 });
  scrollContainer.remove();
}

describe('search and filter UI', () => {
  test('normalizes month range values and recent shortcuts', normalizesMonthRanges);
  test('keeps draft clearing separate from query submission', keepsSearchDraftSeparate);
  test(
    'operates search history with Arrow keys, Enter, and Escape',
    operatesSearchHistoryFromKeyboard,
  );
  test('targets an optional search query parameter', targetsOptionalQueryParameter);
  test(
    'shows and resets all active filter chips without changing database',
    resetsVisibleFilterChips,
  );
  test('shows an unavailable state for empty year metadata', rendersUnavailableYearState);
  test('renders one, three, and five year shortcuts', rendersRecentRangeShortcuts);
  test('resets result scrolling with automatic behavior', resetsResultsScrollWithoutAnimation);
});
