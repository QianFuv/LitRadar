/**
 * Shared Node request interception server for Vitest.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';
import { setupServer } from 'msw/node';

/**
 * Baseline handlers that survive `resetHandlers`.
 *
 * Article details look up optional journal metadata; tests that do not assert it get an
 * explicit "not found", which the UI treats as missing metadata.
 */
const BASELINE_HANDLERS: RequestHandler[] = [
  http.get('http://localhost/api/journals/:journalId', () =>
    HttpResponse.json({ detail: 'Journal not found' }, { status: 404 }),
  ),
];

export const server = setupServer(...BASELINE_HANDLERS);

/**
 * Install explicit scenario handlers for the current test only.
 *
 * @param handlers - Request handlers owned by the test's selected domains.
 */
export function installScenarioHandlers(...handlers: RequestHandler[]): void {
  server.use(...handlers);
}
