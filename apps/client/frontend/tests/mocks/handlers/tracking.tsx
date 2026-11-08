/**
 * Explicit tracking scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import type { ManualPushStatus } from '@/lib/api';
import {
  createMaskedNotificationSettingsScenario,
  type MaskedNotificationSettingsScenario,
} from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

/**
 * Return the stable idle manual-push status used outside polling-specific tests.
 *
 * @returns Idle manual-push response.
 */
export function idleManualPushStatusResponse(): Response {
  return HttpResponse.json({
    job_id: null,
    status: 'idle',
    message: '尚未运行',
    started_at: null,
    finished_at: null,
    deadline_at: null,
    cancellation_requested: false,
    can_cancel: false,
    can_retry: false,
    pushed: 0,
    selected: 0,
    total_candidates: 0,
    summary: '',
    folder_id: null,
    folder_name: null,
  } satisfies ManualPushStatus);
}

/**
 * Create tracking handlers backed by typed shared scenarios.
 *
 * @param settingsOverrides - Optional notification response overrides.
 * @returns Tracking request handlers.
 */
export function createTrackingScenarioHandlers(
  settingsOverrides: Partial<MaskedNotificationSettingsScenario> = {},
): RequestHandler[] {
  return [
    http.get(`${API_URL}/tracking/notification-settings`, () =>
      HttpResponse.json(createMaskedNotificationSettingsScenario(settingsOverrides)),
    ),
    http.get(`${API_URL}/tracking/ai-endpoints`, () =>
      HttpResponse.json(['https://ai.invalid/v1/', 'https://backup.invalid/v1/']),
    ),
  ];
}

/** Stable tracking happy-path handlers. */
export const TRACKING_SCENARIO_HANDLERS = createTrackingScenarioHandlers();
