/**
 * Administrator user, invite, runtime, scheduler, and announcement API operations.
 */

import {
  parseProviderCatalogResponse,
  parseRuntimeSettingList,
  parseSchedulerStatus,
  parseScheduledTaskInfo,
  parseScheduledTaskList,
  type ProviderCatalogResponse,
  type RuntimeSettingInfo,
  type RuntimeSettingsUpdate,
  type SchedulerStatus,
  type ScheduledTaskCreate,
  type ScheduledTaskInfo,
  type ScheduledTaskUpdate,
} from '@/lib/api-contract';
import { buildApiUrl, requestJson } from '@/lib/api/client';
import type {
  AdminInviteCode,
  AdminStats,
  AdminUserInfo,
  AnnouncementCreate,
  AnnouncementInfo,
  AnnouncementUpdate,
} from '@/lib/api/types';

/**
 * Fetch admin stats.
 *
 * @returns Admin stats.
 */
export function adminGetStats(): Promise<AdminStats> {
  return requestJson<AdminStats>(
    buildApiUrl('/api/admin/stats'),
    null,
    undefined,
    '获取统计信息失败',
  );
}

/**
 * Fetch admin user list.
 *
 * @returns Users.
 */
export function adminGetUsers(): Promise<AdminUserInfo[]> {
  return requestJson<AdminUserInfo[]>(
    buildApiUrl('/api/admin/users'),
    null,
    undefined,
    '获取用户列表失败',
  );
}

/**
 * Grant or revoke admin access.
 *
 * @param userId - User id.
 * @param isAdmin - Whether the user should be admin.
 */
export async function adminSetAdmin(userId: number, isAdmin: boolean): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}/admin`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify({ is_admin: isAdmin }),
    },
    '更新管理员状态失败',
  );
}

/**
 * Reset a user's password.
 *
 * @param userId - User id.
 * @param newPassword - New password.
 */
export async function adminResetPassword(userId: number, newPassword: string): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}/reset-password`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ new_password: newPassword }),
    },
    '重置密码失败',
  );
}

/**
 * Delete a user.
 *
 * @param userId - User id.
 */
export async function adminDeleteUser(userId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/users/${userId}`),
    null,
    { method: 'DELETE' },
    '删除用户失败',
  );
}

/**
 * Fetch admin invite codes.
 *
 * @returns Invite codes.
 */
export function adminGetInviteCodes(): Promise<AdminInviteCode[]> {
  return requestJson<AdminInviteCode[]>(
    buildApiUrl('/api/admin/invite-codes'),
    null,
    undefined,
    '获取邀请码列表失败',
  );
}

/**
 * Create an admin invite code.
 *
 * @returns Created invite code summary.
 */
export function adminCreateInviteCode(): Promise<{ id: number; code: string }> {
  return requestJson<{ id: number; code: string }>(
    buildApiUrl('/api/admin/invite-codes'),
    null,
    { method: 'POST' },
    '创建邀请码失败',
  );
}

/**
 * Delete an unused admin invite code.
 *
 * @param codeId - Invite code id.
 */
export async function adminDeleteInviteCode(codeId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/invite-codes/${codeId}`),
    null,
    { method: 'DELETE' },
    '删除邀请码失败',
  );
}

/**
 * Fetch runtime settings.
 *
 * @returns Runtime settings.
 */
export function adminGetRuntimeSettings(): Promise<RuntimeSettingInfo[]> {
  return requestJson<RuntimeSettingInfo[]>(
    buildApiUrl('/api/admin/runtime-settings'),
    null,
    undefined,
    '获取运行配置失败',
    parseRuntimeSettingList,
  );
}

/**
 * Fetch built-in Provider capabilities and discovered catalog files.
 *
 * @returns Capability-aware Provider and catalog metadata.
 */
export function adminGetProviderCatalog(): Promise<ProviderCatalogResponse> {
  return requestJson<ProviderCatalogResponse>(
    buildApiUrl('/api/admin/provider-catalog'),
    null,
    undefined,
    '获取 Provider 配置目录失败',
    parseProviderCatalogResponse,
  );
}

