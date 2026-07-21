/**
 * Announcement priority, dismissal-window, signature, and storage-recovery coverage.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { AnnouncementsDialog } from '@/components/announcements-dialog';
import type { AnnouncementInfo } from '@/lib/api';
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
 * Install the announcement endpoint and render the production dialog.
 *
 * @param announcements - Enabled announcement response.
 * @returns Render result and query client.
 */
function renderAnnouncements(announcements: AnnouncementInfo[]) {
  server.use(
    http.get('http://localhost/api/announcements', () => HttpResponse.json(announcements)),
  );
  return renderWithQuery(<AnnouncementsDialog />);
}

/**
 * Verify priority labels remain paired with API order and permanent dismissal is persisted.
 */
async function rendersPrioritiesAndPersistsPermanentDismissal(): Promise<void> {
  const announcements = [
    createAnnouncement(1, 'high'),
    createAnnouncement(2, 'normal'),
    createAnnouncement(3, 'low'),
  ];
  const user = userEvent.setup();
  renderAnnouncements(announcements);

  const dialog = await screen.findByRole('dialog', { name: '系统公告' });
  const titles = within(dialog).getAllByRole('heading', { level: 3 });
  expect(titles.map((heading) => heading.textContent)).toEqual([
    'Announcement 1',
    'Announcement 2',
    'Announcement 3',
  ]);
  expect(within(dialog).getByText('高优先级')).toBeInTheDocument();
  expect(within(dialog).getByText('普通')).toBeInTheDocument();
  expect(within(dialog).getByText('低优先级')).toBeInTheDocument();

  await user.click(within(dialog).getByRole('button', { name: '永久关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  for (const announcement of announcements) {
    expect(window.localStorage.getItem(`${STORAGE_PREFIX}${announcement.id}`)).toBe('0');
  }
}

/**
 * Verify malformed and expired dismissal values recover while active dismissals stay hidden.
 */
async function recoversDismissalStorageAndPersistsSevenDays(): Promise<void> {
  const now = FIXED_NOW.getTime();
  window.localStorage.setItem(`${STORAGE_PREFIX}11`, 'not-a-number');
  window.localStorage.setItem(`${STORAGE_PREFIX}12`, String(now - 1));
  window.localStorage.setItem(`${STORAGE_PREFIX}13`, String(now + 60_000));
  window.localStorage.setItem(`${STORAGE_PREFIX}14`, '0');
  const user = userEvent.setup();
  renderAnnouncements([
    createAnnouncement(11),
    createAnnouncement(12),
    createAnnouncement(13),
    createAnnouncement(14),
  ]);

  const dialog = await screen.findByRole('dialog', { name: '系统公告' });
  expect(within(dialog).getByText('Announcement 11')).toBeInTheDocument();
  expect(within(dialog).getByText('Announcement 12')).toBeInTheDocument();
  expect(within(dialog).queryByText('Announcement 13')).not.toBeInTheDocument();
  expect(within(dialog).queryByText('Announcement 14')).not.toBeInTheDocument();
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}11`)).toBeNull();
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}12`)).toBeNull();

  await user.click(within(dialog).getByRole('button', { name: '7天内不再提示' }));
  const sevenDaysLater = now + 7 * 24 * 3600 * 1000;
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}11`)).toBe(String(sevenDaysLater));
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}12`)).toBe(String(sevenDaysLater));
}

/**
 * Verify the today action stores the fixed local end-of-day boundary.
 */
async function persistsEndOfTodayDismissal(): Promise<void> {
  const user = userEvent.setup();
  renderAnnouncements([createAnnouncement(21)]);
  const dialog = await screen.findByRole('dialog', { name: '系统公告' });

  await user.click(within(dialog).getByRole('button', { name: '今日不再提示' }));

  const expectedEndOfDay = new Date(FIXED_NOW);
  expectedEndOfDay.setHours(23, 59, 59, 999);
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}21`)).toBe(
    String(expectedEndOfDay.getTime()),
  );
}

/**
 * Verify closing one signature does not suppress a newly arrived announcement signature.
 */
async function reopensForNewAnnouncementSignature(): Promise<void> {
  const first = createAnnouncement(31);
  const second = createAnnouncement(32, 'high');
  const user = userEvent.setup();
  const { queryClient } = renderAnnouncements([first]);

  const firstDialog = await screen.findByRole('dialog', { name: '系统公告' });
  await user.click(within(firstDialog).getByRole('button', { name: '关闭' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  expect(window.localStorage.getItem(`${STORAGE_PREFIX}31`)).toBeNull();

  queryClient.setQueryData(['announcements'], [first, second]);

  const reopenedDialog = await screen.findByRole('dialog', { name: '系统公告' });
  expect(within(reopenedDialog).getByText('Announcement 31')).toBeInTheDocument();
  expect(within(reopenedDialog).getByText('Announcement 32')).toBeInTheDocument();
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(FIXED_NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('announcements dialog', () => {
  test(
    'renders priority labels and persists permanent dismissal',
    rendersPrioritiesAndPersistsPermanentDismissal,
  );
  test(
    'recovers dismissal storage and persists seven days',
    recoversDismissalStorageAndPersistsSevenDays,
  );
  test('persists the local end-of-day dismissal', persistsEndOfTodayDismissal);
  test('reopens for a new announcement signature', reopensForNewAnnouncementSignature);
});
