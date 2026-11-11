/**
 * Favorite folder, membership, export, and tracking-folder API operations.
 */

import { $api, apiClient, readData, readDownloadFilename } from '@/lib/api/client';
import { apiCursorQuery, apiPathKey, apiQuery } from '@/lib/api/query';
import type {
  ArticleId,
  CitationFormat,
  FavoriteArticlePage,
  FavoriteArticleRef,
  FavoriteCheck,
  FavoriteItem,
  Folder,
  RelationGraphFavorite,
  RelationGraphFavoriteCreate,
  RelationGraphFavoritePage,
  RelationGraphFavoriteUpdate,
} from '@/lib/api/types';

const CITATION_EXPORT_EXTENSIONS: Record<CitationFormat, string> = {
  bibtex: 'bib',
  endnote: 'xml',
  ris: 'ris',
};

/** Completed favorite citation export. */
export interface FavoriteExportDownload {
  /** Citation data returned by the backend. */
  blob: Blob;
  /** Safe filename for the browser download. */
  filename: string;
}

/**
 * Fetch all folders for the current user.
 *
 * @returns Folders.
 */
export async function getFolders(): Promise<Folder[]> {
  return readData(await apiClient.GET('/api/favorites/folders'));
}

/**
 * Create a favorite folder.
 *
 * @param name - Folder name.
 * @param isTracking - Whether the folder is the tracking folder.
 * @returns Created folder.
 */
export async function createFolder(name: string, isTracking = false): Promise<Folder> {
  return readData(
    await apiClient.POST('/api/favorites/folders', { body: { name, is_tracking: isTracking } }),
  );
}

/**
 * Rename a folder.
 *
 * @param folderId - Folder id.
 * @param name - New name.
 */
export async function renameFolder(folderId: number, name: string): Promise<void> {
  await apiClient.PUT('/api/favorites/folders/{folder_id}', {
    params: { path: { folder_id: folderId } },
    body: { name },
  });
}

/**
 * Delete a folder.
 *
 * @param folderId - Folder id.
 */
export async function deleteFolder(folderId: number): Promise<void> {
  await apiClient.DELETE('/api/favorites/folders/{folder_id}', {
    params: { path: { folder_id: folderId } },
  });
}

/**
 * Read one stable favorite page without counting preceding rows.
 *
 * @param folderId - Owned favorite folder.
 * @param limit - Maximum returned rows.
 * @param cursor - Opaque server continuation or null for the first page.
 * @param signal - Optional cancellation signal for an obsolete page request.
 * @returns Favorite rows and the next cursor.
 */
export async function getFolderArticlePage(
  folderId: number,
  limit: number,
  cursor: string | null = null,
  signal?: AbortSignal,
): Promise<FavoriteArticlePage> {
  return readData(
    await apiClient.GET('/api/favorites/folders/{folder_id}/articles/page', {
      params: {
        path: { folder_id: folderId },
        query: { limit, ...(cursor !== null ? { cursor } : {}) },
      },
      signal,
    }),
  );
}

/**
 * Add an article to a folder.
 *
 * @param folderId - Folder id.
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Favorite item.
 */
export async function addFavorite(
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): Promise<FavoriteItem> {
  return readData(
    await apiClient.POST('/api/favorites/folders/{folder_id}/articles', {
      params: { path: { folder_id: folderId } },
      body: { article_id: articleId, db_name: dbName, note: '' },
    }),
  );
}

/**
 * Remove an article from a folder.
 *
 * @param folderId - Folder id.
 * @param articleId - Article id.
 * @param dbName - Database name.
 */
export async function removeFavorite(
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): Promise<void> {
  await apiClient.DELETE('/api/favorites/folders/{folder_id}/articles/{article_id}', {
    params: {
      path: { folder_id: folderId, article_id: Number(articleId) },
      query: { db_name: dbName },
    },
  });
}

/**
 * Bulk remove favorite articles from a folder.
 *
 * @param folderId - Folder id.
 * @param articles - Article references.
 * @returns Removed count.
 */
