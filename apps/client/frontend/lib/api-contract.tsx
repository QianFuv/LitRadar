/**
 * Generated API aliases and runtime decoders for security-sensitive responses.
 */

import type { components } from '@/lib/generated/api-schema';
import { z } from 'zod';

type ApiSchemas = components['schemas'];

/** Announcement as shown to readers and administrators. */
export type AnnouncementInfo = Omit<ApiSchemas['Announcement'], 'priority'> & {
  priority: NonNullable<ApiSchemas['AnnouncementCreate']['priority']>;
};

export type AuthUser = ApiSchemas['UserResponse'];
export type LoginResponse = ApiSchemas['LoginResponse'];
export type AuthProviders = ApiSchemas['AuthProvidersResponse'];
export type OidcLinkStart = ApiSchemas['OidcLinkResponse'];
export type InviteRequirement = ApiSchemas['InviteRequiredResponse'];
export type InviteCodeStatus = ApiSchemas['InviteResponse']['status'];
export type InviteCode = ApiSchemas['InviteResponse'];
export type AdminInviteCode = ApiSchemas['AdminInviteResponse'];
type GeneratedAdminInviteCodeCreate = ApiSchemas['AdminInviteCodeCreate'];
export type AdminInviteCodeCreate = {
  expires_at?: Exclude<GeneratedAdminInviteCodeCreate['expires_at'], null | undefined>;
  max_uses?: Exclude<GeneratedAdminInviteCodeCreate['max_uses'], null | undefined>;
};
export type TrackingStatus = ApiSchemas['TrackingStatusResponse'];
export type ManualPushStatus = ApiSchemas['ManualPushStatus'];
export type ManualPushState = ApiSchemas['ManualPushStatus']['status'];
export type SchedulerRunState = ApiSchemas['Task']['last_status'];
export type NotificationSettings = ApiSchemas['NotificationSettingsResponse'];
type GeneratedNotificationSettingsUpdate = ApiSchemas['NotificationSettingsUpdate'];
type NotificationSecretField = 'ai_api_key' | 'ai_backup_api_key' | 'pushplus_token';
export type NotificationSettingsUpdate = Required<
  Omit<GeneratedNotificationSettingsUpdate, NotificationSecretField>
> &
  Pick<GeneratedNotificationSettingsUpdate, NotificationSecretField>;
type RuntimeSecretItemInfo = ApiSchemas['SecretItem'];
type RuntimeSecretPoolUpdate = Required<ApiSchemas['RuntimeSecretPoolUpdate']>;
export type RuntimeSettingApplyMode = ApiSchemas['Info']['apply_mode'];
export type RuntimeSettingGroup = ApiSchemas['Info']['group'];
type RuntimeSettingControl = string;
export type RuntimeSettingInfo = Omit<
  ApiSchemas['Info'],
  'control' | 'input_type' | 'secret_items' | 'source' | 'updated_at'
> & {
  control: RuntimeSettingControl;
  input_type: 'text' | 'password' | 'email' | 'boolean' | 'number' | 'url';
  secret_items: RuntimeSecretItemInfo[];
  source: 'database' | 'default';
  updated_at: number | null;
};
export type ProviderCapabilityInfo = ApiSchemas['CapabilityInfo'];
export type ProviderCatalogInfo = Omit<
  ApiSchemas['ProviderCatalog'],
  'csv_filename' | 'database_filename'
> & {
  csv_filename: string | null;
  database_filename: string | null;
};
export type ProviderCatalogResponse = {
  providers: ProviderCapabilityInfo[];
  catalogs: ProviderCatalogInfo[];
};
/** Per-Provider proxy decisions, decoded from the `provider_proxy_policy` setting value. */
export type ProviderProxyPolicy = ApiSchemas['ProviderProxyPolicy'];
/** Default and per-catalog Provider orders, decoded from a `provider_order` setting value. */
export type ProviderOrderConfiguration = ApiSchemas['ProviderOrders'];
/** Catalog-to-Provider routes, decoded from the `index_provider_routes` setting value. */
export type IndexProviderRoutes = ApiSchemas['IndexProviderRoutes'];
export type RuntimeSettingsUpdate = {
  values: NonNullable<ApiSchemas['RuntimeSettingsUpdate']['values']>;
  secret_pool_updates: Record<string, RuntimeSecretPoolUpdate>;
};
export type ScheduledJobSpec = NonNullable<ApiSchemas['Task']['job']>;
export type ScheduledTaskInfo = Omit<
  ApiSchemas['Task'],
  'job' | 'last_run_at' | 'legacy_command'
