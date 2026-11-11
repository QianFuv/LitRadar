/**
 * Announcement toasts: priority, per-announcement dismissal windows, storage recovery, and
 * once-per-page display.
 */

import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import { AnnouncementToasts } from '@/components/announcement-toasts';
import { announcementsQuery, type AnnouncementInfo } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const FIXED_NOW = new Date('2026-07-22T10:30:00+08:00');
const STORAGE_PREFIX = 'litradar:v1:announcement_dismissed:';

/**
 * Build a typed announcement fixture.
 *
 * @param id - Stable announcement identifier.
 * @param priority - Display priority.
 * @returns Announcement response item.
 */
function createAnnouncement(
  id: number,
  priority: AnnouncementInfo['priority'] = 'normal',
): AnnouncementInfo {
  return {
    id,
    title: `Announcement ${id}`,
    message: `Message ${id}`,
    priority,
    enabled: true,
    created_at: 1_700_000_000 + id,
    updated_at: 1_700_000_100 + id,
  };
}

/**
 * Install the announcement endpoint and render the production toasts.
 *
 * @param announcements - Enabled announcement response.
 * @returns Render result and query client.
 */
function renderAnnouncements(announcements: AnnouncementInfo[]) {
  server.use(
    http.get('http://localhost/api/announcements', () => HttpResponse.json(announcements)),
  );
  return renderWithQuery(<AnnouncementToasts />);
}

/**
 * Find the toast that shows one announcement. Base UI keeps toasts out of the accessibility tree
 * until their entry transition ends, which jsdom never runs, so hidden elements are included.
 *
 * @param id - Announcement identifier.
 * @returns Toast element.
 */
async function findToast(id: number): Promise<HTMLElement> {
  const title = await screen.findByRole('heading', { name: `Announcement ${id}`, hidden: true });
  const toast = title.closest<HTMLElement>('[role="dialog"], [role="alertdialog"]');
  expect(toast).not.toBeNull();
  return toast!;
}

/**
 * Report whether an announcement toast is on screen.
 *
 * @param id - Announcement identifier.
 * @returns Whether its toast is rendered.
 */
function hasToast(id: number): boolean {
  return screen.queryByRole('heading', { name: `Announcement ${id}`, hidden: true }) !== null;
}

/** Show every unread announcement with its priority and persist a permanent dismissal for one. */
async function rendersPrioritiesAndDismissesOnePermanently(): Promise<void> {
  const user = userEvent.setup();
  await renderAnnouncements([
    createAnnouncement(1, 'high'),
    createAnnouncement(2, 'normal'),
    createAnnouncement(3, 'low'),
  ]);

  expect(within(await findToast(1)).getByText('高优先级')).toBeInTheDocument();
  expect(within(await findToast(2)).getByText('普通')).toBeInTheDocument();
  expect(within(await findToast(3)).getByText('低优先级')).toBeInTheDocument();

  await user.click(
    within(await findToast(2)).getByRole('button', { name: '永久关闭', hidden: true }),
  );
  await waitFor(() => expect(hasToast(2)).toBe(false));
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}2`)).toBe('0');
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}1`)).toBeNull();
  expect(hasToast(1)).toBe(true);
}

/** Recover malformed and expired records, keep active dismissals hidden, and store seven days. */
async function recoversDismissalStorageAndPersistsSevenDays(): Promise<void> {
  const now = FIXED_NOW.getTime();
  window.localStorage.setItem(`${STORAGE_PREFIX}11`, 'not-a-number');
  window.localStorage.setItem(`${STORAGE_PREFIX}12`, String(now - 1));
  window.localStorage.setItem(`${STORAGE_PREFIX}13`, String(now + 60_000));
  window.localStorage.setItem(`${STORAGE_PREFIX}14`, '0');
  const user = userEvent.setup();
  await renderAnnouncements([
    createAnnouncement(11),
    createAnnouncement(12),
    createAnnouncement(13),
    createAnnouncement(14),
  ]);

  const toast = await findToast(11);
  await findToast(12);
  expect(hasToast(13)).toBe(false);
  expect(hasToast(14)).toBe(false);
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}11`)).toBeNull();
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}12`)).toBeNull();

  await user.click(within(toast).getByRole('button', { name: '7天内不再提示', hidden: true }));
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}11`)).toBe(
    String(now + 7 * 24 * 3600 * 1000),
  );
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}12`)).toBeNull();
}

/** Store the local end-of-day boundary for the today action. */
async function persistsEndOfTodayDismissal(): Promise<void> {
  const user = userEvent.setup();
  await renderAnnouncements([createAnnouncement(21)]);
  await user.click(
    within(await findToast(21)).getByRole('button', { name: '今日不再提示', hidden: true }),
  );

  const expectedEndOfDay = new Date(FIXED_NOW);
  expectedEndOfDay.setHours(23, 59, 59, 999);
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}21`)).toBe(
    String(expectedEndOfDay.getTime()),
  );
}

/** Keep a closed toast away on refetch while a newly published announcement still appears. */
async function showsEachAnnouncementOncePerPage(): Promise<void> {
  const first = createAnnouncement(31);
  const second = createAnnouncement(32, 'high');
  const user = userEvent.setup();
  const { queryClient } = await renderAnnouncements([first]);

  await user.click(within(await findToast(31)).getByLabelText('关闭'));
  await waitFor(() => expect(hasToast(31)).toBe(false));
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}31`)).toBeNull();

  await act(() => queryClient.setQueryData(announcementsQuery().queryKey, [first, second]));

  await findToast(32);
  expect(hasToast(31)).toBe(false);
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(FIXED_NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('announcement toasts', () => {
  test(
    'render priorities and dismiss one announcement permanently',
    rendersPrioritiesAndDismissesOnePermanently,
  );
  test(
    'recover dismissal storage and persist seven days',
    recoversDismissalStorageAndPersistsSevenDays,
  );
  test('persist the local end-of-day dismissal', persistsEndOfTodayDismissal);
  test('show each announcement once per page', showsEachAnnouncementOncePerPage);
});
