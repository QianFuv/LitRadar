/**
 * Explicit authentication scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import {
  createAuthProvidersScenario,
  createAuthUserScenario,
  createLoginScenario,
  type AuthProvidersScenario,
  type AuthUserScenario,
  type LoginScenario,
} from '@/tests/mocks/scenarios';

const API_URL = 'http://localhost/api';

/** Authorization URL returned by the mocked OIDC link start endpoint. */
export const OIDC_AUTHORIZATION_URL = 'https://auth.example.test/application/o/authorize/?state=s';

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

/**
 * Create stateful account handlers for email and single sign-on management.
 *
 * Email updates and link removal mutate one in-memory user so later responses reflect them;
 * an email of `taken@example.test` returns the backend duplicate-email conflict.
 *
 * @param userOverrides - Initial authenticated user fields.
 * @param providerOverrides - Sign-in provider summary fields.
 * @returns Account request handlers.
 */
export function createAccountScenarioHandlers(
  userOverrides: Partial<AuthUserScenario> = {},
  providerOverrides: Partial<AuthProvidersScenario> = {},
): RequestHandler[] {
  let currentUser = createAuthUserScenario(userOverrides);
  return [
    http.get(`${API_URL}/auth/me`, () => HttpResponse.json(currentUser)),
    http.get(`${API_URL}/auth/providers`, () =>
      HttpResponse.json(createAuthProvidersScenario(providerOverrides)),
    ),
    http.put(`${API_URL}/auth/email`, async ({ request }) => {
      const { email } = (await request.json()) as { email: string | null };
      if (email === 'taken@example.test') {
        return HttpResponse.json({ detail: 'Email already exists' }, { status: 409 });
      }
      currentUser = { ...currentUser, email: email?.toLowerCase() ?? null };
      return HttpResponse.json(currentUser);
    }),
    http.post(`${API_URL}/auth/oidc/link`, () =>
      HttpResponse.json({ authorization_url: OIDC_AUTHORIZATION_URL }),
    ),
    http.delete(`${API_URL}/auth/oidc/link`, () => {
      currentUser = { ...currentUser, is_oidc_linked: false };
      return HttpResponse.json(currentUser);
    }),
  ];
}

/** Stable authentication happy-path handlers. */
export const AUTH_SCENARIO_HANDLERS = createAuthScenarioHandlers();
