/**
 * Global DOM assertions, cleanup, storage isolation, and MSW lifecycle hooks.
 */

import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterAll, afterEach, beforeAll } from 'vitest';

import { server } from '@/tests/mocks/server';

process.env.NEXT_PUBLIC_API_URL = 'http://localhost';

/**
 * Start request interception and fail tests on unhandled network calls.
 */
function startMockServer(): void {
  server.listen({ onUnhandledRequest: 'error' });
}

/**
 * Reset DOM, handlers, and browser storage after each test.
 */
function resetTestState(): void {
  cleanup();
  server.resetHandlers();
  window.localStorage.clear();
  window.sessionStorage.clear();
}

/**
 * Close request interception after the test process finishes.
 */
function closeMockServer(): void {
  server.close();
}

beforeAll(startMockServer);
afterEach(resetTestState);
afterAll(closeMockServer);
