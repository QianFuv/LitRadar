/**
 * Compatibility coverage for the domain-oriented API facade.
 */

import { describe, expect, test } from 'vitest';

import * as facade from '@/lib/api';
import { adminGetStats } from '@/lib/api/admin';
import { getCurrentUser } from '@/lib/api/auth';
import { DEFAULT_DATABASE } from '@/lib/api/client';
import { getFolders } from '@/lib/api/favorites';
import { getArticles } from '@/lib/api/index';
import { getTrackingStatus } from '@/lib/api/tracking';

/**
 * Verify legacy consumers receive the exact domain function references.
 */
function preservesFacadeExports(): void {
  expect(facade.adminGetStats).toBe(adminGetStats);
  expect(facade.getCurrentUser).toBe(getCurrentUser);
  expect(facade.getFolders).toBe(getFolders);
  expect(facade.getArticles).toBe(getArticles);
  expect(facade.getTrackingStatus).toBe(getTrackingStatus);
  expect(facade.DEFAULT_DATABASE).toBe(DEFAULT_DATABASE);
}

describe('API compatibility facade', () => {
  test('re-exports each domain client without wrappers', preservesFacadeExports);
});
