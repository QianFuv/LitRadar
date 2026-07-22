/**
 * Same-origin API URL resolution coverage.
 */

import { afterEach, describe, expect, test, vi } from 'vitest';

import { buildApiUrl, resolveApiBase } from '@/lib/api/client';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('API client URL resolution', () => {
  test('uses the current browser origin', () => {
    expect(resolveApiBase()).toBe(window.location.origin);
    expect(buildApiUrl('/api/articles')).toBe('http://localhost/api/articles');
  });

  test('uses a deterministic non-browser fallback', () => {
    vi.stubGlobal('window', undefined);

    expect(resolveApiBase()).toBe('http://localhost');
    expect(buildApiUrl('/api/health')).toBe('http://localhost/api/health');
  });
});
