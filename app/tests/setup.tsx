/**
 * Global DOM assertions, cleanup, storage isolation, and MSW lifecycle hooks.
 */

import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterAll, afterEach, beforeAll } from 'vitest';

import { server } from '@/tests/mocks/server';

/**
 * Create a static media-query result with reduced motion enabled for unit tests.
 *
 * @param query - Browser media query string.
 * @returns Standards-shaped immutable media-query result.
 */
function createTestMediaQueryList(query: string): MediaQueryList {
  return {
    matches: query === '(prefers-reduced-motion: reduce)',
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    addListener: () => undefined,
    dispatchEvent: () => false,
    removeEventListener: () => undefined,
    removeListener: () => undefined,
  };
}

/**
 * Install deterministic reduced-motion media-query behavior for jsdom.
 */
function installTestMediaQueries(): void {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: createTestMediaQueryList,
  });
}

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

beforeAll(installTestMediaQueries);
beforeAll(startMockServer);
afterEach(resetTestState);
afterAll(closeMockServer);
