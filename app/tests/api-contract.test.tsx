/**
 * Runtime contract validation tests for generated control-plane aliases.
 */

import { describe, expect, test } from 'vitest';

import {
  ApiContractError,
  parseLoginResponse,
  parseManualPushStatus,
  parseNotificationSettings,
  parseProviderCatalogResponse,
  parseRuntimeSettingList,
  parseSchedulerStatus,
} from '@/lib/api-contract';
import {
  createAdminErrorScenarioHandlers,
  createAuthScenarioHandlers,
  createDiscoveryScenarioHandlers,
  createFavoriteScenarioHandlers,
  createTrackingScenarioHandlers,
} from '@/tests/mocks/handlers';
import {
  createArticlePageScenario,
  createErrorScenario,
  createLoginScenario,
  createMaskedNotificationSettingsScenario,
  createWeeklyUpdatesScenario,
} from '@/tests/mocks/scenarios';
import { installScenarioHandlers } from '@/tests/mocks/server';

/**
 * Verify valid generated auth payloads are accepted without coercion.
 */
function acceptsValidLoginContract(): void {
  const payload = createLoginScenario();

  expect(parseLoginResponse(payload)).toBe(payload);
}

/**
 * Verify malformed secret-setting and background payloads fail closed.
 */
function rejectsMalformedControlPlaneContracts(): void {
  expect(() =>
    parseRuntimeSettingList([
      {
        field: 'ai_api_key',
        label: 'AI key',
        description: 'secret',
        input_type: 'password',
        value: 'masked',
        source: 'database',
        updated_at: null,
      },
    ]),
  ).toThrow(ApiContractError);
  expect(() =>
    parseProviderCatalogResponse({
      providers: [
        {
          name: 'scholarly',
          index_content: true,
          article_abstract: true,
          article_full_text: false,
        },
      ],
      catalogs: [
        {
          stem: 'english_journals',
          csv_filename: '../english_journals.csv',
          database_filename: null,
        },
      ],
    }),
  ).toThrow(ApiContractError);
  expect(() =>
    parseManualPushStatus({
      job_id: 'job-1',
      status: 'unknown',
      message: 'bad status',
      started_at: 1,
      finished_at: null,
      pushed: 0,
      selected: 0,
      total_candidates: null,
      summary: '',
      folder_id: null,
      folder_name: null,
    }),
  ).toThrow(ApiContractError);
  expect(() =>
    parseSchedulerStatus({
      last_checked_at: 42,
      workers: [],
      recent_runs: [
        {
          id: 1,
          task_id: 2,
          task_name: 'daily',
          scheduled_for: 40,
          status: 'impossible',
          worker_id: null,
          claimed_at: null,
          started_at: null,
          finished_at: null,
        },
      ],
    }),
  ).toThrow(ApiContractError);
}

/**
 * Verify strong runtime metadata and safe Provider catalogs are accepted.
 */
function acceptsRuntimeMetadataAndProviderCatalog(): void {
  const booleanSetting = {
    field: 'secure_cookies',
    label: 'Secure cookies',
    description: 'Use secure session cookies.',
    group: 'server_security',
    control: 'boolean',
    apply_mode: 'restart_required',
    allowed_values: ['true', 'false'],
    input_type: 'boolean',
    is_secret: false,
    value: 'false',
    has_value: true,
    masked_value: '',
    secret_items: [],
    source: 'default',
    updated_at: null,
  };
  const futureControl = {
    ...booleanSetting,
    field: 'future_setting',
    label: 'Future setting',
    description: 'Backend-declared future setting.',
    group: 'observability',
    control: 'future_text',
    apply_mode: 'next_command',
    allowed_values: [],
    input_type: 'text',
    value: 'value',
  };
  const runtimeSettings = [booleanSetting, futureControl];
  const providerCatalog = {
    providers: [
      {
        name: 'scholarly',
        index_content: true,
        article_abstract: true,
        article_full_text: false,
      },
    ],
    catalogs: [
      {
        stem: 'english_journals',
        csv_filename: 'english_journals.csv',
        database_filename: null,
      },
    ],
  };

  expect(parseRuntimeSettingList(runtimeSettings)).toBe(runtimeSettings);
  expect(parseProviderCatalogResponse(providerCatalog)).toBe(providerCatalog);
  expect(() => parseRuntimeSettingList([booleanSetting, booleanSetting])).toThrow(ApiContractError);
  expect(() => parseRuntimeSettingList([{ ...booleanSetting, group: 'unknown_group' }])).toThrow(
    ApiContractError,
  );
  expect(() =>
    parseRuntimeSettingList([
      {
        ...futureControl,
        field: 'article_abstract_provider_orders',
        control: 'provider_order',
        value: '{"default":["scholarly","scholarly"],"catalogs":{}}',
      },
    ]),
  ).toThrow(ApiContractError);
}