> & {
  job: ScheduledJobSpec | null;
  last_run_at: number | null;
  legacy_command: string | null;
};
export type ScheduledTaskCreate = Required<ApiSchemas['ScheduledTaskCreate']>;
export type ScheduledTaskUpdate = ApiSchemas['ScheduledTaskUpdate'];
type ScheduledTaskRunInfo = Omit<
  ApiSchemas['Run'],
  'claimed_at' | 'finished_at' | 'started_at' | 'worker_id'
> & {
  claimed_at: number | null;
  finished_at: number | null;
  started_at: number | null;
  worker_id: string | null;
};
type SchedulerWorkerInfo = ApiSchemas['Worker'];
export type SchedulerStatus = Omit<
  ApiSchemas['Status'],
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

/** Safe Provider, catalog, control, and setting identifier grammar. */
const SAFE_RUNTIME_NAME = z
  .string()
  .min(2)
  .max(128)
  .regex(/^[a-z0-9][a-z0-9._-]*$/);

/** Ordered Provider list without repeated names. */
const UNIQUE_RUNTIME_NAMES = z
  .array(SAFE_RUNTIME_NAME)
  .refine((names) => new Set(names).size === names.length);

/**
 * Decode JSON text and validate the decoded value, reporting syntax errors as schema issues.
 *
 * @param schema - Schema for the decoded value.
 * @returns Schema that accepts JSON text and outputs the validated value.
 */
function jsonText<T>(schema: z.ZodType<T>) {
  return z
    .string()
    .transform((text, context): unknown => {
      try {
        return JSON.parse(text);
      } catch {
        context.addIssue({ code: 'custom', message: 'Invalid JSON' });
        return z.NEVER;
      }
    })
    .pipe(schema);
}

/**
 * Parse a payload with a contract schema, raising {@link ApiContractError} on any mismatch.
 *
 * @param schema - Contract schema.
 * @param value - Unknown payload.
 * @param contractName - Human-readable contract name.
 * @returns Validated payload.
 */
function parseContract<T>(schema: z.ZodType<T>, value: unknown, contractName: string): T {
  const result = schema.safeParse(value);
  if (!result.success) {
    throw new ApiContractError(contractName);
  }
  return result.data;
}

const PROVIDER_ORDERS_SCHEMA = z.strictObject({
  default: UNIQUE_RUNTIME_NAMES,
  catalogs: z.record(SAFE_RUNTIME_NAME, UNIQUE_RUNTIME_NAMES),
}) satisfies z.ZodType<ProviderOrderConfiguration>;

const INDEX_PROVIDER_ROUTES_SCHEMA = z
  .record(SAFE_RUNTIME_NAME, SAFE_RUNTIME_NAME)
  .refine((routes) => Object.keys(routes).length > 0) satisfies z.ZodType<IndexProviderRoutes>;

const PROVIDER_PROXY_POLICY_SCHEMA = z.record(
  SAFE_RUNTIME_NAME,
  z.boolean(),
) satisfies z.ZodType<ProviderProxyPolicy>;

const AUTH_USER_SCHEMA = z.looseObject({
  id: z.number(),
  username: z.string(),
  is_admin: z.boolean(),
  email: z.string().nullable(),
  auth_kind: z.enum(['local', 'oidc']),
  is_oidc_linked: z.boolean(),
}) satisfies z.ZodType<AuthUser>;

const AUTH_PROVIDERS_SCHEMA = z.looseObject({
  oidc_enabled: z.boolean(),
  oidc_display_name: z.string(),
}) satisfies z.ZodType<AuthProviders>;