/**
 * Update runtime settings.
 *
 * @param payload - Runtime settings payload.
 * @returns Updated runtime settings.
 */
export function adminUpdateRuntimeSettings(
  payload: RuntimeSettingsUpdate,
): Promise<RuntimeSettingInfo[]> {
  return requestJson<RuntimeSettingInfo[]>(
    buildApiUrl('/api/admin/runtime-settings'),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新运行配置失败',
    parseRuntimeSettingList,
  );
}

/**
 * Fetch scheduled tasks.
 *
 * @returns Scheduled tasks.
 */
export function adminGetScheduledTasks(): Promise<ScheduledTaskInfo[]> {
  return requestJson<ScheduledTaskInfo[]>(
    buildApiUrl('/api/admin/scheduled-tasks'),
    null,
    undefined,
    '获取定时任务失败',
    parseScheduledTaskList,
  );
}

/**
 * Fetch persisted scheduler health and recent run metadata.
 *
 * @returns Durable scheduler status without captured process output.
 */
export function adminGetSchedulerStatus(): Promise<SchedulerStatus> {
  return requestJson<SchedulerStatus>(
    buildApiUrl('/api/admin/scheduler/status'),
    null,
    undefined,
    '获取调度器状态失败',
    parseSchedulerStatus,
  );
}

/**
 * Create a scheduled task.
 *
 * @param payload - Scheduled task payload.
 * @returns Created scheduled task.
 */
export function adminCreateScheduledTask(payload: ScheduledTaskCreate): Promise<ScheduledTaskInfo> {
  return requestJson<ScheduledTaskInfo>(
    buildApiUrl('/api/admin/scheduled-tasks'),
    null,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    },
    '创建定时任务失败',
    parseScheduledTaskInfo,
  );
}

/**
 * Update a scheduled task.
 *
 * @param taskId - Task id.
 * @param payload - Scheduled task patch.
 * @returns Updated scheduled task.
 */
export function adminUpdateScheduledTask(
  taskId: number,
  payload: ScheduledTaskUpdate,
): Promise<ScheduledTaskInfo> {
  return requestJson<ScheduledTaskInfo>(
    buildApiUrl(`/api/admin/scheduled-tasks/${taskId}`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新定时任务失败',
    parseScheduledTaskInfo,
  );
}

/**
 * Delete a scheduled task.
 *
 * @param taskId - Task id.
 */
export async function adminDeleteScheduledTask(taskId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/scheduled-tasks/${taskId}`),
    null,
    { method: 'DELETE' },
    '删除定时任务失败',
  );
}

/**
 * Fetch admin announcement list.
 *
 * @returns Announcements.
 */
export function adminGetAnnouncements(): Promise<AnnouncementInfo[]> {
  return requestJson<AnnouncementInfo[]>(
    buildApiUrl('/api/admin/announcements'),
    null,
    undefined,
    '获取公告列表失败',
  );
}

/**
 * Create an announcement.
 *
 * @param payload - Announcement payload.
 * @returns Created announcement.
 */
export function adminCreateAnnouncement(payload: AnnouncementCreate): Promise<AnnouncementInfo> {
  return requestJson<AnnouncementInfo>(
    buildApiUrl('/api/admin/announcements'),
    null,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    },
    '创建公告失败',
  );
}

/**
 * Update an announcement.
 *
 * @param announcementId - Announcement id.
 * @param payload - Announcement patch.
 * @returns Updated announcement.
 */
export function adminUpdateAnnouncement(
  announcementId: number,
  payload: AnnouncementUpdate,
): Promise<AnnouncementInfo> {
  return requestJson<AnnouncementInfo>(
    buildApiUrl(`/api/admin/announcements/${announcementId}`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    },
    '更新公告失败',
  );
}

/**
 * Delete an announcement.
 *
 * @param announcementId - Announcement id.
 */
export async function adminDeleteAnnouncement(announcementId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/admin/announcements/${announcementId}`),
    null,
    { method: 'DELETE' },
    '删除公告失败',
  );
}
