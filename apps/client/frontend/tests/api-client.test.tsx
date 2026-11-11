/**
 * Same-origin API URL resolution coverage.
 */

import { describe, expect, test } from 'vite-plus/test';

import { buildApiUrl, resolveApiBase } from '@/lib/api/client';

describe('API client URL resolution', () => {
  test('uses the current browser origin', () => {
    expect(resolveApiBase()).toBe(window.location.origin);
    expect(buildApiUrl('/api/articles')).toBe('http://localhost/api/articles');
  });
});