const OIDC_LINK_START_SCHEMA = z.looseObject({
  authorization_url: z.string().min(1),
}) satisfies z.ZodType<OidcLinkStart>;

const LOGIN_RESPONSE_SCHEMA = z.looseObject({
  expires_at: z.number(),
  user: AUTH_USER_SCHEMA,
}) satisfies z.ZodType<LoginResponse>;

const INVITE_REQUIREMENT_SCHEMA = z.looseObject({
  required: z.boolean(),
  bootstrap_required: z.boolean(),
}) satisfies z.ZodType<InviteRequirement>;

const TRACKING_STATUS_SCHEMA = z.looseObject({
  tracking_folder: z.looseObject({ id: z.number(), name: z.string() }).nullable(),
  total_folders: z.number(),
  weekly_articles_available: z.number(),
  notification_configured: z.boolean(),
}) satisfies z.ZodType<TrackingStatus>;

const MANUAL_PUSH_STATUS_SCHEMA = z.looseObject({
  job_id: z.string().nullable(),
  status: z.enum([
    'idle',
    'pending',
    'running',
    'completed',
    'failed',
    'cancelled',
    'timed_out',
    'unknown',
  ]),
  message: z.string(),
  started_at: z.number().nullable(),
  finished_at: z.number().nullable(),
  deadline_at: z.number().nullable(),
  cancellation_requested: z.boolean(),
  can_cancel: z.boolean(),
  can_retry: z.boolean(),
  pushed: z.number(),
  selected: z.number(),
  total_candidates: z.number().nullable(),
  summary: z.string(),
  folder_id: z.number().nullable(),
  folder_name: z.string().nullable(),
}) satisfies z.ZodType<ManualPushStatus>;

const ANNOUNCEMENT_SCHEMA = z.looseObject({
  id: z.number(),
  title: z.string(),
  message: z.string(),
  priority: z.enum(['high', 'normal', 'low']).catch('normal'),
  enabled: z.boolean(),
  created_at: z.number(),
  updated_at: z.number(),
}) satisfies z.ZodType<AnnouncementInfo>;

/** Plaintext secrets that a notification settings response must never carry. */
const NOTIFICATION_SECRET_FIELDS = ['pushplus_token', 'ai_api_key', 'ai_backup_api_key'];

const NOTIFICATION_SETTINGS_SCHEMA = z
  .looseObject({
    id: z.number(),
    user_id: z.number(),
    keywords: z.array(z.string()),
    directions: z.array(z.string()),
    selected_databases: z.array(z.string()),
    delivery_method: z.string(),
    pushplus_token_mask: z.string(),
    pushplus_template: z.string(),
    pushplus_topic: z.string(),
    pushplus_channel: z.string(),
    ai_base_url: z.string(),
    ai_api_key_mask: z.string(),
    ai_model: z.string(),
    ai_system_prompt: z.string(),
    ai_backup_base_url: z.string(),
    ai_backup_api_key_mask: z.string(),
    ai_backup_model: z.string(),
    ai_backup_system_prompt: z.string(),
    has_pushplus_token: z.boolean(),
    has_ai_api_key: z.boolean(),
    has_ai_backup_api_key: z.boolean(),
    sync_to_tracking_folder: z.boolean(),
    ai_retry_attempts: z.number(),
    enabled: z.boolean(),
    created_at: z.number(),
    updated_at: z.number(),
  })
  .refine((settings) =>
    NOTIFICATION_SECRET_FIELDS.every((field) => !Object.hasOwn(settings, field)),
  ) satisfies z.ZodType<NotificationSettings>;

const RUNTIME_SECRET_ITEM_SCHEMA = z.looseObject({
  reference: z.string().min(1),
  masked_value: z.string(),
}) satisfies z.ZodType<RuntimeSecretItemInfo>;