export async function bulkRemoveFavorites(
  folderId: number,
  articles: FavoriteArticleRef[],
): Promise<number> {
  const data = readData(
    await apiClient.POST('/api/favorites/folders/{folder_id}/articles/bulk-remove', {
      params: { path: { folder_id: folderId } },
      body: { articles },
    }),
  );
  return data.count;
}

/**
 * Bulk move favorite articles between folders.
 *
 * @param folderId - Source folder id.
 * @param targetFolderId - Target folder id.
 * @param articles - Article references.
 * @returns Moved count.
 */
export async function bulkMoveFavorites(
  folderId: number,
  targetFolderId: number,
  articles: FavoriteArticleRef[],
): Promise<number> {
  const data = readData(
    await apiClient.POST('/api/favorites/folders/{folder_id}/articles/bulk-move', {
      params: { path: { folder_id: folderId } },
      body: { target_folder_id: targetFolderId, articles },
    }),
  );
  return data.count;
}

/**
 * Download a folder citation export through the authenticated API client.
 *
 * @param folderId - Folder id.
 * @param format - Citation format.
 * @returns Citation blob and safe download filename.
 */
export async function downloadFavoriteExport(
  folderId: number,
  format: CitationFormat,
): Promise<FavoriteExportDownload> {
  const result = await apiClient.GET('/api/favorites/folders/{folder_id}/export', {
    params: { path: { folder_id: folderId }, query: { format } },
    parseAs: 'blob',
  });
  return {
    blob: readData(result),
    filename:
      readDownloadFilename(result.response) ?? `favorites.${CITATION_EXPORT_EXTENSIONS[format]}`,
  };
}

/**
 * Check which folders contain an article.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Favorite checks.
 */
export async function checkFavorite(
  articleId: ArticleId,
  dbName: string,
): Promise<FavoriteCheck[]> {
  return readData(
    await apiClient.GET('/api/favorites/check', {
      params: { query: { article_id: Number(articleId), db_name: dbName } },
    }),
  );
}

/**
 * Check favorite state for many articles.
 *
 * @param articleIds - Article ids.
 * @param dbName - Database name.
 * @param signal - Cancellation signal for the active lookup.
 * @returns Favorite checks keyed by article id.
 */
export async function checkFavoritesBatch(
  articleIds: ArticleId[],
  dbName: string,
  signal?: AbortSignal,
): Promise<Record<ArticleId, FavoriteCheck[]>> {
  if (articleIds.length === 0) return {};
  const data = readData(
    await apiClient.POST('/api/favorites/check/batch', {
      body: { article_ids: articleIds, db_name: dbName },
      signal,
    }),
  );
  return Object.fromEntries(data.map((item) => [item.article_id, item.folders]));
}

/**
 * Set the tracking folder.
 *
 * @param folderId - Folder id.
 */
export async function setTrackingFolder(folderId: number): Promise<void> {
  await apiClient.PUT('/api/favorites/tracking', { body: { folder_id: folderId } });
}

/**
 * Fetch one cursor page of the current user's saved relation graphs, newest change first.
 *
 * @param limit - Page size.
 * @param cursor - Opaque cursor from the previous page, or null for the first page.
 * @param signal - Optional abort signal.
 * @returns Saved graph summaries and the next cursor.
 */
async function getRelationGraphFavoritePage(
  limit: number,
  cursor: string | null = null,
  signal?: AbortSignal,
): Promise<RelationGraphFavoritePage> {
  return readData(
    await apiClient.GET('/api/favorites/graphs', {
      params: { query: { limit, ...(cursor !== null ? { cursor } : {}) } },
      signal,
    }),
  );
}

/**
 * Save a relation graph snapshot as a new favorite.
 *
 * @param input - Title, database, root article, node count, and snapshot.
 * @returns Saved graph.
 */
export async function createRelationGraphFavorite(
  input: RelationGraphFavoriteCreate,
): Promise<RelationGraphFavorite> {
  return readData(await apiClient.POST('/api/favorites/graphs', { body: input }));
}

/**
 * Rename a saved relation graph or replace its snapshot.
 *
 * @param graphId - Saved graph id.
 * @param input - New title, or a new snapshot with its node count, or both.
 * @returns Updated saved graph.
 */
