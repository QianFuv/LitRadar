/**
 * Tracking background-job polling coverage using the extracted feature view and API client.
 */

import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test, vi } from 'vitest';

import { TrackingPageContent } from '@/components/tracking/tracking-page-content';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const navigationMocks = vi.hoisted(() => ({
  push: vi.fn(),
}));

vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: navigationMocks.push }),
}));

let statusRequestCount = 0;

/**
 * Build a complete manual-push status fixture.
 *
 * @param status - Background job state.
 * @param message - Display message.
 * @param pushed - Delivered article count.
 * @returns Manual push status payload.
 */
function manualPushStatus(status: string, message: string, pushed: number) {
  return {
    job_id: 'job-1',
    status,
    message,
    started_at: 1,
    finished_at: status === 'completed' ? 2 : null,
    pushed,
    selected: 2,
    total_candidates: 3,
    summary: 'fixture summary',
    folder_id: 4,
    folder_name: 'Tracking',
  };
}

/**
 * Return current tracking configuration.
 *
 * @returns Tracking status response.
 */
function trackingStatusResponse(): Response {
  return HttpResponse.json({
    tracking_folder: { id: 4, name: 'Tracking' },
    total_folders: 1,
    weekly_articles_available: 3,
    notification_configured: false,
  });
}

/**
 * Return the available database fixture.
 *
 * @returns Database list response.
 */
function databasesResponse(): Response {
  return HttpResponse.json(['fixture.sqlite']);
}

/**
 * Return the tracking folder fixture.
 *
 * @returns Folder list response.
 */
function foldersResponse(): Response {
  return HttpResponse.json([
    { id: 4, name: 'Tracking', is_tracking: true, article_count: 0, created_at: 1 },
  ]);
}

/**
 * Return an unconfigured notification response.
 *
 * @returns Null settings response.
 */
function notificationSettingsResponse(): Response {
  return HttpResponse.json(null);
}

/**
 * Start a running background push fixture.
 *
 * @returns Running status response.
 */
function startPushResponse(): Response {
  return HttpResponse.json(manualPushStatus('running', '任务已启动', 0));
}

/**
 * Reject a manual push because another user owns the process-local slot.
 *
 * @returns Generic service-capacity error response.
 */
function saturatedPushResponse(): Response {
  return HttpResponse.json({ detail: 'Service temporarily unavailable' }, { status: 503 });
}

/**
 * Return one running poll followed by a completed poll.
 *
 * @returns Current background status response.
 */
function pollPushResponse(): Response {
  statusRequestCount += 1;
  return HttpResponse.json(
    statusRequestCount === 1
      ? manualPushStatus('running', '任务执行中', 0)
      : manualPushStatus('completed', '推送完成', 2),
  );
}

/**
 * Verify a running push is polled until completion and updates the UI.
 */
async function pollsUntilCompleted(): Promise<void> {
  statusRequestCount = 0;
  server.use(
    http.get('http://localhost/api/tracking/status', trackingStatusResponse),
    http.get('http://localhost/api/meta/databases', databasesResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/tracking/notification-settings', notificationSettingsResponse),
    http.post('http://localhost/api/tracking/push-weekly', startPushResponse),
    http.get('http://localhost/api/tracking/push-weekly/status', pollPushResponse),
  );
  const user = userEvent.setup();

  renderWithQuery(<TrackingPageContent userId={31} />);

  await user.click(await screen.findByRole('button', { name: '推送到追踪文件夹' }));
  expect(await screen.findByText('任务执行中')).toBeInTheDocument();
  expect(
    await screen.findByText('推送完成（已推送 2 篇）', {}, { timeout: 5_000 }),
  ).toBeInTheDocument();
  expect(statusRequestCount).toBeGreaterThanOrEqual(2);
}

/**
 * Verify a capacity rejection displays its safe detail without starting polling.
 */
async function displaysCapacityErrorWithoutPolling(): Promise<void> {
  statusRequestCount = 0;
  server.use(
    http.get('http://localhost/api/tracking/status', trackingStatusResponse),
    http.get('http://localhost/api/meta/databases', databasesResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/tracking/notification-settings', notificationSettingsResponse),
    http.post('http://localhost/api/tracking/push-weekly', saturatedPushResponse),
    http.get('http://localhost/api/tracking/push-weekly/status', pollPushResponse),
  );
  const user = userEvent.setup();

  renderWithQuery(<TrackingPageContent userId={32} />);

  await user.click(await screen.findByRole('button', { name: '推送到追踪文件夹' }));
  expect(await screen.findByText('Service temporarily unavailable')).toBeInTheDocument();
  expect(statusRequestCount).toBe(0);
}

/**
 * Verify unsaved tracking settings block navigation until explicitly confirmed.
 */
async function confirmsUnsavedNavigation(): Promise<void> {
  server.use(
    http.get('http://localhost/api/tracking/status', trackingStatusResponse),
    http.get('http://localhost/api/meta/databases', databasesResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/tracking/notification-settings', notificationSettingsResponse),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingPageContent userId={33} />);

  await user.click(await screen.findByRole('switch', { name: '启用推荐' }));
  const homeLink = screen.getByRole('link', { name: '返回首页' });
  await user.click(homeLink);

  expect(navigationMocks.push).not.toHaveBeenCalled();
  expect(screen.getByRole('alertdialog', { name: '离开未保存的配置？' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '取消' }));
  expect(homeLink).toHaveFocus();
  expect(navigationMocks.push).not.toHaveBeenCalled();

  await user.click(homeLink);
  await user.click(screen.getByRole('button', { name: '确认离开' }));
  expect(navigationMocks.push).toHaveBeenCalledWith('/');
}

describe('tracking polling flow', () => {
  test('polls a running push until completion', pollsUntilCompleted, 10_000);
  test('shows a capacity error without polling', displaysCapacityErrorWithoutPolling);
  test('confirms navigation away from unsaved settings', confirmsUnsavedNavigation);
});
