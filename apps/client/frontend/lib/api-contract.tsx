/**
 * Generated API aliases and runtime decoders for security-sensitive responses.
 */

import type { components } from '@/lib/generated/api-schema';
import { isBoolean, isRecord, isString } from '@/lib/type-guards';

type ApiSchemas = components['schemas'];

export type AuthUser = ApiSchemas['UserResponse'];
export type LoginResponse = ApiSchemas['LoginResponse'];
export type AuthProviders = ApiSchemas['AuthProvidersResponse'];
export type OidcLinkStart = ApiSchemas['OidcLinkResponse'];
export type InviteRequirement = ApiSchemas['InviteRequiredResponse'];
export type InviteCodeStatus = ApiSchemas['InviteCodeStatus'];
export type InviteCode = Required<ApiSchemas['InviteCodeResponse']>;
export type AdminInviteCode = Required<ApiSchemas['AdminInviteCodeInfo']>;
type GeneratedAdminInviteCodeCreate = ApiSchemas['AdminInviteCodeCreate'];
export type AdminInviteCodeCreate = {
  expires_at?: Exclude<GeneratedAdminInviteCodeCreate['expires_at'], null | undefined>;
  max_uses?: Exclude<GeneratedAdminInviteCodeCreate['max_uses'], null | undefined>;
};
export type TrackingStatus = ApiSchemas['TrackingStatusResponse'];
export type ManualPushStatus = ApiSchemas['ManualWeeklyPushStatus'];
export type ManualPushState = ApiSchemas['ManualPushState'];
export type SchedulerRunState = ApiSchemas['SchedulerRunState'];
export type NotificationSettings = ApiSchemas['NotificationSettingsResponse'];
type GeneratedNotificationSettingsUpdate = ApiSchemas['NotificationSettingsUpdate'];
type NotificationSecretField = 'ai_api_key' | 'ai_backup_api_key' | 'pushplus_token';
export type NotificationSettingsUpdate = Required<
  Omit<GeneratedNotificationSettingsUpdate, NotificationSecretField>
> &
  Pick<GeneratedNotificationSettingsUpdate, NotificationSecretField>;
type RuntimeSecretItemInfo = ApiSchemas['RuntimeSecretItemInfo'];
type RuntimeSecretPoolUpdate = Required<ApiSchemas['RuntimeSecretPoolUpdate']>;
export type RuntimeSettingApplyMode = ApiSchemas['RuntimeSettingApplyMode'];
export type RuntimeSettingGroup = ApiSchemas['RuntimeSettingGroup'];
type RuntimeSettingControl = string;
export type RuntimeSettingInfo = Omit<
  ApiSchemas['RuntimeSettingInfo'],
  'control' | 'input_type' | 'secret_items' | 'source' | 'updated_at'
> & {
  control: RuntimeSettingControl;
  input_type: 'text' | 'password' | 'email' | 'boolean' | 'number' | 'url';
  secret_items: RuntimeSecretItemInfo[];
  source: 'database' | 'default';
  updated_at: number | null;
};
export type ProviderCapabilityInfo = ApiSchemas['ProviderCapabilityInfo'];
export type ProviderCatalogInfo = Omit<
  ApiSchemas['ProviderCatalogInfo'],
  'csv_filename' | 'database_filename'
> & {
  csv_filename: string | null;
  database_filename: string | null;
};
export type ProviderCatalogResponse = {
  providers: ProviderCapabilityInfo[];
  catalogs: ProviderCatalogInfo[];
};
export type ProviderProxyPolicy = Record<string, boolean>;
export type ProviderOrderConfiguration = ApiSchemas['ProviderOrderConfiguration'];
export type IndexProviderRoutes = Record<string, string>;
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
type ScheduledTaskRunInfo = Omit<
  ApiSchemas['ScheduledTaskRunInfo'],
  'claimed_at' | 'finished_at' | 'started_at' | 'worker_id'
> & {
  claimed_at: number | null;
  finished_at: number | null;
  started_at: number | null;
  worker_id: string | null;
};
type SchedulerWorkerInfo = ApiSchemas['SchedulerWorkerInfo'];
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
 * Return whether a value is a string or null.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is nullable text.
 */