export async function updateRelationGraphFavorite(
  graphId: number,
  input: RelationGraphFavoriteUpdate,
): Promise<RelationGraphFavorite> {
  return readData(
    await apiClient.PUT('/api/favorites/graphs/{graph_id}', {
      params: { path: { graph_id: graphId } },
      body: input,
    }),
  );
}

/**
 * Delete a saved relation graph.
 *
 * @param graphId - Saved graph id.
 */
export async function deleteRelationGraphFavorite(graphId: number): Promise<void> {
  await apiClient.DELETE('/api/favorites/graphs/{graph_id}', {
    params: { path: { graph_id: graphId } },
  });
}

/**
 * Query of the current user's favorite folders.
 *
 * @returns Query options.
 */
export function foldersQuery() {
  return apiQuery($api.queryOptions('get', '/api/favorites/folders'));
}

/** Rows per favorite folder page. */
const FAVORITE_FOLDER_PAGE_SIZE = 50;

/**
 * Cursor-paged query of one folder's favorite articles.
 *
 * @param folderId - Owned favorite folder.
 * @returns Infinite query options.
 */
export function folderArticlePagesQuery(folderId: number) {
  const queryKey = $api.queryOptions('get', '/api/favorites/folders/{folder_id}/articles/page', {
    params: { path: { folder_id: folderId }, query: { limit: FAVORITE_FOLDER_PAGE_SIZE } },
  }).queryKey;
  return apiCursorQuery(
    queryKey,
    (cursor, signal) => getFolderArticlePage(folderId, FAVORITE_FOLDER_PAGE_SIZE, cursor, signal),
    (page) => page.page.next_cursor,
  );
}

/** Key prefix shared by every folder article page query. */
export const FOLDER_ARTICLES_QUERY_PREFIX = apiPathKey(
  'get',
  '/api/favorites/folders/{folder_id}/articles/page',
);

/**
 * Query of the folders that contain one article.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Query options.
 */
export function favoriteCheckQuery(articleId: ArticleId, dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/favorites/check', {
      params: { query: { article_id: Number(articleId), db_name: dbName } },
    }),
  );
}

/** Key prefix shared by every single-article favorite check. */
export const FAVORITE_CHECK_QUERY_PREFIX = apiPathKey('get', '/api/favorites/check');

/**
 * Key of one batch favorite lookup for a set of articles.
 *
 * @param articleIds - Sorted unique article ids.
 * @param dbName - Database name.
 * @returns openapi-react-query key of the batch request.
 */
export function favoriteCheckBatchQueryKey(articleIds: ArticleId[], dbName: string) {
  return $api.queryOptions('post', '/api/favorites/check/batch', {
    body: { article_ids: articleIds, db_name: dbName },
  }).queryKey;
}

/** Key prefix shared by every batch favorite lookup. */
export const FAVORITE_CHECK_BATCH_QUERY_PREFIX = apiPathKey('post', '/api/favorites/check/batch');

/**
 * Cursor-paged query of the current user's saved relation graphs.
 *
 * @param limit - Page size.
 * @returns Infinite query options.
 */
export function relationGraphFavoritePagesQuery(limit: number) {
  const queryKey = $api.queryOptions('get', '/api/favorites/graphs', {
    params: { query: { limit } },
  }).queryKey;
  return apiCursorQuery(
    queryKey,
    (cursor, signal) => getRelationGraphFavoritePage(limit, cursor, signal),
    (page) => page.next_cursor,
  );
}

/** Key prefix shared by every saved relation graph page query. */
export const RELATION_GRAPH_FAVORITES_QUERY_PREFIX = apiPathKey('get', '/api/favorites/graphs');

/**
 * Query of one saved relation graph with its snapshot.
 *
 * @param graphId - Saved graph id.
 * @returns Query options.
 */
export function relationGraphFavoriteQuery(graphId: number) {
  return apiQuery(
    $api.queryOptions('get', '/api/favorites/graphs/{graph_id}', {
      params: { path: { graph_id: graphId } },
    }),
  );
}
