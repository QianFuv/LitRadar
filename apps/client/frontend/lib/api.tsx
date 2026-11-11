/**
 * Compatibility facade for the domain-oriented LitRadar browser API client.
 */

export type {
  AuthUser,
  ManualPushState,
  ManualPushStatus,
  NotificationSettingsUpdate,
  IndexProviderRoutes,
  ProviderCapabilityInfo,
  ProviderCatalogResponse,
  ProviderOrderConfiguration,
  RuntimeSettingInfo,
  RuntimeSettingApplyMode,
  RuntimeSettingGroup,
  RuntimeSettingsUpdate,
  SchedulerRunState,
  ScheduledJobSpec,
  ScheduledTaskInfo,
} from '@/lib/api-contract';
export { DEFAULT_DATABASE, SELECTED_DATABASE_KEY, readSelectedDatabase } from '@/lib/api/client';
export * from '@/lib/api/admin';
export * from '@/lib/api/auth';
export * from '@/lib/api/cfp';
export * from '@/lib/api/favorites';
export * from '@/lib/api/index';
export * from '@/lib/api/tracking';
export * from '@/lib/api/types';

export type { NotificationSettings } from '@/lib/api-contract';
export type { ProviderCatalogInfo } from '@/lib/api-contract';
export type { ScheduledTaskCreate } from '@/lib/api-contract';
export { ApiError } from '@/lib/api/client';
