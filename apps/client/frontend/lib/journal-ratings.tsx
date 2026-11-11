/** Shared journal-rating URL state and article request serialization. */

import type { WorkspaceSearch } from '@/lib/search-params';

/** Fixed rating systems exposed by journal metadata. */
export const JOURNAL_RATING_SYSTEMS = [
  { key: 'utd_rating', label: 'UTD' },
  { key: 'abs_rating', label: 'ABS' },
  { key: 'fms_rating', label: 'FMS' },
  { key: 'fmscn_rating', label: 'FMS 中国' },
] as const;

/** One supported rating parameter name. */
export type JournalRatingKey = (typeof JOURNAL_RATING_SYSTEMS)[number]['key'];

/**
 * Read the four rating selections from the workspace search state.
 *
 * @param search - Current workspace search state.
 * @returns Selected grades for each rating system.
 */
export function readJournalRatings(search: WorkspaceSearch): Record<JournalRatingKey, string[]> {
  return {
    utd_rating: search.utd_rating ?? [],
    abs_rating: search.abs_rating ?? [],
    fms_rating: search.fms_rating ?? [],
    fmscn_rating: search.fmscn_rating ?? [],
  };
}

/**
 * Return workspace search state with all four rating selections replaced or cleared.
 *
 * @param search - Current workspace search state.
 * @param ratings - Next selections, or null to clear every rating group.
 * @returns Next workspace search state.
 */
export function withJournalRatings(
  search: WorkspaceSearch,
  ratings: Record<JournalRatingKey, string[]> | null,
): WorkspaceSearch {
  return {
    ...search,
    utd_rating: ratings?.utd_rating,
    abs_rating: ratings?.abs_rating,
    fms_rating: ratings?.fms_rating,
    fmscn_rating: ratings?.fmscn_rating,
  };
}
