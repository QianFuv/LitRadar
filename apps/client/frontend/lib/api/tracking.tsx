/**
 * Tracking status, manual push, and notification-setting API operations.
 */

import {
  parseAiEndpointCatalog,
  parseManualPushStatus,
  parseNotificationSettings,
  parseNullableNotificationSettings,
  parseTrackingStatus,
  type ManualPushStatus,
  type NotificationSettings,
  type NotificationSettingsUpdate,
  type TrackingStatus,
} from '@/lib/api-contract';
import { $api, apiClient, readData } from '@/lib/api/client';
import { validatedApiQuery } from '@/lib/api/query';

/**
 * Fetch tracking status.
 *
 * @returns Tracking status.
 */
export async function getTrackingStatus(): Promise<TrackingStatus> {
  return parseTrackingStatus(readData(await apiClient.GET('/api/tracking/status')));
}

/**
 * Start weekly article push.
 *
 * @returns Push status.
 */
export async function pushWeeklyToTracking(): Promise<ManualPushStatus> {
  return parseManualPushStatus(readData(await apiClient.POST('/api/tracking/push-weekly')));
}

/**
 * Request cancellation of one durable weekly-push run.
 *
 * @param jobId - Owner-visible durable job identifier.
 * @returns Updated durable push status.
 */
export async function cancelPushWeeklyRun(jobId: string): Promise<ManualPushStatus> {
  return parseManualPushStatus(
    readData(
      await apiClient.POST('/api/tracking/push-weekly/runs/{run_id}/cancel', {
        params: { path: { run_id: jobId } },
      }),
    ),
  );
}

/**
 * Acknowledge the latest ambiguous weekly push and start a safe new attempt.
 *
 * @param jobId - Owner-visible ambiguous job identifier.
 * @returns Newly queued push status.
 */
export async function acknowledgeUnknownPushWeeklyRun(jobId: string): Promise<ManualPushStatus> {
  return parseManualPushStatus(
    readData(
      await apiClient.POST('/api/tracking/push-weekly/runs/{run_id}/acknowledge', {
        params: { path: { run_id: jobId } },
      }),
    ),
  );
}

/**
 * Update notification settings.
 *
 * @param settings - Settings payload.
 * @returns Saved settings.
 */
export async function updateNotificationSettings(
  settings: NotificationSettingsUpdate,
): Promise<NotificationSettings> {
  return parseNotificationSettings(
    readData(await apiClient.PUT('/api/tracking/notification-settings', { body: settings })),
  );
}

/**
 * Query of the tracking status, contract-validated.
 *
 * @returns Query options.
 */
export function trackingStatusQuery() {
  return validatedApiQuery($api.queryOptions('get', '/api/tracking/status'), parseTrackingStatus);
}

/**
 * Query of the latest manual weekly push, contract-validated.
 *
 * @returns Query options.
 */
export function pushWeeklyStatusQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/tracking/push-weekly/status'),
    parseManualPushStatus,
  );
}

/**
 * Query of the redacted notification settings, contract-validated.
 *
 * @returns Query options.
 */
export function notificationSettingsQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/tracking/notification-settings'),
    parseNullableNotificationSettings,
  );
}

/**
 * Query of the administrator-approved AI endpoints, contract-validated.
 *
 * @returns Query options.
 */
export function aiEndpointsQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/tracking/ai-endpoints'),
    parseAiEndpointCatalog,
  );
}
