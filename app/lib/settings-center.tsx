/**
 * URL helpers and stable section identifiers for the aggregated settings center.
 */

/** Stable identifiers accepted by the settings query parameter. */
export const SETTINGS_SECTION_IDS = [
  'general',
  'tracking',
  'notifications',
  'data-sources',
  'account',
  'tokens',
] as const;

/** One navigable category in the aggregated settings center. */
export type SettingsSectionId = (typeof SETTINGS_SECTION_IDS)[number];

/** Minimal read-only search-parameter snapshot accepted by URL builders. */
export type SettingsSearchParams = Pick<URLSearchParams, 'toString'>;

/**
 * Parse a settings query value into its stable section identifier.
 *
 * @param value - Raw `settings` query value.
 * @returns Matching section identifier, or null for missing and invalid values.
 */
export function parseSettingsSection(value: string | null): SettingsSectionId | null {
  return SETTINGS_SECTION_IDS.find((section) => section === value) ?? null;
}

/**
 * Return whether a section belongs to the shared tracking-settings draft.
 *
 * @param section - Settings section to classify.
 * @returns Whether the section uses the tracking view model.
 */
export function isTrackingSettingsSection(
  section: SettingsSectionId,
): section is Extract<SettingsSectionId, 'tracking' | 'notifications'> {
  return section === 'tracking' || section === 'notifications';
}

/**
 * Build a settings-center URL while preserving every unrelated query parameter.
 *
 * @param pathname - Current protected pathname.
 * @param searchParams - Current read-only query snapshot.
 * @param section - Section to open, or null to remove settings state.
 * @returns Relative application URL with the requested settings state.
 */
export function buildSettingsCenterHref(
  pathname: string,
  searchParams: SettingsSearchParams,
  section: SettingsSectionId | null,
): string {
  const nextSearchParams = new URLSearchParams(searchParams.toString());
  if (section) {
    nextSearchParams.set('settings', section);
  } else {
    nextSearchParams.delete('settings');
  }
  const query = nextSearchParams.toString();
  return query ? `${pathname}?${query}` : pathname;
}
