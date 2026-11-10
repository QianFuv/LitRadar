/**
 * Canonical root-workspace view identifiers and destinations.
 */

export const WORKSPACE_VIEW_IDS = [
  'search',
  'favorites',
  'weekly-updates',
  'cfp-tracking',
  'relations',
] as const;

export type WorkspaceView = (typeof WORKSPACE_VIEW_IDS)[number];

/** View shown when the URL names no view or an unknown one. */
export const DEFAULT_WORKSPACE_VIEW = 'search' satisfies WorkspaceView;

/**
 * Return the canonical root search state for one article-workspace view.
 *
 * @param view - Valid workspace view identifier.
 * @returns Search state that names only the view, or nothing for the default search view.
 */
export function getWorkspaceViewSearch(view: WorkspaceView) {
  return view === DEFAULT_WORKSPACE_VIEW ? {} : { view };
}
