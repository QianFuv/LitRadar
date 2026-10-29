/**
 * Search draft, history, filter-pill, month-range, and scroll-reset coverage.
 */

import { useQueryState, parseAsString } from 'nuqs';
import { NuqsTestingAdapter } from 'nuqs/adapters/testing';
import { ThemeProvider } from 'next-themes';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { SearchBar } from '@/components/feature/search-bar';
import { SearchFilters } from '@/components/feature/search-filters';
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

vi.mock('@/components/ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
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
 * Replace one query parameter from outside the search form.
 *
 * @param props - Query parameter name and replacement value.
 * @returns Query-state update control.
 */
function QuerySetter({ parameter, value }: { parameter: string; value: string }) {
  const [, setValue] = useQueryState(parameter, parseAsString);
  return (
    <button type="button" onClick={() => void setValue(value)}>
      Replace query
    </button>
  );
}

/**
 * Return an authenticated fixture user.
 *
 * @returns Current-user response.
 */
function currentUserResponse(): Response {
  return HttpResponse.json({ id: 21, username: 'filter_user', is_admin: false });
}

/** Return an explicit empty rating response for unrelated sidebar scenarios. */
function emptyRatingsResponse(): Response {
  return HttpResponse.json({ utd_rating: [], abs_rating: [], fms_rating: [], fmscn_rating: [] });
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
 * Verify an external URL query change replaces a stale local draft.
 */
async function synchronizesExternalSearchQuery(): Promise<void> {
  const user = userEvent.setup();
  renderWithQuery(
    <NuqsTestingAdapter searchParams="?q=applied" hasMemory>
      <SearchBar />
      <QuerySetter parameter="q" value="external" />
    </NuqsTestingAdapter>,
  );
  const input = screen.getByRole('combobox', { name: '搜索文章' });

  await user.clear(input);
  await user.type(input, 'draft');
  expect(input).toHaveValue('draft');

  await user.click(screen.getByRole('button', { name: 'Replace query' }));
  await waitFor(() => expect(input).toHaveValue('external'));
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
  expect(screen.getByRole('option', { name: 'first query' }).parentElement).toHaveAttribute(
    'data-motion-history-key',
    'first query',
  );
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

  await user.click(input);
  await user.click(screen.getByRole('button', { name: '清空' }));
  await waitFor(() =>
    expect(screen.queryByRole('listbox', { name: '最近搜索' })).not.toBeInTheDocument(),
  );
  expect(window.localStorage.getItem('litradar:v1:search_history')).toBeNull();
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

/** Return publication years whose recent shortcuts differ from the fixture month range. */
function yearsResponse(): Response {
  return HttpResponse.json([
    { year: 2020, issue_count: 1, journal_count: 1 },
    { year: new Date().getFullYear(), issue_count: 1, journal_count: 1 },
  ]);
}

/**
 * Open one filter pill and return its panel.
 *
 * @param user - User-event session.
 * @param name - Accessible pill name pattern.
 * @returns Opened filter panel.
 */
async function openFilterPanel(
  user: ReturnType<typeof userEvent.setup>,
  name: RegExp,
): Promise<HTMLElement> {
  await screen.findByRole('combobox', { name: '检索数据库' });
  await user.click(await screen.findByRole('button', { name }));
  return screen.findByRole('dialog');
}

/**
 * Close the currently open filter panel with its completion action.
 *
 * @param user - User-event session.
 */
async function closeFilterPanel(user: ReturnType<typeof userEvent.setup>): Promise<void> {
  await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: '完成' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
}

/**
 * Verify every applied filter is summarized on its pill and clearing keeps query and database.
 */
async function summarizesAndClearsAppliedFilters(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(['fixture.sqlite'])),
    http.get('http://localhost/api/meta/areas', () =>
      HttpResponse.json([{ value: 'Information Systems', count: 3 }]),
    ),
    http.get('http://localhost/api/meta/journals', () =>
      HttpResponse.json([{ journal_id: 'journal-1', title: 'Journal One' }]),
    ),
    http.get('http://localhost/api/years', yearsResponse),
    http.get('http://localhost/api/meta/ratings', () =>
      HttpResponse.json({
        utd_rating: [],
        abs_rating: [
          { value: '4', count: 2 },
          { value: '4*', count: 1 },
        ],
        fms_rating: [{ value: 'A', count: 1 }],
        fmscn_rating: [],
      }),
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(
    <ThemeProvider attribute="class">
      <AuthProvider>
        <NuqsTestingAdapter
          searchParams="?q=systems&area=Information%20Systems&journal_id=journal-1&month_range=2024-01..2024-12&abs_rating=4,4*&fms_rating=A"
          hasMemory
        >
          <SearchFilters />
          <QueryProbe parameter="q" testId="retained-query" />
        </NuqsTestingAdapter>
      </AuthProvider>
    </ThemeProvider>,
  );

  await screen.findByRole('combobox', { name: '检索数据库' });
  expect(await screen.findByRole('button', { name: '领域：信息系统' })).toBeInTheDocument();
  expect(await screen.findByRole('button', { name: '期刊：Journal One' })).toBeInTheDocument();
  expect(
    await screen.findByRole('button', { name: '发表时间：2024年01月 - 2024年12月' }),
  ).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '期刊评级：已选 3 项' })).toBeInTheDocument();

  const ratingPanel = await openFilterPanel(user, /^期刊评级/);
  await user.click(within(ratingPanel).getByRole('checkbox', { name: 'ABS 4' }));
  expect(within(ratingPanel).getByRole('checkbox', { name: 'ABS 4*' })).toBeChecked();
  await closeFilterPanel(user);
  expect(await screen.findByRole('button', { name: '期刊评级：已选 2 项' })).toBeInTheDocument();

  await user.click(screen.getByTitle('清空全部筛选'));
  await waitFor(() => expect(screen.getByRole('button', { name: '领域' })).toBeInTheDocument());
  expect(screen.getByRole('button', { name: '期刊' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '发表时间' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '期刊评级' })).toBeInTheDocument();
  expect(screen.queryByTitle('清空全部筛选')).not.toBeInTheDocument();
  expect(screen.getByTestId('retained-query')).toHaveTextContent('systems');
  expect(readSelectedDatabase()).toBe('fixture.sqlite');
}

