/**
 * Shared Node request interception server for Vitest.
 */

import type { RequestHandler } from 'msw';
import { setupServer } from 'msw/node';

export const server = setupServer();

/**
 * Install explicit scenario handlers for the current test only.
 *
 * @param handlers - Request handlers owned by the test's selected domains.
 */
export function installScenarioHandlers(...handlers: RequestHandler[]): void {
  server.use(...handlers);
}
