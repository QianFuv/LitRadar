/**
 * Administrator user, invite, runtime, scheduler, and announcement API operations.
 */

import {
  parseAnnouncement,
  parseAnnouncementList,
  parseProviderCatalogResponse,
  parseRuntimeSettingList,
  parseSchedulerStatus,
  parseScheduledTaskInfo,
  parseScheduledTaskList,
  type RuntimeSettingInfo,
  type RuntimeSettingsUpdate,
  type ScheduledTaskCreate,
  type ScheduledTaskInfo,
  type ScheduledTaskUpdate,
} from '@/lib/api-contract';
import { $api, apiClient, readData } from '@/lib/api/client';
import { apiQuery, validatedApiQuery } from '@/lib/api/query';
import type {
  AdminInviteCode,
  AdminInviteCodeCreate,
  AdminStats,
  AnnouncementCreate,
  AnnouncementInfo,
  AnnouncementUpdate,
  TelemetryKind,
  TelemetryPeriod,
} from '@/lib/api/types';

/**
 * Fetch admin stats.
 *
 * @returns Admin stats.
 */
export async function adminGetStats(): Promise<AdminStats> {
  return readData(await apiClient.GET('/api/admin/stats'));
}

/**
 * Grant or revoke admin access.
 *
 * @param userId - User id.
 * @param isAdmin - Whether the user should be admin.
 */
export async function adminSetAdmin(userId: number, isAdmin: boolean): Promise<void> {
  await apiClient.PUT('/api/admin/users/{user_id}/admin', {
    params: { path: { user_id: userId } },
    body: { is_admin: isAdmin },
  });
}

/**
 * Reset a user's password.
 *
 * @param userId - User id.
 * @param newPassword - New password.
 */
export async function adminResetPassword(userId: number, newPassword: string): Promise<void> {
  await apiClient.POST('/api/admin/users/{user_id}/reset-password', {
    params: { path: { user_id: userId } },
    body: { new_password: newPassword },
  });
}

/**
 * Delete a user.
 *
 * @param userId - User id.
 */
export async function adminDeleteUser(userId: number): Promise<void> {
  await apiClient.DELETE('/api/admin/users/{user_id}', { params: { path: { user_id: userId } } });
}

/**
 * Create an admin invite code.
 *
 * @param input - Optional expiry and maximum-use overrides.
 * @returns Created invite code.
 */
export async function adminCreateInviteCode(
  input: AdminInviteCodeCreate = {},
): Promise<AdminInviteCode> {
  return readData(await apiClient.POST('/api/admin/invite-codes', { body: input }));
}

/**
 * Irreversibly revoke an invite code.
 *
 * @param codeId - Invite code id.
 */
export async function adminRevokeInviteCode(codeId: number): Promise<void> {
  await apiClient.DELETE('/api/admin/invite-codes/{code_id}', {
    params: { path: { code_id: codeId } },
  });
}

/**
 * Update runtime settings.
 *
 * @param payload - Runtime settings payload.
 * @returns Updated runtime settings.
 */
export async function adminUpdateRuntimeSettings(
  payload: RuntimeSettingsUpdate,
): Promise<RuntimeSettingInfo[]> {
  return parseRuntimeSettingList(
    readData(await apiClient.PUT('/api/admin/runtime-settings', { body: payload })),
  );
}

/**
 * Create a scheduled task.
 *
 * @param payload - Scheduled task payload.
 * @returns Created scheduled task.
 */
export async function adminCreateScheduledTask(
  payload: ScheduledTaskCreate,
): Promise<ScheduledTaskInfo> {
  return parseScheduledTaskInfo(
    readData(await apiClient.POST('/api/admin/scheduled-tasks', { body: payload })),
  );
}

/**
 * Update a scheduled task.
 *
 * @param taskId - Task id.
 * @param payload - Scheduled task patch.
 * @returns Updated scheduled task.
 */
