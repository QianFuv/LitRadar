/**
 * Same-origin API URL resolution coverage.
 */

import { describe, expect, test, vi } from 'vite-plus/test';

import { apiClient, resolveApiBase } from '@/lib/api/client';

describe('API client URL resolution', () => {
  test('sends requests to the current browser origin', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValue(Response.json(['journals.sqlite']));

    expect(resolveApiBase()).toBe(window.location.origin);
    await apiClient.GET('/api/meta/databases');

    expect(fetchSpy.mock.calls[0][0]).toBe('http://localhost/api/meta/databases');
  });
});
