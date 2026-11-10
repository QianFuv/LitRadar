/**
 * URL search contracts: query-string round trips, list encoding, and schema validation.
 */

import { describe, expect, test } from 'vite-plus/test';

import {
  LOGIN_SEARCH_SCHEMA,
  parseSearchString,
  stringifySearchObject,
  WORKSPACE_SEARCH_SCHEMA,
} from '@/lib/search-params';

/** Verify parsing keeps parameter order and the first value of a repeated parameter. */
function keepsOrderAndFirstRepeatedValue(): void {
  const search = parseSearchString('?q=graph&area=Medicine&admin=overview&q=ignored');

  expect(Object.keys(search)).toEqual(['q', 'area', 'admin']);
  expect(search).toEqual({ q: 'graph', area: 'Medicine', admin: 'overview' });
}

/** Verify serialization omits absent and empty values and keeps key order. */
function omitsEmptyValues(): void {
  expect(
    stringifySearchObject({
      view: 'favorites',
      q: '',
      area: [],
      folder: 4,
      admin: undefined,
      settings: null,
    }),
  ).toBe('?view=favorites&folder=4');
  expect(stringifySearchObject({})).toBe('');
}

/** Verify list items keep commas through a full URL round trip, as the previous format did. */
function roundTripsListsWithCommas(): void {
  const areas = ['Regional, Environmental & Resource Studies', 'Finance'];
  const searchString = stringifySearchObject({ area: areas, q: 'graph neural' });

  expect(searchString).toBe(
    '?area=Regional%252C+Environmental+%26+Resource+Studies%2CFinance&q=graph+neural',
  );
  expect(WORKSPACE_SEARCH_SCHEMA.parse(parseSearchString(searchString))).toEqual({
    area: areas,
    q: 'graph neural',
  });
}

/** Verify existing comma-separated links still decode into list selections. */
function readsExistingCommaSeparatedLinks(): void {
  const search = WORKSPACE_SEARCH_SCHEMA.parse(
    parseSearchString('?abs_rating=4,4*&journal_id=101&area=Regional%252C+Studies,Finance'),
  );

  expect(search.abs_rating).toEqual(['4', '4*']);
  expect(search.journal_id).toEqual(['101']);
  expect(search.area).toEqual(['Regional, Studies', 'Finance']);
}

/** Verify malformed values are treated as absent and unknown parameters survive. */
function toleratesInvalidValues(): void {
  const search = WORKSPACE_SEARCH_SCHEMA.parse(
    parseSearchString('?view=unsupported&kind=graphs&folder=4x&saved=7&settings=unknown&extra=1'),
  );

  expect(search.view).toBeUndefined();
  expect(search.kind).toBe('graphs');
  expect(search.folder).toBeUndefined();
  expect(search.saved).toBe(7);
  expect(search.settings).toBe('unknown');
  expect(search.extra).toBe('1');
  expect(WORKSPACE_SEARCH_SCHEMA.parse({ folder: 3, area: ['a'] })).toEqual({
    folder: 3,
    area: ['a'],
  });
}

/** Verify the login page reads its return path and recovery flags as plain strings. */
function parsesLoginSearch(): void {
  expect(
    LOGIN_SEARCH_SCHEMA.parse(
      parseSearchString('?next=%2F%3Fview%3Dfavorites&logout_recovery=1&oidc_error=state'),
    ),
  ).toEqual({ next: '/?view=favorites', logout_recovery: '1', oidc_error: 'state' });
}

describe('search params', () => {
  test('keeps parameter order and the first repeated value', keepsOrderAndFirstRepeatedValue);
  test('omits absent and empty values', omitsEmptyValues);
  test('round-trips list items that contain commas', roundTripsListsWithCommas);
  test('reads existing comma-separated links', readsExistingCommaSeparatedLinks);
  test('treats invalid values as absent and keeps unknown parameters', toleratesInvalidValues);
  test('parses login search parameters', parsesLoginSearch);
});