/** Value schemas of the controls whose setting value is structured text. */
const RUNTIME_CONTROL_VALUE_SCHEMAS = new Map<string, z.ZodType>([
  ['boolean', z.enum(['true', 'false'])],
  ['index_provider_routes', jsonText(INDEX_PROVIDER_ROUTES_SCHEMA)],
  ['provider_order', jsonText(PROVIDER_ORDERS_SCHEMA)],
  ['provider_proxy_policy', jsonText(PROVIDER_PROXY_POLICY_SCHEMA)],
]);

/** Controls whose metadata belongs to Provider routing. */
const PROVIDER_ROUTING_CONTROLS = new Set([
  'index_provider_routes',
  'provider_order',
  'provider_proxy_policy',
]);

/** Order-setting fields that must use the Provider order control. */
const PROVIDER_ORDER_FIELDS = new Set([
  'article_abstract_provider_orders',
  'article_fulltext_provider_orders',
]);

/** Mask shown for a stored secret. */
const SECRET_MASK = '••••';

const RUNTIME_SETTING_BASE_SCHEMA = z.looseObject({
  field: SAFE_RUNTIME_NAME,
  label: z.string().min(1),
  description: z.string().min(1),
  group: z.enum([
    'source_access',
    'provider_routing',
    'server_security',
    'observability',
    'single_sign_on',
  ]),
  control: SAFE_RUNTIME_NAME,
  apply_mode: z.enum(['next_request', 'next_command', 'restart_required']),
  allowed_values: z.array(z.string()).refine((values) => new Set(values).size === values.length),
  input_type: z.enum(['text', 'password', 'email', 'boolean', 'number', 'url']),
  is_secret: z.boolean(),
  value: z.string(),
  has_value: z.boolean(),
  masked_value: z.string(),
  secret_items: z.array(RUNTIME_SECRET_ITEM_SCHEMA),
  source: z.enum(['database', 'default']),
  updated_at: z.number().nullable(),
});

type RuntimeSettingCandidate = z.output<typeof RUNTIME_SETTING_BASE_SCHEMA>;

/**
 * Check that a specialized control's value is structurally valid, such as JSON routing text.
 *
 * @param setting - Structurally valid setting descriptor.
 * @returns Whether the value matches its control.
 */
function hasValidRuntimeControlValue(setting: RuntimeSettingCandidate): boolean {
  const valueSchema = RUNTIME_CONTROL_VALUE_SCHEMAS.get(setting.control);
  return valueSchema ? valueSchema.safeParse(setting.value).success : true;
}

/**
 * Check that reserved fields keep their specialized controls and Provider controls stay in the
 * routing group.
 *
 * @param setting - Structurally valid setting descriptor.
 * @returns Whether field, control, and group agree.
 */
function hasRuntimeControlBinding(setting: RuntimeSettingCandidate): boolean {
  const { field, control } = setting;
  return !(
    (field === 'index_provider_routes' && control !== 'index_provider_routes') ||
    (PROVIDER_ORDER_FIELDS.has(field) && control !== 'provider_order') ||
    (field === 'provider_proxy_policy') !== (control === 'provider_proxy_policy') ||
    (PROVIDER_ROUTING_CONTROLS.has(control) && setting.group !== 'provider_routing')
  );
}

/**
 * Check the proxy settings: the proxy URL is a restart-applied password secret and the proxy
 * policy is non-secret restart-applied text without options.
 *
 * @param setting - Structurally valid setting descriptor.
 * @returns Whether the proxy metadata is consistent.
 */
function hasRuntimeProxyMetadata(setting: RuntimeSettingCandidate): boolean {
  const isRestartApplied = setting.apply_mode === 'restart_required';
  if (setting.field === 'provider_proxy_url') {
    return (
      setting.control === 'text' &&
      setting.group === 'source_access' &&
      isRestartApplied &&
      setting.input_type === 'password' &&
      setting.is_secret
    );
  }
  if (setting.control === 'provider_proxy_policy') {
    return (
      isRestartApplied &&
      setting.input_type === 'text' &&
      !setting.is_secret &&
      setting.allowed_values.length === 0
    );
  }
  return true;
}

