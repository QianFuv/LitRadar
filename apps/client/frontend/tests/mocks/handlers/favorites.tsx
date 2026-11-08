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

/**
 * Return a bounded favorite page for component and API-flow fixtures.
 *
 * @param items - Favorite article rows.
 * @param nextCursor - Optional next page continuation.
 * @returns JSON page response matching the real cursor endpoint.
 */
export function favoriteArticlePageResponse(
  items: unknown[],
  nextCursor: string | null = null,
): Response {
  return HttpResponse.json({
    items,
    page: {
      total: null,
      limit: 50,
      offset: 0,
      next_cursor: nextCursor,
      has_more: nextCursor !== null,
    },
  });
}
