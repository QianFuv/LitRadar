/**
 * Explicit administrator scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import { createErrorScenario, type ErrorScenario } from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

/**
 * Create an administrator error override backed by the shared error envelope.
 *
 * @param overrides - Optional error response overrides.
 * @returns Administrator error request handlers.
 */
export function createAdminErrorScenarioHandlers(
  overrides: Partial<ErrorScenario> = {},
): RequestHandler[] {
  return [
    http.get(`${API_URL}/admin/users`, () =>
      HttpResponse.json(createErrorScenario(overrides), { status: 401 }),
    ),
  ];
}
