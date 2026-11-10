/**
 * URL helpers and stable section identifiers for the administrator center.
 */

import type { WorkspaceSearch } from '@/lib/search-params';

/** Stable identifiers accepted by the administrator query parameter. */
export const ADMIN_SECTION_IDS = [
  'overview',
  'load',
  'users',
  'invite-codes',
  'runtime-settings',
  'scheduled-tasks',
  'announcements',
] as const;

/** One navigable category in the administrator center. */
export type AdminSectionId = (typeof ADMIN_SECTION_IDS)[number];

/**
 * Parse an administrator query value into its stable section identifier.
 *
 * @param value - Raw `admin` query value.
 * @returns Matching section identifier, or null for missing and invalid values.
 */
export function parseAdminSection(value: string | undefined): AdminSectionId | null {
  return ADMIN_SECTION_IDS.find((section) => section === value) ?? null;
}

/**
 * Return workspace search state with the administrator center opened at one section or closed.
 *
 * Opening one center closes the other; every unrelated parameter is preserved.
 *
 * @param search - Current workspace search state.
 * @param section - Section to open, or null to remove the admin parameter.
 * @returns Next workspace search state.
 */
export function withAdminSection(
  search: WorkspaceSearch,
  section: AdminSectionId | null,
): WorkspaceSearch {
  if (section) {
    return { ...search, admin: section, settings: undefined };
  }
  return { ...search, admin: undefined };
}
