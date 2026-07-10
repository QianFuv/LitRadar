/**
 * Generated API aliases and runtime decoders for security-sensitive responses.
 */

import type { components } from '@/lib/generated/api-schema';

type ApiSchemas = components['schemas'];

export type AuthUser = ApiSchemas['UserResponse'];
export type LoginResponse = ApiSchemas['LoginResponse'];
export type InviteRequirement = ApiSchemas['InviteRequiredResponse'];
export type TrackingStatus = ApiSchemas['TrackingStatusResponse'];
export type ManualPushStatus = ApiSchemas['ManualWeeklyPushStatus'];
export type NotificationSettings = ApiSchemas['NotificationSettingsResponse'];
type GeneratedNotificationSettingsUpdate = ApiSchemas['NotificationSettingsUpdate'];
type NotificationSecretField = 'ai_api_key' | 'ai_backup_api_key' | 'pushplus_token';
export type NotificationSettingsUpdate = Required<
  Omit<GeneratedNotificationSettingsUpdate, NotificationSecretField>
> &
  Pick<GeneratedNotificationSettingsUpdate, NotificationSecretField>;
export type RuntimeSecretItemInfo = ApiSchemas['RuntimeSecretItemInfo'];
export type RuntimeSecretPoolUpdate = Required<ApiSchemas['RuntimeSecretPoolUpdate']>;
export type RuntimeSettingInfo = Omit<
  ApiSchemas['RuntimeSettingInfo'],
  'input_type' | 'secret_items' | 'source' | 'updated_at'
> & {
  input_type: 'text' | 'password' | 'email' | 'boolean';
  secret_items: RuntimeSecretItemInfo[];
  source: 'database' | 'default';
  updated_at: number | null;
};
export type RuntimeSettingsUpdate = {
  values: NonNullable<ApiSchemas['RuntimeSettingsUpdate']['values']>;
  secret_pool_updates: Record<string, RuntimeSecretPoolUpdate>;
};
export type ScheduledJobSpec = ApiSchemas['ScheduledJobSpec'];
export type ScheduledTaskInfo = Omit<
  ApiSchemas['ScheduledTaskInfo'],
  'job' | 'last_run_at' | 'legacy_command'
> & {
  job: ScheduledJobSpec | null;
  last_run_at: number | null;
  legacy_command: string | null;
};
export type ScheduledTaskCreate = Required<ApiSchemas['ScheduledTaskCreate']>;
export type ScheduledTaskUpdate = ApiSchemas['ScheduledTaskUpdate'];
export type ScheduledTaskRunInfo = Omit<
  ApiSchemas['ScheduledTaskRunInfo'],
  'claimed_at' | 'finished_at' | 'started_at' | 'worker_id'
> & {
  claimed_at: number | null;
  finished_at: number | null;
  started_at: number | null;
  worker_id: string | null;
};
export type SchedulerWorkerInfo = ApiSchemas['SchedulerWorkerInfo'];
export type SchedulerStatus = Omit<
  ApiSchemas['SchedulerStatusResponse'],
  'last_checked_at' | 'recent_runs' | 'workers'
> & {
  last_checked_at: number | null;
  recent_runs: ScheduledTaskRunInfo[];
  workers: SchedulerWorkerInfo[];
};

/**
 * Convert an unknown JSON value into a trusted contract type.
 */
export type ContractParser<T> = (value: unknown) => T;

/**
 * Error raised when a successful API response violates its generated contract.
 */
export class ApiContractError extends Error {
  /**
   * Create a contract validation error.
   *
   * @param contractName - Human-readable contract name.
   */
  constructor(contractName: string) {
    super(`Invalid API response for ${contractName}`);
    this.name = 'ApiContractError';
    Object.setPrototypeOf(this, ApiContractError.prototype);
  }
}

/**
 * Return whether an unknown value is a non-null object record.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a record.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}

/**
 * Return whether a value is a string or null.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is nullable text.
 */
function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

/**
 * Return whether a value is a finite number or null.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a nullable finite number.
 */
function isNullableNumber(value: unknown): value is number | null {
  return value === null || (typeof value === 'number' && Number.isFinite(value));
}

