/**
 * URL search-parameter contracts, serialization, and update helpers for the router.
 *
 * Values keep the plain `key=value` query format: list parameters join their items with commas
 * and escape commas inside an item as `%2C`, so existing links and bookmarks stay valid.
 */

import { useNavigate } from '@tanstack/react-router';
import { useCallback } from 'react';
import { z } from 'zod';

import { WORKSPACE_VIEW_IDS } from '@/lib/workspace-view';

/** Route id of the authenticated workspace layout that owns the workspace search state. */
export const WORKSPACE_ROUTE_ID = '/_workspace';

/** Route id of the public login page. */
export const LOGIN_ROUTE_ID = '/login';

/** Favorite kinds shown on the favorites page. */
export const FAVORITE_KINDS = ['articles', 'graphs'] as const;

/** One favorite kind shown on the favorites page. */
export type FavoriteKind = (typeof FAVORITE_KINDS)[number];

const LIST_SEPARATOR = ',';
const ENCODED_LIST_SEPARATOR = '%2C';

/**
 * Join list items into one query value, escaping separators inside items.
 *
 * @param items - List items in display order.
 * @returns Comma-separated query value.
 */
function encodeSearchList(items: readonly string[]): string {
  return items
    .map((item) => item.replaceAll(LIST_SEPARATOR, ENCODED_LIST_SEPARATOR))
    .join(LIST_SEPARATOR);
}

/**
 * Split one comma-separated query value into its items.
 *
 * @param value - Decoded query value.
 * @returns List items, or an empty list for an empty value.
 */
function decodeSearchList(value: string): string[] {
  if (value === '') {
    return [];
  }
  return value
    .split(LIST_SEPARATOR)
    .map((item) => item.replaceAll(ENCODED_LIST_SEPARATOR, LIST_SEPARATOR));
}

/** Optional free-text parameter. */
const SEARCH_TEXT = z.string().optional().catch(undefined);

/** Optional list parameter, read from its comma-separated URL form or set as an array. */
const SEARCH_LIST = z
  .union([z.array(z.string()), z.string().transform(decodeSearchList)])
  .optional()
  .catch(undefined);

/** Optional integer parameter; malformed values are treated as absent. */
const SEARCH_INTEGER = z
  .union([
    z.int(),
    z
      .string()
      .regex(/^-?\d+$/)
      .transform(Number),
  ])
  .optional()
  .catch(undefined);

/**
 * Search state of the authenticated workspace. Invalid values are treated as absent and
 * unknown parameters are kept, so links that carry extra state survive navigation.
 */
export const WORKSPACE_SEARCH_SCHEMA = z.looseObject({
  view: z.enum(WORKSPACE_VIEW_IDS).optional().catch(undefined),
  q: SEARCH_TEXT,
  area: SEARCH_LIST,
  journal_id: SEARCH_LIST,
  month_range: SEARCH_TEXT,
  utd_rating: SEARCH_LIST,
  abs_rating: SEARCH_LIST,
  fms_rating: SEARCH_LIST,
  fmscn_rating: SEARCH_LIST,
  db: SEARCH_TEXT,
  journal: SEARCH_TEXT,
  weekly_q: SEARCH_TEXT,
  cfp_db: SEARCH_TEXT,
  cfp_journal: SEARCH_TEXT,
  folder: SEARCH_INTEGER,
  kind: z.enum(FAVORITE_KINDS).optional().catch(undefined),
  article: SEARCH_TEXT,
  saved: SEARCH_INTEGER,
  settings: SEARCH_TEXT,
  admin: SEARCH_TEXT,
  oidc_link: SEARCH_TEXT,
});

/** Validated search state of the authenticated workspace. */
export type WorkspaceSearch = z.output<typeof WORKSPACE_SEARCH_SCHEMA>;

/** Search state of the public login page. */
export const LOGIN_SEARCH_SCHEMA = z.looseObject({
  next: SEARCH_TEXT,
  logout_recovery: SEARCH_TEXT,
  oidc_error: SEARCH_TEXT,
});

/**
 * Parse a location search string into plain string values, keeping the first of repeated keys.
 *
 * @param searchString - Location search, with or without the leading `?`.
 * @returns Search values keyed by parameter name.
 */
export function parseSearchString(searchString: string): Record<string, string> {
  const search = new Map<string, string>();
  for (const [key, value] of new URLSearchParams(searchString)) {
    if (!search.has(key)) {
      search.set(key, value);
    }
  }
  return Object.fromEntries(search);
}

/**
 * Check whether a search value is a scalar that serializes to one query value.
 *
 * @param value - Search value from a validated or updated search object.
 * @returns Whether the value is a string, number, or boolean.
 */
function isSearchScalar(value: unknown): value is string | number | boolean {
  return typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean';
}

/**
 * Serialize one search value; absent values and empty strings or lists serialize to nothing.
 *
 * @param value - Search value from a validated or updated search object.
 * @returns Query value, or an empty string when the parameter should be omitted.
 */
function serializeSearchValue(value: unknown): string {
  if (Array.isArray(value)) {
    return encodeSearchList(value.filter(isSearchScalar).map(String));
  }
  return isSearchScalar(value) ? String(value) : '';
}

/**
 * Serialize a search object into a location search string in key order.
 *
 * @param search - Search object to serialize.
 * @returns Location search with a leading `?`, or an empty string.
 */
export function stringifySearchObject(search: Record<string, unknown>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(search)) {
    const serialized = serializeSearchValue(value);
    if (serialized) {
      params.set(key, serialized);
    }
  }
  const query = params.toString();
  return query ? `?${query}` : '';
}

/**
 * Return a function that rewrites the workspace search state in place.
 *
 * Updates replace the current history entry and keep the scroll position, so filter and view
 * state changes do not add back-button steps.
 *
 * @returns Updater that receives the current search state and returns the next one.
 */
export function useUpdateWorkspaceSearch() {
  const navigate = useNavigate();
  return useCallback(
    (update: (search: WorkspaceSearch) => WorkspaceSearch) =>
      navigate({ to: '/', search: update, replace: true, resetScroll: false }),
    [navigate],
  );
}
