'use client';

/** Shared journal-rating URL state and article request serialization. */

import { parseAsArrayOf, parseAsString, useQueryStates } from 'nuqs';

/** Fixed rating systems exposed by journal metadata. */
export const JOURNAL_RATING_SYSTEMS = [
  { key: 'utd_rating', label: 'UTD' },
  { key: 'abs_rating', label: 'ABS' },
  { key: 'fms_rating', label: 'FMS' },
  { key: 'fmscn_rating', label: 'FMS 中国' },
] as const;

/** One supported rating parameter name. */
export type JournalRatingKey = (typeof JOURNAL_RATING_SYSTEMS)[number]['key'];

const JOURNAL_RATING_PARSERS = {
  utd_rating: parseAsArrayOf(parseAsString).withDefault([]),
  abs_rating: parseAsArrayOf(parseAsString).withDefault([]),
  fms_rating: parseAsArrayOf(parseAsString).withDefault([]),
  fmscn_rating: parseAsArrayOf(parseAsString).withDefault([]),
};

/**
 * Read and update the four rating selections together in the current URL.
 *
 * @returns Selections and an updater that also supports clearing all four groups.
 */
export function useJournalRatingFilters() {
  return useQueryStates(JOURNAL_RATING_PARSERS);
}

/**
 * Append exact repeated rating parameters while retaining all other query filters.
 *
 * @param params - Article query parameters to update.
 * @param ratings - Selected grades for each rating system.
 */
export function appendJournalRatingParams(
  params: URLSearchParams,
  ratings: Record<JournalRatingKey, string[]>,
): void {
  for (const { key } of JOURNAL_RATING_SYSTEMS) {
    for (const value of ratings[key]) params.append(key, value);
  }
}