/**
 * Verify empty year metadata renders no fabricated year selectors.
 */
async function rendersUnavailableYearState(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  server.use(
    http.get('http://localhost/api/meta/ratings', emptyRatingsResponse),
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(['fixture.sqlite'])),
    http.get('http://localhost/api/meta/areas', () => HttpResponse.json([])),
    http.get('http://localhost/api/meta/journals', () => HttpResponse.json([])),
    http.get('http://localhost/api/years', () => HttpResponse.json([])),
  );
  const user = userEvent.setup();
  renderWithQuery(
    <ThemeProvider attribute="class">
      <AuthProvider>
        <NuqsTestingAdapter
          searchParams="?q=systems&area=Information%20Systems&journal_id=journal-1&month_range=2024-01..2024-12"
          hasMemory
        >
          <SearchFilters />
        </NuqsTestingAdapter>
      </AuthProvider>
    </ThemeProvider>,
  );

  const timePanel = await openFilterPanel(user, /^发表时间/);
  expect(await within(timePanel).findByText('暂无可用发表年份')).toBeInTheDocument();
  expect(within(timePanel).queryByLabelText('起始年份')).not.toBeInTheDocument();
  expect(within(timePanel).queryByRole('radiogroup', { name: '发表时间' })).not.toBeInTheDocument();
  expect(readSelectedDatabase()).toBe('fixture.sqlite');
}

/**
 * Verify year metadata enables the three recent-range shortcuts and a custom range.
 */