/**
 * Check boolean input pairing, the ordered boolean options, and that selects offer options.
 *
 * @param setting - Structurally valid setting descriptor.
 * @returns Whether control, input type, and options agree.
 */
function hasRuntimeControlOptions(setting: RuntimeSettingCandidate): boolean {
  const isBooleanControl = setting.control === 'boolean';
  if (isBooleanControl !== (setting.input_type === 'boolean')) {
    return false;
  }
  const [firstValue, secondValue, ...extraValues] = setting.allowed_values;
  if (
    isBooleanControl &&
    (firstValue !== 'true' || secondValue !== 'false' || extraValues.length > 0)
  ) {
    return false;
  }
  return !(setting.control === 'select' && setting.allowed_values.length === 0);
}

/**
 * Check that secrets are redacted: secret pools and secret text expose only masks, and plain
 * settings expose no mask or pool items.
 *
 * @param setting - Structurally valid setting descriptor.
 * @returns Whether the redaction metadata is consistent.
 */
function hasRuntimeRedaction(setting: RuntimeSettingCandidate): boolean {
  const itemCount = setting.secret_items.length;
  const expectedMask = setting.has_value ? SECRET_MASK : '';
  const isRedactedSecret =
    setting.is_secret &&
    setting.input_type === 'password' &&
    setting.allowed_values.length === 0 &&
    setting.value === '' &&
    setting.masked_value === expectedMask;
  if (setting.control === 'secret_pool') {
    return isRedactedSecret && setting.has_value === itemCount > 0;
  }
  if (setting.is_secret) {
    return isRedactedSecret && setting.control === 'text' && itemCount === 0;
  }
  return setting.masked_value === '' && itemCount === 0;
}

const RUNTIME_SETTING_SCHEMA = RUNTIME_SETTING_BASE_SCHEMA.refine(hasValidRuntimeControlValue)
  .refine(hasRuntimeControlBinding)
  .refine(hasRuntimeProxyMetadata)
  .refine(hasRuntimeControlOptions)
  .refine(hasRuntimeRedaction) satisfies z.ZodType<RuntimeSettingInfo>;

const RUNTIME_SETTING_LIST_SCHEMA = z
  .array(RUNTIME_SETTING_SCHEMA)
  .refine(
    (settings) => new Set(settings.map((setting) => setting.field)).size === settings.length,
  ) satisfies z.ZodType<RuntimeSettingInfo[]>;

const PROVIDER_CAPABILITY_SCHEMA = z
  .strictObject({
    name: SAFE_RUNTIME_NAME,
    index_content: z.boolean(),
    article_abstract: z.boolean(),
    article_full_text: z.boolean(),
  })
  .refine(
    (provider) => provider.index_content || provider.article_abstract || provider.article_full_text,
  ) satisfies z.ZodType<ProviderCapabilityInfo>;

const PROVIDER_CATALOG_SCHEMA = z
  .strictObject({
    stem: SAFE_RUNTIME_NAME,
    csv_filename: z.string().nullable(),
    database_filename: z.string().nullable(),
  })
  .refine(
    (catalog) =>
      (catalog.csv_filename !== null || catalog.database_filename !== null) &&
      (catalog.csv_filename === null || catalog.csv_filename === `${catalog.stem}.csv`) &&
      (catalog.database_filename === null ||
        catalog.database_filename === `${catalog.stem}.sqlite`),
  ) satisfies z.ZodType<ProviderCatalogInfo>;

const PROVIDER_CATALOG_RESPONSE_SCHEMA = z
  .strictObject({
    providers: z.array(PROVIDER_CAPABILITY_SCHEMA),
    catalogs: z.array(PROVIDER_CATALOG_SCHEMA),
  })
  .refine(
    (response) =>
      new Set(response.providers.map((provider) => provider.name)).size ===
        response.providers.length &&
      new Set(response.catalogs.map((catalog) => catalog.stem)).size === response.catalogs.length,
  ) satisfies z.ZodType<ProviderCatalogResponse>;

const SCHEDULER_RUN_STATE_SCHEMA = z.enum([
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
]) satisfies z.ZodType<SchedulerRunState>;

