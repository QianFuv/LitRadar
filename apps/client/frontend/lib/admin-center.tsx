/**
 * URL helpers and stable section identifiers for the administrator center.
 */

/** Stable identifiers accepted by the administrator query parameter. */
export const ADMIN_SECTION_IDS = [
  'overview',
  'users',
  'invite-codes',
  'runtime-settings',
  'scheduled-tasks',
  'announcements',
] as const;

/** One navigable category in the administrator center. */
export type AdminSectionId = (typeof ADMIN_SECTION_IDS)[number];

/** Minimal read-only search-parameter snapshot accepted by administrator URL builders. */
export type AdminSearchParams = Pick<URLSearchParams, 'toString'>;

/**
 * Parse an administrator query value into its stable section identifier.
 *
 * @param value - Raw `admin` query value.
 * @returns Matching section identifier, or null for missing and invalid values.
 */
export function parseAdminSection(value: string | null): AdminSectionId | null {
  return ADMIN_SECTION_IDS.find((section) => section === value) ?? null;
}

/**
 * Build an administrator-center URL while preserving unrelated query parameters.
 *
 * @param pathname - Current protected pathname.
 * @param searchParams - Current read-only query snapshot.
 * @param section - Section to open, or null to remove administrator state.
 * @returns Relative application URL with the requested administrator state.
 */
export function buildAdminCenterHref(
  pathname: string,
  searchParams: AdminSearchParams,
  section: AdminSectionId | null,
): string {
  const nextSearchParams = new URLSearchParams(searchParams.toString());
  if (section) {
    nextSearchParams.set('admin', section);
    nextSearchParams.delete('settings');
  } else {
    nextSearchParams.delete('admin');
  }
  const query = nextSearchParams.toString();
  return query ? `${pathname}?${query}` : pathname;
}