async function rendersRecentRangeShortcuts(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  server.use(
    http.get('http://localhost/api/meta/ratings', emptyRatingsResponse),
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
  const user = userEvent.setup();
  renderWithQuery(
    <ThemeProvider attribute="class">
      <AuthProvider>
        <NuqsTestingAdapter hasMemory>
          <SearchFilters />
        </NuqsTestingAdapter>
      </AuthProvider>
    </ThemeProvider>,
  );

  const timePanel = await openFilterPanel(user, /^发表时间/);
  expect(await within(timePanel).findByRole('radio', { name: '近 1 年' })).toBeInTheDocument();
  expect(within(timePanel).getByRole('radio', { name: '近 3 年' })).toBeInTheDocument();
  expect(within(timePanel).getByRole('radio', { name: '近 5 年' })).toBeInTheDocument();
  expect(within(timePanel).getByRole('radio', { name: '不限' })).toHaveAttribute(
    'aria-checked',
    'true',
  );
  await user.click(within(timePanel).getByRole('radio', { name: '近 3 年' }));
  expect(within(timePanel).getByRole('radio', { name: '近 3 年' })).toHaveAttribute(
    'aria-checked',
    'true',
  );
  await user.click(within(timePanel).getByRole('radio', { name: '自定义范围' }));
  expect(within(timePanel).getByRole('combobox', { name: '起始年份' })).toBeInTheDocument();
  await closeFilterPanel(user);
  expect(screen.getByRole('button', { name: '发表时间：近 3 年' })).toBeInTheDocument();
}

/** Verify rating unions, intersections, reset scopes and database-specific choices. */
async function selectsRatingsAndClearsThemWhenChangingDatabase(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  const requestedDatabases: string[] = [];
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/meta/databases', () =>
      HttpResponse.json(['fixture.sqlite', 'ccf.sqlite']),
    ),
    http.get('http://localhost/api/meta/areas', () =>
      HttpResponse.json([{ value: 'Medicine', count: 2 }]),
    ),
    http.get('http://localhost/api/meta/journals', () =>
      HttpResponse.json([{ journal_id: '101', title: 'Journal One' }]),
    ),
    http.get('http://localhost/api/years', () => HttpResponse.json([])),
    http.get('http://localhost/api/meta/ratings', ({ request }) => {
      const database = new URL(request.url).searchParams.get('db') ?? '';
      requestedDatabases.push(database);
      return database === 'ccf.sqlite'
        ? emptyRatingsResponse()
        : HttpResponse.json({
            utd_rating: [{ value: 'UTD24', count: 1 }],
            abs_rating: [
              { value: '4', count: 2 },
              { value: '4*', count: 1 },
            ],
            fms_rating: [{ value: 'A', count: 1 }],
            fmscn_rating: [{ value: 'T1', count: 1 }],
          });
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(
    <ThemeProvider attribute="class">
      <AuthProvider>
        <NuqsTestingAdapter searchParams="?q=retained&area=Medicine&journal_id=101" hasMemory>
          <SearchFilters />
          <QueryProbe parameter="q" testId="retained-query" />
        </NuqsTestingAdapter>
      </AuthProvider>
    </ThemeProvider>,
  );
  let ratingPanel = await openFilterPanel(user, /^期刊评级/);
  await user.click(await within(ratingPanel).findByRole('checkbox', { name: 'ABS 4' }));
  await user.click(within(ratingPanel).getByRole('checkbox', { name: 'ABS 4*' }));
  await user.click(within(ratingPanel).getByRole('checkbox', { name: 'FMS A' }));
  await user.click(within(ratingPanel).getByRole('checkbox', { name: 'FMS 中国 T1' }));
  expect(within(ratingPanel).getByRole('checkbox', { name: 'ABS 4' })).toBeChecked();
  expect(within(ratingPanel).getByRole('checkbox', { name: 'ABS 4*' })).toBeChecked();
  await closeFilterPanel(user);
  expect(await screen.findByRole('button', { name: '期刊评级：已选 4 项' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '领域：Medicine' })).toBeInTheDocument();
  expect(await screen.findByRole('button', { name: '期刊：Journal One' })).toBeInTheDocument();
  expect(screen.getByTestId('retained-query')).toHaveTextContent('retained');

  ratingPanel = await openFilterPanel(user, /^期刊评级/);
  await user.click(within(ratingPanel).getByRole('button', { name: '清除' }));
  await waitFor(() =>
    expect(within(ratingPanel).getByRole('checkbox', { name: 'ABS 4' })).not.toBeChecked(),
  );
  await user.click(within(ratingPanel).getByRole('checkbox', { name: 'UTD UTD24' }));
  await closeFilterPanel(user);
  expect(screen.getByRole('button', { name: '领域：Medicine' })).toBeInTheDocument();
  expect(screen.getByTestId('retained-query')).toHaveTextContent('retained');

  screen.getByRole('combobox', { name: '检索数据库' }).focus();
  await user.keyboard('{ArrowDown}');
  await user.keyboard('{End}{Enter}');
  await waitFor(() => expect(readSelectedDatabase()).toBe('ccf.sqlite'));
  await waitFor(() => expect(screen.getByRole('button', { name: '期刊评级' })).toBeInTheDocument());
  ratingPanel = await openFilterPanel(user, /^期刊评级/);
  expect(await within(ratingPanel).findByText('当前数据库暂无期刊评级信息。')).toBeVisible();
  expect(
    within(ratingPanel).queryByRole('checkbox', { name: 'UTD UTD24' }),
  ).not.toBeInTheDocument();
  expect(requestedDatabases).toContain('fixture.sqlite');
  expect(requestedDatabases).toContain('ccf.sqlite');
}

beforeEach(() => {
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  });
});