/**
 * Return whether a value is a finite number.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a finite number.
 */
function isNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/**
 * Return whether a value is an array of strings.
 *
 * @param value - Value to inspect.
 * @returns Whether every item is text.
 */
function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

/**
 * Return whether every array item satisfies a type guard.
 *
 * @param value - Value to inspect.
 * @param guard - Item-level type guard.
 * @returns Whether the value is an array of guarded items.
 */
function isArrayOf<T>(value: unknown, guard: (item: unknown) => item is T): value is T[] {
  return Array.isArray(value) && value.every(guard);
}

/**
 * Parse one contract using its type guard.
 *
 * @param value - Unknown JSON payload.
 * @param contractName - Human-readable contract name.
 * @param guard - Contract type guard.
 * @returns Validated contract payload.
 */
function parseContract<T>(
  value: unknown,
  contractName: string,
  guard: (candidate: unknown) => candidate is T,
): T {
  if (!guard(value)) {
    throw new ApiContractError(contractName);
  }
  return value;
}

/**
 * Return whether a value is an authenticated user response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated user contract.
 */
function isAuthUser(value: unknown): value is AuthUser {
  return (
    isRecord(value) &&
    isNumber(value.id) &&
    typeof value.username === 'string' &&
    typeof value.is_admin === 'boolean'
  );
}

/**
 * Return whether a value is a login response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated login contract.
 */
function isLoginResponse(value: unknown): value is LoginResponse {
  return isRecord(value) && isNumber(value.expires_at) && isAuthUser(value.user);
}

/**
 * Return whether a value is an invite requirement response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated invite contract.
 */
function isInviteRequirement(value: unknown): value is InviteRequirement {
  return (
    isRecord(value) &&
    typeof value.required === 'boolean' &&
    typeof value.bootstrap_required === 'boolean'
  );
}

/**
 * Return whether a value is a tracking-folder summary.
 *
 * @param value - Value to inspect.
 * @returns Whether the value contains a numeric id and text name.
 */
function isTrackingFolder(value: unknown): value is { id: number; name: string } {
  return isRecord(value) && isNumber(value.id) && typeof value.name === 'string';
}

/**
 * Return whether a value is a tracking status response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated tracking contract.
 */
function isTrackingStatus(value: unknown): value is TrackingStatus {
  return (
    isRecord(value) &&
    (value.tracking_folder === null || isTrackingFolder(value.tracking_folder)) &&
    isNumber(value.total_folders) &&
    isNumber(value.weekly_articles_available) &&
    typeof value.notification_configured === 'boolean'
  );
}

/**
 * Return whether a value is a manual background push status.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated background status contract.
 */
function isManualPushStatus(value: unknown): value is ManualPushStatus {
  return (
    isRecord(value) &&
    isNullableString(value.job_id) &&
    ['idle', 'running', 'completed', 'failed'].includes(String(value.status)) &&
    typeof value.message === 'string' &&
    isNullableNumber(value.started_at) &&
    isNullableNumber(value.finished_at) &&
    isNumber(value.pushed) &&
    isNumber(value.selected) &&
    isNullableNumber(value.total_candidates) &&
    typeof value.summary === 'string' &&
    isNullableNumber(value.folder_id) &&
    isNullableString(value.folder_name)
  );
}

const NOTIFICATION_STRING_FIELDS = [
  'delivery_method',
  'pushplus_token_mask',
  'pushplus_template',
  'pushplus_topic',
  'pushplus_channel',
  'ai_base_url',
  'ai_api_key_mask',
  'ai_model',
  'ai_system_prompt',
  'ai_backup_base_url',
  'ai_backup_api_key_mask',
  'ai_backup_model',
  'ai_backup_system_prompt',
] as const;

/**
 * Return whether a value is a notification settings response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated secret-setting contract.
 */
