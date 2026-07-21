/**
 * Explicit favorites scenario handlers.
 */

import { http, HttpResponse, type RequestHandler } from 'msw';

import type { components } from '@/lib/generated/api-schema';

const API_URL = 'http://localhost/api';

type FolderScenario = components['schemas']['FolderResponse'];

/**
 * Create favorites handlers with a typed folder list.
 *
 * @param folders - Folder responses returned by the list endpoint.
 * @returns Favorites request handlers.
 */
export function createFavoriteScenarioHandlers(folders: FolderScenario[] = []): RequestHandler[] {
  return [http.get(`${API_URL}/favorites/folders`, () => HttpResponse.json(folders))];
}

/** Stable favorites happy-path handlers. */
export const FAVORITE_SCENARIO_HANDLERS = createFavoriteScenarioHandlers();
