/** Validate enum fields without coercing malformed JSON response values. */

import { describe, expect, test } from 'vitest';

import {
  ApiContractError,
  parseManualPushStatus,
  parseRuntimeSettingList,
  parseSchedulerStatus,
} from '@/lib/api-contract';

const RUNTIME_SETTING = {
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

describe('response enum types', () => {
  test.each(['group', 'apply_mode', 'input_type', 'source'] as const)(
    'rejects an array in runtime setting %s',
    (field) => {
      expect(parseRuntimeSettingList([RUNTIME_SETTING])).toEqual([RUNTIME_SETTING]);
      const malformed = { ...RUNTIME_SETTING, [field]: [RUNTIME_SETTING[field]] };
      expect(() => parseRuntimeSettingList([malformed])).toThrow(ApiContractError);
    },
  );

  test('rejects an array in manual push status', () => {
    const payload = {
      job_id: 'job-1',
      status: 'running',
      message: 'Running',
      started_at: 1,
      finished_at: null,
      deadline_at: 3,
      cancellation_requested: false,
      can_cancel: true,
      can_retry: false,
      pushed: 0,
      selected: 0,
      total_candidates: null,
      summary: '',
      folder_id: null,
      folder_name: null,
    };
    expect(parseManualPushStatus(payload)).toBe(payload);
    expect(() => parseManualPushStatus({ ...payload, status: ['running'] })).toThrow(
      ApiContractError,
    );
  });

  test('rejects an array in scheduler run status', () => {
    const run = {
      id: 1,
      task_id: 2,
      task_name: 'daily',
      scheduled_for: 40,
      status: 'running',
      worker_id: 'worker-1',
      claimed_at: 40,
      started_at: 40,
      finished_at: null,
    };
    const payload = { last_checked_at: 42, workers: [], recent_runs: [run] };
    expect(parseSchedulerStatus(payload)).toBe(payload);
    expect(() =>
      parseSchedulerStatus({ ...payload, recent_runs: [{ ...run, status: ['running'] }] }),
    ).toThrow(ApiContractError);
  });
});
