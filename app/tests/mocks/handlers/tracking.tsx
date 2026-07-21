/**
 * Explicit tracking scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import {
  createMaskedNotificationSettingsScenario,
  type MaskedNotificationSettingsScenario,
} from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

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
  ];
}

/** Stable tracking happy-path handlers. */
export const TRACKING_SCENARIO_HANDLERS = createTrackingScenarioHandlers();