/** Optional database and candidate limit shared by notify and push jobs. */
const DELIVERY_JOB_FIELDS = {
  database: z.string().nullable().optional(),
  max_candidates: z.number().nullable().optional(),
};

const SCHEDULED_JOB_SCHEMA = z.discriminatedUnion('kind', [
  z.looseObject({
    kind: z.literal('index'),
    metadata_file: z.string().nullable().optional(),
    notify: z.boolean().optional(),
    push: z.boolean().optional(),
  }),
  z.looseObject({ kind: z.literal('notify'), ...DELIVERY_JOB_FIELDS }),
  z.looseObject({ kind: z.literal('push'), ...DELIVERY_JOB_FIELDS }),
]) satisfies z.ZodType<ScheduledJobSpec>;

const SCHEDULED_TASK_SCHEMA = z
  .looseObject({
    id: z.number(),
    name: z.string(),
    job: SCHEDULED_JOB_SCHEMA.nullable(),
    legacy_command: z.string().nullable(),
    cron: z.string(),
    timezone: z.string(),
    timeout_seconds: z.int().min(1).max(86_400),
    coalesce: z.boolean(),
    enabled: z.boolean(),
    last_run_at: z.number().nullable(),
    last_status: SCHEDULER_RUN_STATE_SCHEMA,
    created_at: z.number(),
    updated_at: z.number(),
  })
  .refine(
    (task) => (task.job === null) !== (task.legacy_command === null),
  ) satisfies z.ZodType<ScheduledTaskInfo>;

const SCHEDULER_STATUS_SCHEMA = z.looseObject({
  last_checked_at: z.number().nullable(),
  workers: z.array(
    z.looseObject({
      worker_id: z.string(),
      started_at: z.number(),
      heartbeat_at: z.number(),
      is_healthy: z.boolean(),
    }),
  ),
  recent_runs: z.array(
    z.looseObject({
      id: z.number(),
      task_id: z.number(),
      task_name: z.string(),
      scheduled_for: z.number(),
      status: SCHEDULER_RUN_STATE_SCHEMA,
      worker_id: z.string().nullable(),
      claimed_at: z.number().nullable(),
      started_at: z.number().nullable(),
      finished_at: z.number().nullable(),
    }),
  ),
}) satisfies z.ZodType<SchedulerStatus>;

/**
 * Parse an authenticated user response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated user response.
 */
export function parseAuthUser(value: unknown): AuthUser {
  return parseContract(AUTH_USER_SCHEMA, value, 'UserResponse');
}

/**
 * Parse a login response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated login response.
 */
export function parseLoginResponse(value: unknown): LoginResponse {
  return parseContract(LOGIN_RESPONSE_SCHEMA, value, 'LoginResponse');
}

/**
 * Parse a sign-in provider summary.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated provider summary.
 */
export function parseAuthProviders(value: unknown): AuthProviders {
  return parseContract(AUTH_PROVIDERS_SCHEMA, value, 'AuthProvidersResponse');
}

/**
 * Parse an OIDC link start response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated authorization redirect target.
 */
export function parseOidcLinkStart(value: unknown): OidcLinkStart {
  return parseContract(OIDC_LINK_START_SCHEMA, value, 'OidcLinkResponse');
}

/**
 * Parse an invite requirement response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated invite requirement.
 */
export function parseInviteRequirement(value: unknown): InviteRequirement {
  return parseContract(INVITE_REQUIREMENT_SCHEMA, value, 'InviteRequiredResponse');
}

/**
 * Parse a tracking status response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated tracking status.
 */
export function parseTrackingStatus(value: unknown): TrackingStatus {
  return parseContract(TRACKING_STATUS_SCHEMA, value, 'TrackingStatusResponse');
}

/**
 * Parse a manual background push status response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated manual push status.
 */
export function parseManualPushStatus(value: unknown): ManualPushStatus {
  return parseContract(MANUAL_PUSH_STATUS_SCHEMA, value, 'ManualWeeklyPushStatus');
}

/**
 * Parse nullable notification settings.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated settings or null when unconfigured.
 */