function isNotificationSettings(value: unknown): value is NotificationSettings {
  return (
    isRecord(value) &&
    isNumber(value.id) &&
    isNumber(value.user_id) &&
    isStringArray(value.keywords) &&
    isStringArray(value.directions) &&
    isStringArray(value.selected_databases) &&
    !('pushplus_token' in value) &&
    !('ai_api_key' in value) &&
    !('ai_backup_api_key' in value) &&
    NOTIFICATION_STRING_FIELDS.every((field) => typeof value[field] === 'string') &&
    typeof value.has_pushplus_token === 'boolean' &&
    typeof value.has_ai_api_key === 'boolean' &&
    typeof value.has_ai_backup_api_key === 'boolean' &&
    typeof value.sync_to_tracking_folder === 'boolean' &&
    isNumber(value.ai_retry_attempts) &&
    typeof value.enabled === 'boolean' &&
    isNumber(value.created_at) &&
    isNumber(value.updated_at)
  );
}

/**
 * Return whether a value is one secret-pool item descriptor.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated secret-pool item contract.
 */
function isRuntimeSecretItemInfo(value: unknown): value is RuntimeSecretItemInfo {
  return (
    isRecord(value) && typeof value.reference === 'string' && typeof value.masked_value === 'string'
  );
}

/**
 * Return whether a value is one runtime setting descriptor.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated runtime-setting contract.
 */
function isRuntimeSettingInfo(value: unknown): value is RuntimeSettingInfo {
  return (
    isRecord(value) &&
    typeof value.field === 'string' &&
    typeof value.label === 'string' &&
    typeof value.description === 'string' &&
    ['text', 'password', 'email', 'boolean'].includes(String(value.input_type)) &&
    typeof value.is_secret === 'boolean' &&
    typeof value.value === 'string' &&
    typeof value.has_value === 'boolean' &&
    typeof value.masked_value === 'string' &&
    isArrayOf(value.secret_items, isRuntimeSecretItemInfo) &&
    ['database', 'default'].includes(String(value.source)) &&
    isNullableNumber(value.updated_at)
  );
}

/**
 * Return whether a value is one scheduled task response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated scheduled-task contract.
 */
function isScheduledTaskInfo(value: unknown): value is ScheduledTaskInfo {
  if (!isRecord(value)) {
    return false;
  }
  const hasTypedJob = isScheduledJobSpec(value.job) && value.legacy_command === null;
  const hasLegacyCommand = value.job === null && typeof value.legacy_command === 'string';
  return (
    isNumber(value.id) &&
    typeof value.name === 'string' &&
    (hasTypedJob || hasLegacyCommand) &&
    typeof value.cron === 'string' &&
    typeof value.timezone === 'string' &&
    Number.isInteger(value.timeout_seconds) &&
    Number(value.timeout_seconds) >= 1 &&
    Number(value.timeout_seconds) <= 86_400 &&
    typeof value.coalesce === 'boolean' &&
    typeof value.enabled === 'boolean' &&
    isNullableNumber(value.last_run_at) &&
    typeof value.last_status === 'string' &&
    isNumber(value.created_at) &&
    isNumber(value.updated_at)
  );
}

/**
 * Return whether a value is one durable scheduler run response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated scheduler-run contract.
 */
function isScheduledTaskRunInfo(value: unknown): value is ScheduledTaskRunInfo {
  return (
    isRecord(value) &&
    isNumber(value.id) &&
    isNumber(value.task_id) &&
    typeof value.task_name === 'string' &&
    isNumber(value.scheduled_for) &&
    [
      'pending',
      'claimed',
      'running',
      'success',
      'failed',
      'timed_out',
      'error',
      'unknown',
    ].includes(String(value.status)) &&
    isNullableString(value.worker_id) &&
    isNullableNumber(value.claimed_at) &&
    isNullableNumber(value.started_at) &&
    isNullableNumber(value.finished_at)
  );
}

/**
 * Return whether a value is one persisted scheduler worker heartbeat.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated scheduler-worker contract.
 */
function isSchedulerWorkerInfo(value: unknown): value is SchedulerWorkerInfo {
  return (
    isRecord(value) &&
    typeof value.worker_id === 'string' &&
    isNumber(value.started_at) &&
    isNumber(value.heartbeat_at) &&
    typeof value.is_healthy === 'boolean'
  );
}

