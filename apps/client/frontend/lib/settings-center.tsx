/**
 * URL helpers and stable section identifiers for the aggregated settings center.
 */

import type { WorkspaceSearch } from '@/lib/search-params';

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

/**
 * Parse a settings query value into its stable section identifier.
 *
 * @param value - Raw `settings` query value.
 * @returns Matching section identifier, or null for missing and invalid values.
 */
export function parseSettingsSection(value: string | undefined): SettingsSectionId | null {
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
 * Return workspace search state with the settings center opened at one section or closed.
 *
 * Opening one center closes the other; every unrelated parameter is preserved.
 *
 * @param search - Current workspace search state.
 * @param section - Section to open, or null to remove the settings parameter.
 * @returns Next workspace search state.
 */
export function withSettingsSection(
  search: WorkspaceSearch,
  section: SettingsSectionId | null,
): WorkspaceSearch {
  if (section) {
    return { ...search, settings: section, admin: undefined };
  }
  return { ...search, settings: undefined };
}
