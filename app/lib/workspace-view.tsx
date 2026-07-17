/**
 * Canonical root-workspace view identifiers, parser, and destinations.
 */

import { parseAsStringLiteral } from 'nuqs';

export const WORKSPACE_VIEW_IDS = ['search', 'favorites', 'weekly-updates'] as const;

export type WorkspaceView = (typeof WORKSPACE_VIEW_IDS)[number];

export const WORKSPACE_VIEW_PARSER = parseAsStringLiteral(WORKSPACE_VIEW_IDS).withDefault('search');

const WORKSPACE_VIEW_HREFS: Readonly<Record<WorkspaceView, string>> = {
  search: '/',
  favorites: '/?view=favorites',
  'weekly-updates': '/?view=weekly-updates',
};

/**
 * Return the canonical root URL for one article-workspace view.
 *
 * @param view - Valid workspace view identifier.
 * @returns Canonical root destination.
 */
export function getWorkspaceViewHref(view: WorkspaceView): string {
  return WORKSPACE_VIEW_HREFS[view];
}
