/**
 * Runtime contract validation tests for generated control-plane aliases.
 */

import { describe, expect, test } from 'vitest';

import {
  ApiContractError,
  parseLoginResponse,
  parseManualPushStatus,
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

describe('generated API runtime contracts', () => {
  test('accepts a valid login response', acceptsValidLoginContract);
  test('accepts durable scheduler status metadata', acceptsSchedulerStatusContract);
  test('rejects malformed control-plane responses', rejectsMalformedControlPlaneContracts);
});
