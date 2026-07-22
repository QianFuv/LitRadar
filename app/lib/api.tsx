/**
 * Compatibility facade for the domain-oriented LitRadar browser API client.
 */

export type {
  AuthUser,
  InviteRequirement,
  LoginResponse,
  ManualPushStatus,
  NotificationSettings,
  NotificationSettingsUpdate,
  IndexProviderRoutes,
  ProviderCapabilityInfo,
  ProviderCatalogInfo,
  ProviderCatalogResponse,
  ProviderOrderConfiguration,
  RuntimeSettingInfo,
  RuntimeSettingApplyMode,
  RuntimeSettingControl,
  RuntimeSettingGroup,
  RuntimeSettingsUpdate,
  SchedulerStatus,
  ScheduledJobSpec,
  ScheduledTaskCreate,
  ScheduledTaskInfo,
  ScheduledTaskUpdate,
  TrackingStatus,
} from '@/lib/api-contract';
export {
  ApiError,
  DEFAULT_DATABASE,
  DEFAULT_DB,
  SELECTED_DATABASE_KEY,
  buildApiUrl,
  buildDatabaseUrl,
  readSelectedDatabase,
  resolveApiBase,
  storeSelectedDatabase,
  type ApiErrorInfo,
} from '@/lib/api/client';
export * from '@/lib/api/admin';
export * from '@/lib/api/auth';
export * from '@/lib/api/favorites';
export * from '@/lib/api/index';
export * from '@/lib/api/tracking';
export * from '@/lib/api/types';
