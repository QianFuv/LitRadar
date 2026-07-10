/**
 * Runtime contract validation tests for generated control-plane aliases.
 */

import { describe, expect, test } from 'vitest';

import {
  ApiContractError,
  parseLoginResponse,
  parseManualPushStatus,
  parseNotificationSettings,
  parseRuntimeSettingList,
  parseSchedulerStatus,
} from '@/lib/api-contract';

/**
 * Verify valid generated auth payloads are accepted without coercion.
 */
function acceptsValidLoginContract(): void {
  const payload = {
    expires_at: 42,
    user: { id: 7, username: 'contract_user', is_admin: true },
  };

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
  const payload = {
    id: 1,
    user_id: 7,
    keywords: [],
    directions: [],
    selected_databases: [],
    delivery_method: 'pushplus',
    has_pushplus_token: true,
    pushplus_token_mask: '••••',
    pushplus_template: 'markdown',
    pushplus_topic: '',
    pushplus_channel: 'wechat',
    sync_to_tracking_folder: false,
    ai_base_url: 'https://ai.example/v1',
    has_ai_api_key: true,
    ai_api_key_mask: '••••',
    ai_model: 'fixture-model',
    ai_system_prompt: '',
    ai_backup_base_url: '',
    has_ai_backup_api_key: false,
    ai_backup_api_key_mask: '',
    ai_backup_model: '',
    ai_backup_system_prompt: '',
    ai_retry_attempts: 3,
    enabled: true,
    created_at: 1,
    updated_at: 2,
  };

  expect(parseNotificationSettings(payload)).toBe(payload);
  expect(() =>
    parseNotificationSettings({ ...payload, pushplus_token: 'plaintext-secret' }),
  ).toThrow(ApiContractError);
  expect(JSON.stringify(parseNotificationSettings(payload))).not.toContain('plaintext-secret');
}

describe('generated API runtime contracts', () => {
  test('accepts a valid login response', acceptsValidLoginContract);
  test('accepts durable scheduler status metadata', acceptsSchedulerStatusContract);
  test('accepts only the masked notification response contract', acceptsMaskedNotificationContract);
  test('rejects malformed control-plane responses', rejectsMalformedControlPlaneContracts);
});
