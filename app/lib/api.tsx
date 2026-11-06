/**
 * Compatibility facade for the domain-oriented LitRadar browser API client.
 */

export type {
  AuthKind,
  AuthProviders,
  AuthUser,
  InviteRequirement,
  LoginResponse,
  ManualPushState,
  ManualPushStatus,
  NotificationSettings,
  NotificationSettingsUpdate,
  OidcLinkStart,
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
  SchedulerRunState,
  ScheduledJobSpec,
  ScheduledTaskCreate,
  ScheduledTaskInfo,
  ScheduledTaskUpdate,
  TrackingStatus,
} from '@/lib/api-contract';
export {
  ApiError,
  DEFAULT_DATABASE,
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
export * from '@/lib/api/cfp';
export * from '@/lib/api/favorites';
export * from '@/lib/api/index';
export * from '@/lib/api/tracking';
export * from '@/lib/api/types';