/**
 * Return whether a value is the durable scheduler status response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated scheduler-status contract.
 */
function isSchedulerStatus(value: unknown): value is SchedulerStatus {
  return (
    isRecord(value) &&
    isNullableNumber(value.last_checked_at) &&
    isArrayOf(value.workers, isSchedulerWorkerInfo) &&
    isArrayOf(value.recent_runs, isScheduledTaskRunInfo)
  );
}

/**
 * Return whether a value is one strictly typed scheduler job.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches a generated scheduler job variant.
 */
function isScheduledJobSpec(value: unknown): value is ScheduledJobSpec {
  if (!isRecord(value) || typeof value.kind !== 'string') {
    return false;
  }
  if (value.kind === 'index') {
    return (
      (!('metadata_file' in value) || isNullableString(value.metadata_file)) &&
      (!('notify' in value) || typeof value.notify === 'boolean') &&
      (!('push' in value) || typeof value.push === 'boolean')
    );
  }
  if (value.kind === 'notify' || value.kind === 'push') {
    return (
      (!('database' in value) || isNullableString(value.database)) &&
      (!('max_candidates' in value) || isNullableNumber(value.max_candidates))
    );
  }
  return false;
}

/**
 * Parse an authenticated user response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated user response.
 */
export function parseAuthUser(value: unknown): AuthUser {
  return parseContract(value, 'UserResponse', isAuthUser);
}

/**
 * Parse a login response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated login response.
 */
export function parseLoginResponse(value: unknown): LoginResponse {
  return parseContract(value, 'LoginResponse', isLoginResponse);
}

/**
 * Parse an invite requirement response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated invite requirement.
 */
export function parseInviteRequirement(value: unknown): InviteRequirement {
  return parseContract(value, 'InviteRequiredResponse', isInviteRequirement);
}

/**
 * Parse a tracking status response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated tracking status.
 */
export function parseTrackingStatus(value: unknown): TrackingStatus {
  return parseContract(value, 'TrackingStatusResponse', isTrackingStatus);
}

/**
 * Parse a manual background push status response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated manual push status.
 */
export function parseManualPushStatus(value: unknown): ManualPushStatus {
  return parseContract(value, 'ManualWeeklyPushStatus', isManualPushStatus);
}

/**
 * Parse nullable notification settings.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated settings or null when unconfigured.
 */
export function parseNullableNotificationSettings(value: unknown): NotificationSettings | null {
  if (value === null) {
    return null;
  }
  return parseContract(value, 'NotificationSettingsResponse', isNotificationSettings);
}

/**
 * Parse notification settings.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated notification settings.
 */
export function parseNotificationSettings(value: unknown): NotificationSettings {
  return parseContract(value, 'NotificationSettingsResponse', isNotificationSettings);
}

/**
 * Parse a list of runtime setting descriptors.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated runtime setting list.
 */
export function parseRuntimeSettingList(value: unknown): RuntimeSettingInfo[] {
  return parseContract(
    value,
    'RuntimeSettingInfo[]',
    (candidate): candidate is RuntimeSettingInfo[] => isArrayOf(candidate, isRuntimeSettingInfo),
  );
}

/**
 * Parse one scheduled task response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated scheduled task.
 */
export function parseScheduledTaskInfo(value: unknown): ScheduledTaskInfo {
  return parseContract(value, 'ScheduledTaskInfo', isScheduledTaskInfo);
}

/**
 * Parse a list of scheduled task responses.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated scheduled task list.
 */
export function parseScheduledTaskList(value: unknown): ScheduledTaskInfo[] {
  return parseContract(
    value,
    'ScheduledTaskInfo[]',
    (candidate): candidate is ScheduledTaskInfo[] => isArrayOf(candidate, isScheduledTaskInfo),
  );
}

/**
 * Parse the durable scheduler status response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated scheduler status.
 */
export function parseSchedulerStatus(value: unknown): SchedulerStatus {
  return parseContract(value, 'SchedulerStatusResponse', isSchedulerStatus);
}
