/**
 * Runtime contract validation tests for generated control-plane aliases.
 */

import { describe, expect, test } from 'vitest';

import {
  ApiContractError,
  parseLoginResponse,
  parseManualPushStatus,
  parseRuntimeSettingList,
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
}

describe('generated API runtime contracts', () => {
  test('accepts a valid login response', acceptsValidLoginContract);
  test('rejects malformed control-plane responses', rejectsMalformedControlPlaneContracts);
});
