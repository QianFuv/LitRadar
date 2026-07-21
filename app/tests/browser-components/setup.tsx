/**
 * Browser Mode assertions and per-test DOM isolation.
 */

import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

/**
 * Remove mounted React trees and reset browser-owned state after each component test.
 */
function resetBrowserComponentState(): void {
  cleanup();
  window.localStorage.clear();
  window.sessionStorage.clear();
  window.scrollTo({ behavior: 'auto', top: 0 });
}

afterEach(resetBrowserComponentState);
