/**
 * Durable tracking-job polling, cancellation, and restart coverage.
 */

import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vitest';

import { TrackingSettingsContent } from '@/components/tracking/tracking-settings-content';
import type { ManualPushState, ManualPushStatus } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let statusRequestCount = 0;

/**
 * Build a complete durable manual-push status fixture.
 *
 * @param status - Public durable job state.
 * @param message - User-visible fixed or successful result message.
 * @param pushed - Delivered article count.
 * @returns Manual push status payload.
 */
function manualPushStatus(
  status: ManualPushState,
  message: string,
  pushed: number,
): ManualPushStatus {
  const isActive = status === 'pending' || status === 'running';
  const isIdle = status === 'idle';
  const isUnknown = status === 'unknown';
  return {
    job_id: isIdle ? null : '0123456789abcdef0123456789abcdef',
    status,
    message,
    started_at: status === 'pending' || isIdle ? null : 1,
    finished_at: isActive || isIdle ? null : 2,
    deadline_at: isIdle ? null : 600,
    cancellation_requested: status === 'cancelled',
    can_cancel: isActive,
    can_retry: !isActive && !isIdle && !isUnknown,
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
 * Return an empty administrator-approved AI endpoint catalog.
 *
 * @returns Empty AI endpoint list response.
 */
function aiEndpointsResponse(): Response {
  return HttpResponse.json([]);
}

/**
 * Install common tracking-page fixture handlers.
 */
function installCommonHandlers(): void {
  server.use(
    http.get('http://localhost/api/tracking/status', trackingStatusResponse),
    http.get('http://localhost/api/meta/databases', databasesResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/tracking/notification-settings', notificationSettingsResponse),
    http.get('http://localhost/api/tracking/ai-endpoints', aiEndpointsResponse),
  );
}

/**
 * Verify a persisted running job resumes polling after a page remount.
 */
async function resumesPersistedJobAfterMount(): Promise<void> {
  statusRequestCount = 0;
  installCommonHandlers();
  server.use(
    http.get('http://localhost/api/tracking/push-weekly/status', () => {
      statusRequestCount += 1;
      return HttpResponse.json(
        statusRequestCount === 1
          ? manualPushStatus('running', '任务执行中', 0)
          : manualPushStatus('completed', '推送完成', 2),
      );
    }),
  );

  renderWithQuery(<TrackingSettingsContent userId={31} section="notifications" />);

  expect(await screen.findByText('任务执行中')).toBeInTheDocument();
  expect(
    await screen.findByText('推送完成（已推送 2 篇）', {}, { timeout: 5_000 }),
  ).toBeInTheDocument();
  expect(statusRequestCount).toBe(2);
}

/**
 * Verify enqueue and cancellation update the durable query cache and expose safe retry.
 */
async function cancelsQueuedJobAndAllowsRetry(): Promise<void> {
  installCommonHandlers();
  let current = manualPushStatus('idle', 'No manual push task is available', 0);
  server.use(
    http.get('http://localhost/api/tracking/push-weekly/status', () => HttpResponse.json(current)),
    http.post('http://localhost/api/tracking/push-weekly', () => {
      current = manualPushStatus('pending', '任务已排队', 0);
      return HttpResponse.json(current, { status: 202 });
    }),
    http.post(
      'http://localhost/api/tracking/push-weekly/runs/0123456789abcdef0123456789abcdef/cancel',
      () => {
        current = manualPushStatus('cancelled', '任务已取消', 0);
        return HttpResponse.json(current);
      },
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={32} section="notifications" />);

  await user.click(await screen.findByRole('button', { name: '推送到追踪文件夹' }));
  expect(await screen.findByText('任务已排队')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '取消任务' }));
  expect(await screen.findByText('任务已取消')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '推送到追踪文件夹' })).toBeEnabled();
}

/**
 * Verify an ambiguous outcome does not expose a silent retry action.
 */
async function blocksRetryForUnknownOutcome(): Promise<void> {
  installCommonHandlers();
  server.use(
    http.get('http://localhost/api/tracking/push-weekly/status', () =>
      HttpResponse.json(manualPushStatus('unknown', '结果未知，请先检查投递记录', 0)),
    ),
  );
  renderWithQuery(<TrackingSettingsContent userId={33} section="notifications" />);

  expect(await screen.findByText('结果未知，请先检查投递记录')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '推送到追踪文件夹' })).toBeDisabled();
  expect(screen.queryByRole('button', { name: '取消任务' })).not.toBeInTheDocument();
}

/**
 * Verify the shared footer restores an unsaved tracking draft explicitly.
 */
async function discardsUnsavedSettings(): Promise<void> {
  installCommonHandlers();
  server.use(
    http.get('http://localhost/api/tracking/push-weekly/status', () =>
      HttpResponse.json(manualPushStatus('idle', 'No manual push task is available', 0)),
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={34} section="tracking" />);

  const recommendationSwitch = await screen.findByRole('switch', { name: '启用推荐' });
  await user.click(recommendationSwitch);
  expect(recommendationSwitch).not.toBeChecked();

  const discardButton = screen.getByRole('button', { name: '取消更改' });
  expect(discardButton).toBeEnabled();
  await user.click(discardButton);

  expect(screen.getByRole('switch', { name: '启用推荐' })).toBeChecked();
  expect(discardButton).toBeDisabled();
}

describe('durable tracking job flow', () => {
  test('resumes polling a persisted job after mount', resumesPersistedJobAfterMount, 8_000);
  test('cancels a queued job and enables safe retry', cancelsQueuedJobAndAllowsRetry);
  test('blocks retry for an unknown outcome', blocksRetryForUnknownOutcome);
  test('discards an unsaved settings draft', discardsUnsavedSettings);
});
