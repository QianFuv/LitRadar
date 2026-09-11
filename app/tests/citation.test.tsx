/**
 * Safe DOI and external-link coverage.
 */

import { describe, expect, test } from 'vitest';

import { getDoiUrl, getSafeHttpUrl } from '@/lib/citation';

/**
 * Verify DOI parsing and the underlying HTTP(S) URL validator.
 */
function validatesExternalDestinations(): void {
  expect(getSafeHttpUrl('https://example.com/article')).toBe('https://example.com/article');
  expect(getSafeHttpUrl('http://example.com/article')).toBe('http://example.com/article');
  expect(getSafeHttpUrl('javascript:alert(1)')).toBeNull();
  expect(getSafeHttpUrl('data:text/html,unsafe')).toBeNull();
  expect(getSafeHttpUrl('/relative/article')).toBeNull();

  expect(getDoiUrl('10.1000/example')).toBe('https://doi.org/10.1000/example');
  expect(getDoiUrl('doi:10.1000/example')).toBe('https://doi.org/10.1000/example');
  expect(getDoiUrl('https://doi.org/10.1000/example')).toBe('https://doi.org/10.1000/example');
  expect(getDoiUrl('javascript:alert(1)')).toBeNull();
}

describe('article external links', () => {
  test('validates DOI and HTTP destinations', validatesExternalDestinations);
});