/** Verify ordinary typing avoids storage reads while arrows can load newly saved history. */
async function readsHistoryOnlyForNavigation(): Promise<void> {
  const user = userEvent.setup();
  renderWithQuery(
    <NuqsTestingAdapter>
      <SearchBar />
    </NuqsTestingAdapter>,
  );
  const input = screen.getByRole('combobox', { name: '搜索文章' });
  await user.click(input);
  const storageRead = vi.spyOn(Storage.prototype, 'getItem');
  await user.type(input, 'a long ordinary search query', { skipClick: true });
  expect(storageRead).not.toHaveBeenCalled();
  window.localStorage.setItem('litradar:v1:search_history', JSON.stringify(['recent query']));
  await user.keyboard('{ArrowDown}');
  expect(storageRead).toHaveBeenCalledWith('litradar:v1:search_history');
  expect(await screen.findByRole('option', { name: 'recent query' })).toBeInTheDocument();
}

/** Verify closed journal controls skip option construction and keep filtering and selection. */
async function defersClosedJournalOptions(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  server.use(
    http.get('http://localhost/api/meta/ratings', emptyRatingsResponse),
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(['fixture.sqlite'])),
    http.get('http://localhost/api/meta/areas', () => HttpResponse.json([])),
    http.get('http://localhost/api/meta/journals', () =>
      HttpResponse.json(
        Array.from({ length: 100 }, (_, index) => ({
          journal_id: String(index),
          title: `Journal ${index}`,
        })),
      ),
    ),
    http.get('http://localhost/api/years', () => HttpResponse.json([])),
  );
  const view = renderWithQuery(
    <AuthProvider>
      <NuqsTestingAdapter hasMemory>
        <SearchFilters />
      </NuqsTestingAdapter>
    </AuthProvider>,
  );
  await waitFor(() =>
    expect(view.queryClient.getQueryData(['meta', 'journals', 'fixture.sqlite'])).toHaveLength(100),
  );
  const options = view.queryClient.getQueryData<{ journal_id: string; title: string }[]>([
    'meta',
    'journals',
    'fixture.sqlite',
  ])!;
  const mapOptions = vi.spyOn(options, 'map');
  view.rerender(
    <AuthProvider>
      <NuqsTestingAdapter hasMemory>
        <SearchFilters className="updated" />
      </NuqsTestingAdapter>
    </AuthProvider>,
  );
  expect(mapOptions).not.toHaveBeenCalled();
  const user = userEvent.setup();
  const journalPanel = await openFilterPanel(user, /^期刊$/);
  expect(
    await within(journalPanel).findByRole('checkbox', { name: 'Journal 99' }),
  ).toBeInTheDocument();
  expect(mapOptions).toHaveBeenCalled();
  await user.type(within(journalPanel).getByRole('textbox', { name: '搜索期刊' }), 'Journal 99');
  expect(
    within(journalPanel).queryByRole('checkbox', { name: 'Journal 0' }),
  ).not.toBeInTheDocument();
  await user.click(within(journalPanel).getByRole('checkbox', { name: 'Journal 99' }));
  expect(within(journalPanel).getByRole('checkbox', { name: 'Journal 99' })).toBeChecked();
  await user.keyboard('{Escape}');
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  const reopenedPanel = await openFilterPanel(user, /^期刊：Journal 99/);
  expect(await within(reopenedPanel).findByRole('checkbox', { name: 'Journal 99' })).toBeChecked();
  expect(within(reopenedPanel).getByRole('textbox', { name: '搜索期刊' })).toHaveValue(
    'Journal 99',
  );
}