function isNullableString(value: unknown): value is string | null {
  return value === null || isString(value);
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
  return Array.isArray(value) && value.every(isString);
}

/**
 * Return whether a record contains exactly the expected keys.
 *
 * @param value - Record to inspect.
 * @param expectedKeys - Required key names.
 * @returns Whether no required or extra keys are present.
 */
function hasExactKeys(value: Record<string, unknown>, expectedKeys: string[]): boolean {
  const actualKeys = Object.keys(value).sort();
  const sortedExpectedKeys = [...expectedKeys].sort();
  return (
    actualKeys.length === sortedExpectedKeys.length &&
    actualKeys.every((key, index) => key === sortedExpectedKeys[index])
  );
}

/**
 * Return whether text uses the safe runtime identifier grammar.
 *
 * @param value - Text to inspect.
 * @returns Whether the text is a safe Provider or catalog identifier.
 */
function isSafeRuntimeName(value: unknown): value is string {
  return (
    isString(value) &&
    value.length >= 2 &&
    value.length <= 128 &&
    /^[a-z0-9][a-z0-9._-]*$/.test(value)
  );
}

/**
 * Return whether a string array contains only safe unique runtime identifiers.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a valid ordered Provider list.
 */
function isSafeUniqueRuntimeNameArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every(isSafeRuntimeName) && new Set(value).size === value.length
  );
}

/**
 * Return whether a value is a declared runtime setting group.
 *
 * @param value - Value to inspect.
 * @returns Whether the group is supported by this frontend.
 */
function isRuntimeSettingGroup(value: unknown): value is RuntimeSettingGroup {
  return (
    isString(value) &&
    [
      'source_access',
      'provider_routing',
      'server_security',
      'observability',
      'single_sign_on',
    ].includes(value)
  );
}

/**
 * Return whether a value is a declared runtime setting apply mode.
 *
 * @param value - Value to inspect.
 * @returns Whether the apply mode is supported by this frontend.
 */
function isRuntimeSettingApplyMode(value: unknown): value is RuntimeSettingApplyMode {
  return isString(value) && ['next_request', 'next_command', 'restart_required'].includes(value);
}

/**
 * Return whether a value is a structurally valid Provider-order configuration.
 *
 * @param value - Value to inspect.
 * @returns Whether the value has one default order and safe catalog overrides.
 */
function isProviderOrderConfiguration(value: unknown): value is ProviderOrderConfiguration {
  if (!isRecord(value) || !hasExactKeys(value, ['default', 'catalogs'])) {
    return false;
  }
  if (!isSafeUniqueRuntimeNameArray(value.default) || !isRecord(value.catalogs)) {
    return false;
  }
  return Object.entries(value.catalogs).every(
    ([catalog, providers]) => isSafeRuntimeName(catalog) && isSafeUniqueRuntimeNameArray(providers),
  );
}

/**
 * Return whether a value is a structurally valid index Provider route map.
 *
 * @param value - Value to inspect.
 * @returns Whether every catalog maps to one safe Provider name.
 */
function isIndexProviderRoutes(value: unknown): value is Record<string, string> {
  return (
    isRecord(value) &&
    Object.keys(value).length > 0 &&
    Object.entries(value).every(
      ([catalog, provider]) => isSafeRuntimeName(catalog) && isSafeRuntimeName(provider),
    )
  );
}

/**
 * Return whether a value is a structurally valid Provider proxy policy.
 *
 * @param value - Value to inspect.
 * @returns Whether every safe Provider name maps to one boolean decision.
 */
function isProviderProxyPolicy(value: unknown): value is ProviderProxyPolicy {
  return (
    isRecord(value) &&
    Object.entries(value).every(
      ([provider, isEnabled]) => isSafeRuntimeName(provider) && isBoolean(isEnabled),
    )
  );
}

/**
 * Parse JSON text without allowing syntax failures to escape as implementation errors.
 *
 * @param value - JSON text.
 * @param contractName - Human-readable contract name.
 * @returns Parsed unknown JSON value.
 */
