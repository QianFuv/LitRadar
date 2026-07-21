/**
 * Explicit authentication scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import { createLoginScenario, type LoginScenario } from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

/**
 * Create authentication handlers backed by typed shared scenarios.
 *
 * @param loginOverrides - Optional login response overrides.
 * @returns Authentication request handlers.
 */
export function createAuthScenarioHandlers(
  loginOverrides: Partial<LoginScenario> = {},
): RequestHandler[] {
  return [
    http.post(`${API_URL}/auth/login`, () =>
      HttpResponse.json(createLoginScenario(loginOverrides)),
    ),
  ];
}

/** Stable authentication happy-path handlers. */
export const AUTH_SCENARIO_HANDLERS = createAuthScenarioHandlers();