/** Keep large filter selections on indexed membership checks while preserving order and toggles. */
async function indexesSelectedJournalMembership(): Promise<void> {
  setSelectedDatabase('fixture.sqlite');
  const selected = Array.from({ length: 100 }, (_, index) => String(index * 2 + 1));
  const selectedAreas = Array.from({ length: 40 }, (_, index) => `PerfArea${index * 2 + 1}`);
  const searchParams = `?journal_id=${selected.join(',')}&area=${selectedAreas.join(',')}`;
  server.use(
    http.get('http://localhost/api/meta/ratings', emptyRatingsResponse),
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.get('http://localhost/api/meta/databases', () => HttpResponse.json(['fixture.sqlite'])),
    http.get('http://localhost/api/meta/areas', () =>
      HttpResponse.json(
        Array.from({ length: 80 }, (_, index) => ({ value: `PerfArea${index}`, count: 1 })),
      ),
    ),
    http.get('http://localhost/api/meta/journals', () =>
      HttpResponse.json(
        Array.from({ length: 200 }, (_, index) => ({
          journal_id: String(index),
          title: `Journal ${index}`,
        })),
      ),
    ),
    http.get('http://localhost/api/years', () => HttpResponse.json([])),
  );
  const view = renderWithQuery(
    <AuthProvider>
      <NuqsTestingAdapter searchParams={searchParams} hasMemory>
        <SearchFilters />
      </NuqsTestingAdapter>
    </AuthProvider>,
  );
  await waitFor(() =>
    expect(view.queryClient.getQueryData(['meta', 'journals', 'fixture.sqlite'])).toHaveLength(200),
  );
  const user = userEvent.setup();
  const includes = vi.spyOn(Array.prototype, 'includes');
  try {
    view.rerender(
      <AuthProvider>
        <NuqsTestingAdapter searchParams={searchParams} hasMemory>
          <SearchFilters className="updated" />
        </NuqsTestingAdapter>
      </AuthProvider>,
    );
    await openFilterPanel(user, /^期刊：已选 100 本/);
    await waitFor(() => expect(document.getElementById('journal-199')).toBeChecked());
    expect(document.getElementById('journal-0')).not.toBeChecked();
    expect(document.querySelector('[id^="journal-"]')).toHaveAccessibleName('Journal 1');
    await closeFilterPanel(user);
    await openFilterPanel(user, /^领域：已选 40 个/);
    await waitFor(() => expect(document.getElementById('area-PerfArea1')).toBeChecked());
    expect(document.getElementById('area-PerfArea0')).not.toBeChecked();
    expect(
      includes.mock.contexts.filter(
        (values) =>
          Array.isArray(values) &&
          ((values.length === 100 && values[0] === '1') ||
            (values.length === 40 && values[0] === 'PerfArea1')),
      ),
    ).toHaveLength(0);
  } finally {
    includes.mockRestore();
  }
  await user.click(document.getElementById('area-PerfArea1')!);
  expect(document.getElementById('area-PerfArea1')).not.toBeChecked();
  await closeFilterPanel(user);
  await openFilterPanel(user, /^期刊：已选 100 本/);
  await user.click(document.getElementById('journal-199')!);
  expect(document.getElementById('journal-199')).not.toBeChecked();
  await user.click(document.getElementById('journal-0')!);
  expect(document.getElementById('journal-0')).toBeChecked();
  expect(document.querySelector('[id^="journal-"]')).toHaveAccessibleName('Journal 0');
}

describe('search and filter UI', () => {
  test(
    'indexes large journal selections without row scans',
    indexesSelectedJournalMembership,
    15000,
  );
  test('defers journal options until opening the popover', defersClosedJournalOptions, 15000);
  test('reads persisted history only for navigation keys', readsHistoryOnlyForNavigation);
  test(
    'combines rating groups and clears their state on database changes',
    selectsRatingsAndClearsThemWhenChangingDatabase,
    15000,
  );
  test('normalizes month range values and recent shortcuts', normalizesMonthRanges);
  test('keeps draft clearing separate from query submission', keepsSearchDraftSeparate);
  test('synchronizes external query changes without an effect', synchronizesExternalSearchQuery);
  test(
    'operates search history with Arrow keys, Enter, and Escape',
    operatesSearchHistoryFromKeyboard,
  );
  test('targets an optional search query parameter', targetsOptionalQueryParameter);
  test(
    'summarizes applied filters on pills and clears them without changing database',
    summarizesAndClearsAppliedFilters,
  );
  test('shows an unavailable state for empty year metadata', rendersUnavailableYearState);
  test('renders one, three, and five year shortcuts', rendersRecentRangeShortcuts, 15000);
});