function parseContractJson(value: string, contractName: string): unknown {
  try {
    return JSON.parse(value) as unknown;
  } catch {
    throw new ApiContractError(contractName);
  }
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
    isString(value.username) &&
    isBoolean(value.is_admin) &&
    isNullableString(value.email) &&
    (value.auth_kind === 'local' || value.auth_kind === 'oidc') &&
    isBoolean(value.is_oidc_linked)
  );
}

/**
 * Return whether a value is a sign-in provider summary.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated providers contract.
 */
function isAuthProviders(value: unknown): value is AuthProviders {
  return isRecord(value) && isBoolean(value.oidc_enabled) && isString(value.oidc_display_name);
}

/**
 * Return whether a value is an OIDC link start response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value carries a non-empty authorization URL.
 */
function isOidcLinkStart(value: unknown): value is OidcLinkStart {
  return isRecord(value) && isString(value.authorization_url) && value.authorization_url.length > 0;
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
  return isRecord(value) && isBoolean(value.required) && isBoolean(value.bootstrap_required);
}

/**
 * Return whether a value is a tracking-folder summary.
 *
 * @param value - Value to inspect.
 * @returns Whether the value contains a numeric id and text name.
 */
function isTrackingFolder(value: unknown): value is { id: number; name: string } {
  return isRecord(value) && isNumber(value.id) && isString(value.name);
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
    isBoolean(value.notification_configured)
  );
}

/**
 * Validate push identity and the declared state before lifecycle fields.
 */
function hasManualPushIdentity(value: Record<string, unknown>): boolean {
  return (
    isNullableString(value.job_id) && isManualPushState(value.status) && isString(value.message)
  );
}

/**
 * Validate nullable timestamps and cancellation capabilities.
 */
function hasManualPushLifecycle(value: Record<string, unknown>): boolean {
  return (
    isNullableNumber(value.started_at) &&
    isNullableNumber(value.finished_at) &&
    isNullableNumber(value.deadline_at) &&
    isBoolean(value.cancellation_requested) &&
    isBoolean(value.can_cancel) &&
    isBoolean(value.can_retry)
  );
}

/**
 * Validate push counts, summary and nullable destination metadata.
 */
function hasManualPushDelivery(value: Record<string, unknown>): boolean {
  return (
    isNumber(value.pushed) &&
    isNumber(value.selected) &&
    isNullableNumber(value.total_candidates) &&
    isString(value.summary) &&
    isNullableNumber(value.folder_id) &&
    isNullableString(value.folder_name)
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
    hasManualPushIdentity(value) &&
    hasManualPushLifecycle(value) &&
    hasManualPushDelivery(value)
  );
}

/**
 * Return whether a value is a declared manual-push state.
 *
 * @param value - Value to inspect.
 * @returns Whether the state is generated by the backend contract.
 */