export async function adminUpdateScheduledTask(
  taskId: number,
  payload: ScheduledTaskUpdate,
): Promise<ScheduledTaskInfo> {
  return parseScheduledTaskInfo(
    readData(
      await apiClient.PUT('/api/admin/scheduled-tasks/{task_id}', {
        params: { path: { task_id: taskId } },
        body: payload,
      }),
    ),
  );
}

/**
 * Delete a scheduled task.
 *
 * @param taskId - Task id.
 */
export async function adminDeleteScheduledTask(taskId: number): Promise<void> {
  await apiClient.DELETE('/api/admin/scheduled-tasks/{task_id}', {
    params: { path: { task_id: taskId } },
  });
}

/**
 * Create an announcement.
 *
 * @param payload - Announcement payload.
 * @returns Created announcement.
 */
export async function adminCreateAnnouncement(
  payload: AnnouncementCreate,
): Promise<AnnouncementInfo> {
  return parseAnnouncement(
    readData(await apiClient.POST('/api/admin/announcements', { body: payload })),
  );
}

/**
 * Update an announcement.
 *
 * @param announcementId - Announcement id.
 * @param payload - Announcement patch.
 * @returns Updated announcement.
 */
export async function adminUpdateAnnouncement(
  announcementId: number,
  payload: AnnouncementUpdate,
): Promise<AnnouncementInfo> {
  return parseAnnouncement(
    readData(
      await apiClient.PUT('/api/admin/announcements/{announcement_id}', {
        params: { path: { announcement_id: announcementId } },
        body: payload,
      }),
    ),
  );
}

/**
 * Delete an announcement.
 *
 * @param announcementId - Announcement id.
 */
export async function adminDeleteAnnouncement(announcementId: number): Promise<void> {
  await apiClient.DELETE('/api/admin/announcements/{announcement_id}', {
    params: { path: { announcement_id: announcementId } },
  });
}

/**
 * Query of the administrator statistics.
 *
 * @returns Query options.
 */
export function adminStatsQuery() {
  return apiQuery($api.queryOptions('get', '/api/admin/stats'));
}

/**
 * Query of anonymous hourly load and search-shape telemetry for one UTC window.
 *
 * @param period - UTC hour-aligned window ending at the next hour boundary.
 * @param kind - Optional interface kind filter, or null for every kind.
 * @returns Query options.
 */
export function adminTelemetryQuery(period: TelemetryPeriod, kind: TelemetryKind | null) {
  return apiQuery(
    $api.queryOptions('get', '/api/admin/telemetry', {
      params: { query: { period, ...(kind ? { kind } : {}) } },
    }),
  );
}

/**
 * Query of the administrator user list.
 *
 * @returns Query options.
 */
export function adminUsersQuery() {
  return apiQuery($api.queryOptions('get', '/api/admin/users'));
}

/**
 * Query of the administrator invite codes.
 *
 * @returns Query options.
 */
export function adminInviteCodesQuery() {
  return apiQuery($api.queryOptions('get', '/api/admin/invite-codes'));
}

/**
 * Query of the runtime settings, contract-validated.
 *
 * @returns Query options.
 */
export function runtimeSettingsQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/admin/runtime-settings'),
    parseRuntimeSettingList,
  );
}

/**
 * Query of the Provider capabilities and catalog files, contract-validated.
 *
 * @returns Query options.
 */
export function providerCatalogQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/admin/provider-catalog'),
    parseProviderCatalogResponse,
  );
}

/**
 * Query of the scheduled tasks, contract-validated.
 *
 * @returns Query options.
 */
export function scheduledTasksQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/admin/scheduled-tasks'),
    parseScheduledTaskList,
  );
}

/**
 * Query of the scheduler health and recent runs, contract-validated.
 *
 * @returns Query options.
 */
export function schedulerStatusQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/admin/scheduler/status'),
    parseSchedulerStatus,
  );
}

/**
 * Query of every announcement for administrators, with priorities validated.
 *
 * @returns Query options.
 */
export function adminAnnouncementsQuery() {
  return validatedApiQuery(
    $api.queryOptions('get', '/api/admin/announcements'),
    parseAnnouncementList,
  );
}
