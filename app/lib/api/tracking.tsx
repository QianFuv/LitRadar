/**
 * Tracking status, manual push, and notification-setting API operations.
 */

import {
  parseManualPushStatus,
  parseNotificationSettings,
  parseNullableNotificationSettings,
  parseTrackingStatus,
  type ManualPushStatus,
  type NotificationSettings,
  type NotificationSettingsUpdate,
  type TrackingStatus,
} from '@/lib/api-contract';
import { buildApiUrl, requestJson } from '@/lib/api/client';

/**
 * Fetch tracking status.
 *
 * @returns Tracking status.
 */
export function getTrackingStatus(): Promise<TrackingStatus> {
  return requestJson<TrackingStatus>(
    buildApiUrl('/api/tracking/status'),
    null,
    undefined,
    '获取追踪状态失败',
    parseTrackingStatus,
  );
}

/**
 * Start weekly article push.
 *
 * @returns Push status.
 */
export function pushWeeklyToTracking(): Promise<ManualPushStatus> {
  return requestJson<ManualPushStatus>(
    buildApiUrl('/api/tracking/push-weekly'),
    null,
    { method: 'POST' },
    '推送每周文章失败',
    parseManualPushStatus,
  );
}

/**
 * Fetch weekly push status.
 *
 * @returns Push status.
 */
export function getPushWeeklyStatus(): Promise<ManualPushStatus> {
  return requestJson<ManualPushStatus>(
    buildApiUrl('/api/tracking/push-weekly/status'),
    null,
    undefined,
    '获取推送状态失败',
    parseManualPushStatus,
  );
}

/**
 * Fetch notification settings.
 *
 * @returns Notification settings or null.
 */
export function getNotificationSettings(): Promise<NotificationSettings | null> {
  return requestJson<NotificationSettings | null>(
    buildApiUrl('/api/tracking/notification-settings'),
    null,
    undefined,
    '获取通知设置失败',
    parseNullableNotificationSettings,
  );
}

/**
 * Update notification settings.
 *
 * @param settings - Settings payload.
 * @returns Saved settings.
 */
export function updateNotificationSettings(
  settings: NotificationSettingsUpdate,
): Promise<NotificationSettings> {
  return requestJson<NotificationSettings>(
    buildApiUrl('/api/tracking/notification-settings'),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(settings),
    },
    '更新通知设置失败',
    parseNotificationSettings,
  );
}