function isManualPushState(value: unknown): value is ManualPushState {
  return (
    isString(value) &&
    [
      'idle',
      'pending',
      'running',
      'completed',
      'failed',
      'cancelled',
      'timed_out',
      'unknown',
    ].includes(value)
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
 * Validate notification ownership and ordered selection arrays.
 */
function hasNotificationSelection(value: Record<string, unknown>): boolean {
  return (
    isNumber(value.id) &&
    isNumber(value.user_id) &&
    isStringArray(value.keywords) &&
    isStringArray(value.directions) &&
    isStringArray(value.selected_databases)
  );
}

/**
 * Reject plaintext secret properties and require masked string fields.
 */
function hasRedactedNotificationFields(value: Record<string, unknown>): boolean {
  return (
    !('pushplus_token' in value) &&
    !('ai_api_key' in value) &&
    !('ai_backup_api_key' in value) &&
    NOTIFICATION_STRING_FIELDS.every((field) => isString(value[field]))
  );
}

/**
 * Validate stored-secret flags, delivery controls and timestamps.
 */
function hasNotificationLifecycle(value: Record<string, unknown>): boolean {
  return (
    isBoolean(value.has_pushplus_token) &&
    isBoolean(value.has_ai_api_key) &&
    isBoolean(value.has_ai_backup_api_key) &&
    isBoolean(value.sync_to_tracking_folder) &&
    isNumber(value.ai_retry_attempts) &&
    isBoolean(value.enabled) &&
    isNumber(value.created_at) &&
    isNumber(value.updated_at)
  );
}

/**
 * Return whether a value is a notification settings response.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches the generated secret-setting contract.
 */
function isNotificationSettings(value: unknown): value is NotificationSettings {
  return (
    isRecord(value) &&
    hasNotificationSelection(value) &&
    hasRedactedNotificationFields(value) &&
    hasNotificationLifecycle(value)
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
    isRecord(value) &&
    isString(value.reference) &&
    value.reference.length > 0 &&
    isString(value.masked_value)
  );
}

/**
 * Return whether a runtime setting value matches its declared specialized control.
 *
 * @param control - Declared control identifier.
 * @param value - Effective non-secret value.
 * @returns Whether specialized JSON and boolean values are structurally valid.
 */
function isRuntimeControlValue(control: string, value: string): boolean {
  if (control === 'boolean') {
    return value === 'true' || value === 'false';
  }
  if (
    control !== 'index_provider_routes' &&
    control !== 'provider_order' &&
    control !== 'provider_proxy_policy'
  ) {
    return true;
  }
  try {
    const parsed = JSON.parse(value) as unknown;
    if (control === 'index_provider_routes') {
      return isIndexProviderRoutes(parsed);
    }
    return control === 'provider_order'
      ? isProviderOrderConfiguration(parsed)
      : isProviderProxyPolicy(parsed);
  } catch {
    return false;
  }
}

/**
 * Recognize controls whose metadata belongs to Provider routing.
 */
function isProviderRuntimeControl(control: string): boolean {
  return (
    control === 'index_provider_routes' ||
    control === 'provider_order' ||
    control === 'provider_proxy_policy'
  );
}

/**
 * Validate reserved field-to-control bindings in their original order.
 */
function hasRuntimeControlBinding(field: string, control: string): boolean {
  return !(
    (field === 'index_provider_routes' && control !== 'index_provider_routes') ||
    ((field === 'article_abstract_provider_orders' ||
      field === 'article_fulltext_provider_orders') &&
      control !== 'provider_order') ||
    (field === 'provider_proxy_policy' && control !== 'provider_proxy_policy') ||
    (control === 'provider_proxy_policy' && field !== 'provider_proxy_policy')
  );
}

/**
 * Require Provider controls to declare the routing metadata group.
 */
function hasRuntimeRoutingGroup(
  value: Record<string, unknown>,
  isProviderControl: boolean,
): boolean {
  return !(isProviderControl && value.group !== 'provider_routing');
}

/**
 * Require the scalar proxy URL to remain a restart-applied password secret.
 */
function hasRuntimeProxyUrlMetadata(
  value: Record<string, unknown>,
  field: string,
  control: string,
  inputType: string,
  isSecret: boolean,
): boolean {
  return !(
    field === 'provider_proxy_url' &&
    (control !== 'text' ||
      value.group !== 'source_access' ||
      value.apply_mode !== 'restart_required' ||
      inputType !== 'password' ||
      !isSecret)
  );
}

/**
 * Require proxy policy metadata to be non-secret restart-applied text.
 */
function hasRuntimeProxyPolicyMetadata(
  value: Record<string, unknown>,
  control: string,
  inputType: string,
  isSecret: boolean,
  allowedValues: string[],
): boolean {
  return !(
    control === 'provider_proxy_policy' &&
    (value.apply_mode !== 'restart_required' ||
      inputType !== 'text' ||
      isSecret ||
      allowedValues.length !== 0)
  );
}

/**
 * Validate boolean input pairing and ordered boolean or select options.
 */
function hasRuntimeControlOptions(
  control: string,
  inputType: string,
  allowedValues: string[],
): boolean {
  if ((control === 'boolean') !== (inputType === 'boolean')) {
    return false;
  }
  if (control === 'boolean') {
    if (allowedValues.length !== 2 || allowedValues[0] !== 'true' || allowedValues[1] !== 'false') {
      return false;
    }
  }
  if (control === 'select' && allowedValues.length === 0) {
    return false;
  }
  return true;
}

/**
 * Return whether runtime metadata is internally consistent and secret-safe.
 *
 * @param value - Runtime setting descriptor.
 * @returns Whether control, group, values, and redaction metadata agree.
 */
function isConsistentRuntimeMetadata(value: Record<string, unknown>): boolean {
  const control = String(value.control);
  const field = String(value.field);
  const inputType = String(value.input_type);
  const isSecret = value.is_secret === true;
  const hasValue = value.has_value === true;
  const allowedValues = value.allowed_values as string[];
  const secretItems = value.secret_items as RuntimeSecretItemInfo[];
  const isProviderControl = isProviderRuntimeControl(control);
  if (
    !hasRuntimeControlBinding(field, control) ||
    !hasRuntimeRoutingGroup(value, isProviderControl) ||
    !hasRuntimeProxyUrlMetadata(value, field, control, inputType, isSecret) ||
    !hasRuntimeProxyPolicyMetadata(value, control, inputType, isSecret, allowedValues) ||
    !hasRuntimeControlOptions(control, inputType, allowedValues)
  ) {
    return false;
  }
  if (control === 'secret_pool') {
    return (
      isSecret &&
      inputType === 'password' &&
      allowedValues.length === 0 &&
      value.value === '' &&
      (hasValue
        ? value.masked_value === '••••' && secretItems.length > 0
        : value.masked_value === '' && secretItems.length === 0)
    );
  }
  if (isSecret) {
    return (
      control === 'text' &&
      inputType === 'password' &&
      allowedValues.length === 0 &&
      value.value === '' &&
      secretItems.length === 0 &&
      value.masked_value === (hasValue ? '••••' : '')
    );
  }
  return value.masked_value === '' && secretItems.length === 0;
}

/**
 * Validate runtime names and nonempty descriptive metadata.
 */
function hasRuntimeSettingIdentity(value: Record<string, unknown>): boolean {
  return (
    isSafeRuntimeName(value.field) &&
    isString(value.label) &&
    value.label.length > 0 &&
    isString(value.description) &&
    value.description.length > 0 &&
    isRuntimeSettingGroup(value.group)
  );
}

/**
 * Validate control metadata, unique options and supported input types.
 */
function hasRuntimeSettingControls(value: Record<string, unknown>): boolean {
  return (
    isSafeRuntimeName(value.control) &&
    isRuntimeSettingApplyMode(value.apply_mode) &&
    isStringArray(value.allowed_values) &&
    new Set(value.allowed_values).size === value.allowed_values.length &&
    isString(value.input_type) &&
    ['text', 'password', 'email', 'boolean', 'number', 'url'].includes(value.input_type)
  );
}

/**
 * Validate values, redaction descriptors and persistence metadata.
 */
function hasRuntimeSettingStorage(value: Record<string, unknown>): boolean {
  return (
    isBoolean(value.is_secret) &&
    isString(value.value) &&
    isBoolean(value.has_value) &&
    isString(value.masked_value) &&
    isArrayOf(value.secret_items, isRuntimeSecretItemInfo) &&
    isString(value.source) &&
    ['database', 'default'].includes(value.source) &&
    isNullableNumber(value.updated_at)
  );
}

/**
 * Validate specialized values before checking cross-field consistency.
 */
function hasValidRuntimeSettingMetadata(value: Record<string, unknown>): boolean {
  return (
    isRuntimeControlValue(value.control as string, value.value as string) &&
    isConsistentRuntimeMetadata(value)
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
    hasRuntimeSettingIdentity(value) &&
    hasRuntimeSettingControls(value) &&
    hasRuntimeSettingStorage(value) &&
    hasValidRuntimeSettingMetadata(value)
  );
}

/**
 * Return whether a value describes one Provider and its aggregate capabilities.
 *
 * @param value - Value to inspect.
 * @returns Whether the Provider descriptor is complete and useful.
 */
function isProviderCapabilityInfo(value: unknown): value is ProviderCapabilityInfo {
  return (
    isRecord(value) &&
    hasExactKeys(value, ['name', 'index_content', 'article_abstract', 'article_full_text']) &&
    isSafeRuntimeName(value.name) &&
    isBoolean(value.index_content) &&
    isBoolean(value.article_abstract) &&
    isBoolean(value.article_full_text) &&
    (value.index_content || value.article_abstract || value.article_full_text)
  );
}

/**
 * Validate exact catalog keys and safe nullable filename fields.
 */
function hasProviderCatalogFields(value: Record<string, unknown>): boolean {
  return !(
    !hasExactKeys(value, ['stem', 'csv_filename', 'database_filename']) ||
    !isSafeRuntimeName(value.stem) ||
    !isNullableString(value.csv_filename) ||
    !isNullableString(value.database_filename)
  );
}

/**
 * Return whether a value is safe catalog filename metadata.
 *
 * @param value - Value to inspect.
 * @returns Whether filenames are basenames matching the canonical catalog stem.
 */
function isProviderCatalogInfo(value: unknown): value is ProviderCatalogInfo {
  if (!isRecord(value) || !hasProviderCatalogFields(value)) {
    return false;
  }
  return (
    isString(value.stem) &&
    (value.csv_filename !== null || value.database_filename !== null) &&
    (value.csv_filename === null || value.csv_filename === `${value.stem}.csv`) &&
    (value.database_filename === null || value.database_filename === `${value.stem}.sqlite`)
  );
}

/**
 * Return whether a value is a complete, duplicate-free Provider catalog response.
 *
 * @param value - Value to inspect.
 * @returns Whether Provider and catalog metadata is safe to render.
 */
function isProviderCatalogResponse(value: unknown): value is ProviderCatalogResponse {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ['providers', 'catalogs']) ||
    !isArrayOf(value.providers, isProviderCapabilityInfo) ||
    !isArrayOf(value.catalogs, isProviderCatalogInfo)
  ) {
    return false;
  }
  return (
    new Set(value.providers.map((provider) => provider.name)).size === value.providers.length &&
    new Set(value.catalogs.map((catalog) => catalog.stem)).size === value.catalogs.length
  );
}

