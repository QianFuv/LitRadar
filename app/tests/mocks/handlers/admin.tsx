/**
 * Explicit administrator scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import type { components } from '@/lib/generated/api-schema';
import { createErrorScenario, type ErrorScenario } from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

type AdminUserScenario = components['schemas']['AdminUserInfo'];

/**
 * Create administrator happy-path handlers with a typed user list.
 *
 * @param users - Administrator user responses returned by the list endpoint.
 * @returns Administrator request handlers.
 */
export function createAdminScenarioHandlers(users: AdminUserScenario[] = []): RequestHandler[] {
  return [http.get(`${API_URL}/admin/users`, () => HttpResponse.json(users))];
}

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

/** Stable administrator happy-path handlers. */
export const ADMIN_SCENARIO_HANDLERS = createAdminScenarioHandlers();
