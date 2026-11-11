/**
 * Real Chromium coverage for the virtualized journal filter: keyboard navigation, selection,
 * filtering, and list semantics over a library-sized option list.
 */

import '@/src/globals.css';

import { screen, waitFor, within } from '@testing-library/react';
import { page, userEvent } from 'vite-plus/test/browser';
import { beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import { SearchFilters } from '@/components/feature/search-filters';
import { AuthProvider } from '@/lib/auth-context';
import { setSelectedDatabase } from '@/lib/selected-database';
import { createAuthUserScenario } from '@/tests/mocks/scenarios';
import { renderWithQuery } from '@/tests/render';

const TOPICS = ['Information Systems', 'Operations Research', 'Sustainable Development'];
const JOURNALS = Array.from({ length: 600 }, (_, index) => ({
  journal_id: String(index + 1),
  title: `Journal of ${TOPICS[index % TOPICS.length]} ${index + 1}`,
}));
const API_FIXTURES = new Map(
  Object.entries({
    '/api/auth/me': createAuthUserScenario({ id: 7, username: 'browser', is_admin: false }),
    '/api/meta/databases': ['browser.sqlite'],
    '/api/meta/areas': [],
    '/api/meta/journals': JOURNALS,
    '/api/years': [],
    '/api/meta/ratings': { utd_rating: [], abs_rating: [], fms_rating: [], fmscn_rating: [] },
  }),
);
/** Time allowed for the journal panel to load its options and open. */
const PANEL_TIMEOUT_MS = 10_000;

/**
 * Answer the filter bar's API requests from fixtures.
 */
function stubApi(): void {
  vi.spyOn(globalThis, 'fetch').mockImplementation((input) => {
    const { pathname } = new URL(input instanceof Request ? input.url : String(input));
    const body = API_FIXTURES.get(pathname);
    return Promise.resolve(
      Response.json(body ?? { title: 'Not Found', status: 404 }, { status: body ? 200 : 404 }),
    );
  });
}

/**
 * Render the filter bar and open the journal panel.
 *
 * @param url - Initial location, such as one carrying `journal_id` selections.
 * @param pillTitle - Title of the journal pill for that selection.
 * @returns Option list element of the open panel.
 */
async function openJournalPanel(url = '/', pillTitle = '期刊'): Promise<HTMLElement> {
  setSelectedDatabase('browser.sqlite');
  await renderWithQuery(
    <AuthProvider>
      <SearchFilters />
    </AuthProvider>,
    { url },
  );
  await expect.element(page.getByTitle(pillTitle, { exact: true })).toBeVisible();
  await page.getByTitle(pillTitle, { exact: true }).click();
  await expect
    .element(page.getByText('在 600 本期刊中选择。'), { timeout: PANEL_TIMEOUT_MS })
    .toBeVisible();
  return waitFor(() => screen.getByRole('listbox'), { timeout: PANEL_TIMEOUT_MS });
}

/**
 * Read the rows currently mounted in the list.
 *
 * @param list - Option list element.
 * @returns Mounted option elements in DOM order.
 */
function mountedOptions(list: HTMLElement): HTMLElement[] {
  return within(list).queryAllByRole('option');
}

/**
 * Read the list position of the highlighted row.
 *
 * @param list - Option list element.
 * @returns One-based position, or null when no row is highlighted.
 */
function highlightedPosition(list: HTMLElement): number | null {
  const row = list.querySelector('[role="option"][data-highlighted]');
  return row ? Number(row.getAttribute('aria-posinset')) : null;
}

/**
 * Check whether a row lies entirely inside the list's scroll viewport.
 *
 * @param list - Option list element.
 * @param position - One-based list position of the row.
 * @returns Whether the row is mounted and fully visible.
 */
function isRowInView(list: HTMLElement, position: number): boolean {
  const row = list.querySelector(`[role="option"][aria-posinset="${position}"]`);
  if (!row) {
    return false;
  }
  const rowBox = row.getBoundingClientRect();
  const listBox = list.getBoundingClientRect();
  return rowBox.top >= listBox.top - 1 && rowBox.bottom <= listBox.bottom + 1;
}

/**
 * Verify only a window of rows mounts while each row announces its place in the full list.
 */
async function mountsVisibleRowsWithListPositions(): Promise<void> {
  const list = await openJournalPanel();

  await waitFor(() => expect(mountedOptions(list).length).toBeGreaterThan(0));
  const rows = mountedOptions(list);
  expect(rows.length).toBeLessThan(40);
  expect(rows[0]).toHaveAttribute('aria-posinset', '1');
  expect(rows[0]).toHaveAttribute('aria-setsize', String(JOURNALS.length));
  expect(rows[0]).toHaveAccessibleName(JOURNALS[0].title);

  list.scrollTop = list.scrollHeight;
  await waitFor(() =>
    expect(within(list).getByRole('option', { name: JOURNALS[599].title })).toHaveAttribute(
      'aria-posinset',
      '600',
    ),
  );
  expect(mountedOptions(list).length).toBeLessThan(40);
}

/**
 * Verify arrow keys move through rows, wrap past the mounted range, and keep the highlight
 * in view, while Home and End stay with the search input.
 */
async function navigatesRowsFromTheKeyboard(): Promise<void> {
  const list = await openJournalPanel();
  const input = screen.getByRole('combobox', { name: '搜索期刊' });
  await waitFor(() => expect(input).toHaveFocus());

  await userEvent.keyboard('{ArrowUp}');
  await waitFor(() => expect(highlightedPosition(list)).toBe(600));
  await waitFor(() => expect(isRowInView(list, 600)).toBe(true));

  await userEvent.keyboard('{ArrowDown}{ArrowDown}');
  await waitFor(() => expect(highlightedPosition(list)).toBe(1));
  await waitFor(() => expect(isRowInView(list, 1)).toBe(true));

  for (let step = 0; step < 19; step += 1) {
    await userEvent.keyboard('{ArrowDown}');
  }
  await waitFor(() => expect(highlightedPosition(list)).toBe(20));
  await waitFor(() => expect(isRowInView(list, 20)).toBe(true));

  await userEvent.keyboard('Op');
  await userEvent.keyboard('{Home}');
  expect(input).toHaveProperty('selectionStart', 0);
  await userEvent.keyboard('{End}');
  expect(input).toHaveProperty('selectionStart', 2);
  await userEvent.keyboard('{Control>}a{/Control}{Backspace}');
  await waitFor(() => expect(input).toHaveValue(''));

  await userEvent.keyboard('{ArrowDown}{ArrowDown}{ArrowDown}{Enter}');
  await waitFor(() =>
    expect(within(list).getByRole('option', { name: JOURNALS[2].title })).toHaveAttribute(
      'aria-selected',
      'true',
    ),
  );
  await expect
    .element(page.getByTitle(`期刊：${JOURNALS[2].title}`, { exact: true }))
    .toBeInTheDocument();
}

/**
 * Verify pointer selection keeps several journals and lists them first when reopened.
 */
async function selectsSeveralJournalsAndListsThemFirst(): Promise<void> {
  await openJournalPanel();
  await page.getByRole('option', { name: JOURNALS[4].title, exact: true }).click();
  await page.getByRole('option', { name: JOURNALS[1].title, exact: true }).click();
  await expect.element(page.getByTitle('期刊：已选 2 本', { exact: true })).toBeInTheDocument();
  await userEvent.keyboard('{Escape}');
  await waitFor(() => expect(screen.queryByRole('listbox')).not.toBeInTheDocument());

  await page.getByTitle('期刊：已选 2 本', { exact: true }).click();
  const list = await waitFor(() => screen.getByRole('listbox'));
  await waitFor(() => expect(mountedOptions(list).length).toBeGreaterThan(2));
  const [first, second, third] = mountedOptions(list);
  expect(first).toHaveAccessibleName(JOURNALS[1].title);
  expect(first).toHaveAttribute('aria-selected', 'true');
  expect(second).toHaveAccessibleName(JOURNALS[4].title);
  expect(second).toHaveAttribute('aria-selected', 'true');
  expect(third).toHaveAccessibleName(JOURNALS[0].title);
  expect(third).toHaveAttribute('aria-selected', 'false');
}

/**
 * Verify search narrows the rows and their announced set size, and shows the empty state.
 */
async function filtersRowsAndShowsTheEmptyState(): Promise<void> {
  const list = await openJournalPanel();
  const input = screen.getByRole('combobox', { name: '搜索期刊' });
  list.scrollTop = list.scrollHeight;
  await waitFor(() => expect(list.scrollTop).toBeGreaterThan(0));

  await userEvent.fill(input, 'information');
  await waitFor(() => expect(mountedOptions(list)[0]).toHaveAccessibleName(JOURNALS[0].title));
  expect(list.scrollTop).toBe(0);
  expect(mountedOptions(list)[0]).toHaveAttribute('aria-setsize', String(JOURNALS.length / 3));

  const query = 'development 59';
  const matches = JOURNALS.filter((journal) => journal.title.toLowerCase().includes(query));
  await userEvent.fill(input, query);
  await waitFor(() =>
    expect(mountedOptions(list).map((row) => row.textContent)).toEqual(
      matches.map((journal) => journal.title),
    ),
  );
  for (const row of mountedOptions(list)) {
    expect(row).toHaveAttribute('aria-setsize', String(matches.length));
  }

  await userEvent.fill(input, 'no such journal');
  await expect.element(page.getByText('未找到期刊。')).toBeVisible();
  expect(screen.queryAllByRole('option')).toHaveLength(0);
}

beforeEach(stubApi);

describe('virtualized journal filter', () => {
  test('mounts visible rows with their list positions', mountsVisibleRowsWithListPositions);
  test('navigates rows from the keyboard', navigatesRowsFromTheKeyboard);
  test('selects several journals and lists them first', selectsSeveralJournalsAndListsThemFirst);
  test('filters rows and shows the empty state', filtersRowsAndShowsTheEmptyState);
});