/**
 * Validate cron, timezone and the integer timeout boundaries.
 */
function hasScheduledTaskTiming(value: Record<string, unknown>): boolean {
  return (
    isString(value.cron) &&
    isString(value.timezone) &&
    Number.isInteger(value.timeout_seconds) &&
    Number(value.timeout_seconds) >= 1 &&
    Number(value.timeout_seconds) <= 86_400
  );
}

/**
 * Validate scheduling flags, latest status and persistence timestamps.
 */
function hasScheduledTaskLifecycle(value: Record<string, unknown>): boolean {
  return (
    isBoolean(value.coalesce) &&
    isBoolean(value.enabled) &&
    isNullableNumber(value.last_run_at) &&
    isSchedulerRunState(value.last_status) &&
    isNumber(value.created_at) &&
    isNumber(value.updated_at)
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
  const hasLegacyCommand = value.job === null && isString(value.legacy_command);
  return (
    isNumber(value.id) &&
    isString(value.name) &&
    (hasTypedJob || hasLegacyCommand) &&
    hasScheduledTaskTiming(value) &&
    hasScheduledTaskLifecycle(value)
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
    isString(value.task_name) &&
    isNumber(value.scheduled_for) &&
    isSchedulerRunState(value.status) &&
    isNullableString(value.worker_id) &&
    isNullableNumber(value.claimed_at) &&
    isNullableNumber(value.started_at) &&
    isNullableNumber(value.finished_at)
  );
}

/**
 * Return whether a value is a declared scheduler-run state.
 *
 * @param value - Value to inspect.
 * @returns Whether the state is generated by the backend contract.
 */
function isSchedulerRunState(value: unknown): value is SchedulerRunState {
  return (
    isString(value) &&
    [
      'idle',
      'pending',
      'claimed',
      'running',
      'success',
      'failed',
      'timed_out',
      'error',
      'unknown',
      'cancelled',
    ].includes(value)
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
    isString(value.worker_id) &&
    isNumber(value.started_at) &&
    isNumber(value.heartbeat_at) &&
    isBoolean(value.is_healthy)
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
 * Validate present index job options, retaining omission and null rules.
 */
function hasIndexJobFields(value: Record<string, unknown>): boolean {
  return (
    (!('metadata_file' in value) || isNullableString(value.metadata_file)) &&
    (!('notify' in value) || isBoolean(value.notify)) &&
    (!('push' in value) || isBoolean(value.push))
  );
}

/**
 * Validate present delivery job database and candidate options.
 */
function hasDeliveryJobFields(value: Record<string, unknown>): boolean {
  return (
    (!('database' in value) || isNullableString(value.database)) &&
    (!('max_candidates' in value) || isNullableNumber(value.max_candidates))
  );
}

/**
 * Return whether a value is one strictly typed scheduler job.
 *
 * @param value - Value to inspect.
 * @returns Whether the value matches a generated scheduler job variant.
 */
function isScheduledJobSpec(value: unknown): value is ScheduledJobSpec {
  if (!isRecord(value) || !isString(value.kind)) {
    return false;
  }
  if (value.kind === 'index') {
    return hasIndexJobFields(value);
  }
  if (value.kind === 'notify' || value.kind === 'push') {
    return hasDeliveryJobFields(value);
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
 * Parse a sign-in provider summary.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated provider summary.
 */
export function parseAuthProviders(value: unknown): AuthProviders {
  return parseContract(value, 'AuthProvidersResponse', isAuthProviders);
}

/**
 * Parse an OIDC link start response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated authorization redirect target.
 */
export function parseOidcLinkStart(value: unknown): OidcLinkStart {
  return parseContract(value, 'OidcLinkResponse', isOidcLinkStart);
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
 * Parse the administrator-approved AI endpoint catalog.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated exact AI base URLs.
 */
export function parseAiEndpointCatalog(value: unknown): string[] {
  return parseContract(value, 'AI endpoint catalog', isStringArray);
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
    (candidate): candidate is RuntimeSettingInfo[] =>
      isArrayOf(candidate, isRuntimeSettingInfo) &&
      new Set(candidate.map((setting) => setting.field)).size === candidate.length,
  );
}

/**
 * Parse the administrator Provider capability and catalog response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated Provider catalog metadata.
 */
export function parseProviderCatalogResponse(value: unknown): ProviderCatalogResponse {
  return parseContract(value, 'ProviderCatalogResponse', isProviderCatalogResponse);
}

/**
 * Parse canonical index Provider routes stored in a runtime setting value.
 *
 * @param value - Runtime setting JSON text.
 * @returns Validated catalog-to-Provider routes.
 */
export function parseIndexProviderRoutes(value: string): IndexProviderRoutes {
  return parseContract(
    parseContractJson(value, 'index_provider_routes'),
    'index_provider_routes',
    isIndexProviderRoutes,
  );
}

/**
 * Parse a canonical per-Provider proxy policy stored in a runtime setting value.
 *
 * @param value - JSON object text.
 * @returns Validated Provider-to-boolean decisions.
 */
export function parseProviderProxyPolicy(value: string): ProviderProxyPolicy {
  return parseContract(
    parseContractJson(value, 'ProviderProxyPolicy'),
    'ProviderProxyPolicy',
    isProviderProxyPolicy,
  );
}

/**
 * Parse canonical online Provider orders stored in a runtime setting value.
 *
 * @param value - Runtime setting JSON text.
 * @returns Validated default and catalog-specific Provider orders.
 */
export function parseProviderOrderConfiguration(value: string): ProviderOrderConfiguration {
  return parseContract(
    parseContractJson(value, 'ProviderOrderConfiguration'),
    'ProviderOrderConfiguration',
    isProviderOrderConfiguration,
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