/**
 * Verify durable scheduler health and run metadata are accepted.
 */
function acceptsSchedulerStatusContract(): void {
  const payload = {
    last_checked_at: 42,
    workers: [
      {
        worker_id: 'worker-1',
        started_at: 1,
        heartbeat_at: 42,
        is_healthy: true,
      },
    ],
    recent_runs: [
      {
        id: 1,
        task_id: 2,
        task_name: 'daily',
        scheduled_for: 40,
        status: 'success',
        worker_id: 'worker-1',
        claimed_at: 40,
        started_at: 40,
        finished_at: 41,
      },
    ],
  };

  expect(parseSchedulerStatus(payload)).toBe(payload);
}

/**
 * Verify notification responses expose only fixed masks and configured flags.
 */
function acceptsMaskedNotificationContract(): void {
  const payload = createMaskedNotificationSettingsScenario();

  expect(parseNotificationSettings(payload)).toBe(payload);
  expect(() =>
    parseNotificationSettings({ ...payload, pushplus_token: 'plaintext-secret' }),
  ).toThrow(ApiContractError);
  expect(JSON.stringify(parseNotificationSettings(payload))).not.toContain('plaintext-secret');
}

/**
 * Verify domain bundles serve all shared scenarios only after explicit installation.
 */
async function servesExplicitSharedScenarioHandlers(): Promise<void> {
  installScenarioHandlers(
    ...createAuthScenarioHandlers(),
    ...createDiscoveryScenarioHandlers(),
    ...createFavoriteScenarioHandlers(),
    ...createTrackingScenarioHandlers(),
    ...createAdminErrorScenarioHandlers(),
  );

  const loginResponse = await fetch('http://localhost/api/auth/login', { method: 'POST' });
  const articlesResponse = await fetch('http://localhost/api/articles');
  const weeklyResponse = await fetch('http://localhost/api/weekly-updates');
  const favoritesResponse = await fetch('http://localhost/api/favorites/folders');
  const notificationResponse = await fetch('http://localhost/api/tracking/notification-settings');
  const errorResponse = await fetch('http://localhost/api/admin/users');

  expect(await loginResponse.json()).toEqual(createLoginScenario());
  expect(await articlesResponse.json()).toEqual(createArticlePageScenario());
  expect(await weeklyResponse.json()).toEqual(createWeeklyUpdatesScenario());
  expect(await favoritesResponse.json()).toEqual([]);
  expect(await notificationResponse.json()).toEqual(createMaskedNotificationSettingsScenario());
  expect(errorResponse.status).toBe(401);
  expect(await errorResponse.json()).toEqual(createErrorScenario());
}

describe('generated API runtime contracts', () => {
  test('accepts a valid login response', acceptsValidLoginContract);
  test('accepts durable scheduler status metadata', acceptsSchedulerStatusContract);
  test(
    'accepts strong runtime metadata and safe Provider catalogs',
    acceptsRuntimeMetadataAndProviderCatalog,
  );
  test('accepts only the masked notification response contract', acceptsMaskedNotificationContract);
  test('rejects malformed control-plane responses', rejectsMalformedControlPlaneContracts);
  test(
    'serves explicitly installed shared scenario handlers',
    servesExplicitSharedScenarioHandlers,
  );
});