export function parseNullableNotificationSettings(value: unknown): NotificationSettings | null {
  return parseContract(
    NOTIFICATION_SETTINGS_SCHEMA.nullable(),
    value,
    'NotificationSettingsResponse',
  );
}

/**
 * Parse notification settings.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated notification settings.
 */
export function parseNotificationSettings(value: unknown): NotificationSettings {
  return parseContract(NOTIFICATION_SETTINGS_SCHEMA, value, 'NotificationSettingsResponse');
}

/**
 * Parse the administrator-approved AI endpoint catalog.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated exact AI base URLs.
 */
export function parseAiEndpointCatalog(value: unknown): string[] {
  return parseContract(z.array(z.string()), value, 'AI endpoint catalog');
}

/**
 * Parse announcements, reading an unknown priority as normal.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated announcements.
 */
export function parseAnnouncementList(value: unknown): AnnouncementInfo[] {
  return parseContract(z.array(ANNOUNCEMENT_SCHEMA), value, 'Announcement[]');
}

/**
 * Parse one announcement, reading an unknown priority as normal.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated announcement.
 */
export function parseAnnouncement(value: unknown): AnnouncementInfo {
  return parseContract(ANNOUNCEMENT_SCHEMA, value, 'Announcement');
}

/**
 * Parse a list of runtime setting descriptors.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated runtime setting list.
 */
export function parseRuntimeSettingList(value: unknown): RuntimeSettingInfo[] {
  return parseContract(RUNTIME_SETTING_LIST_SCHEMA, value, 'RuntimeSettingInfo[]');
}

/**
 * Parse the administrator Provider capability and catalog response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated Provider catalog metadata.
 */
export function parseProviderCatalogResponse(value: unknown): ProviderCatalogResponse {
  return parseContract(PROVIDER_CATALOG_RESPONSE_SCHEMA, value, 'ProviderCatalogResponse');
}

/**
 * Parse canonical index Provider routes stored in a runtime setting value.
 *
 * @param value - Runtime setting JSON text.
 * @returns Validated catalog-to-Provider routes.
 */
export function parseIndexProviderRoutes(value: string): IndexProviderRoutes {
  return parseContract(jsonText(INDEX_PROVIDER_ROUTES_SCHEMA), value, 'IndexProviderRoutes');
}

/**
 * Parse a canonical per-Provider proxy policy stored in a runtime setting value.
 *
 * @param value - JSON object text.
 * @returns Validated Provider-to-boolean decisions.
 */
export function parseProviderProxyPolicy(value: string): ProviderProxyPolicy {
  return parseContract(jsonText(PROVIDER_PROXY_POLICY_SCHEMA), value, 'ProviderProxyPolicy');
}

/**
 * Parse canonical online Provider orders stored in a runtime setting value.
 *
 * @param value - Runtime setting JSON text.
 * @returns Validated default and catalog-specific Provider orders.
 */
export function parseProviderOrderConfiguration(value: string): ProviderOrderConfiguration {
  return parseContract(jsonText(PROVIDER_ORDERS_SCHEMA), value, 'ProviderOrders');
}

/**
 * Parse one scheduled task response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated scheduled task.
 */
export function parseScheduledTaskInfo(value: unknown): ScheduledTaskInfo {
  return parseContract(SCHEDULED_TASK_SCHEMA, value, 'ScheduledTaskInfo');
}

/**
 * Parse a list of scheduled task responses.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated scheduled task list.
 */
export function parseScheduledTaskList(value: unknown): ScheduledTaskInfo[] {
  return parseContract(z.array(SCHEDULED_TASK_SCHEMA), value, 'ScheduledTaskInfo[]');
}

/**
 * Parse the durable scheduler status response.
 *
 * @param value - Unknown JSON payload.
 * @returns Validated scheduler status.
 */
export function parseSchedulerStatus(value: unknown): SchedulerStatus {
  return parseContract(SCHEDULER_STATUS_SCHEMA, value, 'SchedulerStatusResponse');
}
