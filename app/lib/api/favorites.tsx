/**
 * Favorite folder, membership, export, and tracking-folder API operations.
 */

import { buildApiUrl, requestJson, resolveApiBase } from '@/lib/api/client';
import type {
  ArticleId,
  CitationFormat,
  FavoriteArticleItem,
  FavoriteArticleRef,
  FavoriteBatchCheckItem,
  FavoriteCheck,
  FavoriteItem,
  Folder,
} from '@/lib/api/types';

/**
 * Fetch all folders for the current user.
 *
 * @returns Folders.
 */
export function getFolders(): Promise<Folder[]> {
  return requestJson<Folder[]>(
    buildApiUrl('/api/favorites/folders'),
    null,
    undefined,
    '获取收藏夹失败',
  );
}

/**
 * Create a favorite folder.
 *
 * @param name - Folder name.
 * @param isTracking - Whether the folder is the tracking folder.
 * @returns Created folder.
 */
export function createFolder(name: string, isTracking = false): Promise<Folder> {
  return requestJson<Folder>(
    buildApiUrl('/api/favorites/folders'),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ name, is_tracking: isTracking }),
    },
    '创建收藏夹失败',
  );
}

/**
 * Rename a folder.
 *
 * @param folderId - Folder id.
 * @param name - New name.
 */
export async function renameFolder(folderId: number, name: string): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}`),
    null,
    {
      method: 'PUT',
      body: JSON.stringify({ name }),
    },
    '重命名收藏夹失败',
  );
}

/**
 * Delete a folder.
 *
 * @param folderId - Folder id.
 */
export async function deleteFolder(folderId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}`),
    null,
    { method: 'DELETE' },
    '删除收藏夹失败',
  );
}

/**
 * Fetch articles in a folder.
 *
 * @param folderId - Folder id.
 * @param limit - Page size.
 * @param offset - Page offset.
 * @returns Favorite articles.
 */
export function getFolderArticles(
  folderId: number,
  limit: number,
  offset: number,
): Promise<FavoriteArticleItem[]> {
  const params = new URLSearchParams({ limit: String(limit), offset: String(offset) });
  return requestJson<FavoriteArticleItem[]>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles`, params),
    null,
    undefined,
    '获取收藏文章失败',
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
export function addFavorite(
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): Promise<FavoriteItem> {
  return requestJson<FavoriteItem>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ article_id: articleId, db_name: dbName, note: '' }),
    },
    '添加收藏失败',
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
  const params = new URLSearchParams({ db_name: dbName });
  await requestJson<unknown>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/${articleId}`, params),
    null,
    { method: 'DELETE' },
    '移除收藏失败',
  );
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
  const data = await requestJson<{ count: number }>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/bulk-remove`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ articles }),
    },
    '批量移除收藏失败',
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
  const data = await requestJson<{ count: number }>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/bulk-move`),
    null,
    {
      method: 'POST',
      body: JSON.stringify({ target_folder_id: targetFolderId, articles }),
    },
    '批量移动收藏失败',
  );
  return data.count;
}

/**
 * Build a folder export URL.
 *
 * @param folderId - Folder id.
 * @param format - Citation format.
 * @returns Export URL.
 */
export function getExportUrl(folderId: number, format: CitationFormat): string {
  const url = new URL(`/api/favorites/folders/${folderId}/export`, resolveApiBase());
  url.searchParams.set('format', format);
  return url.toString();
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
  const params = new URLSearchParams({ article_id: articleId, db_name: dbName });
  try {
    return await requestJson<FavoriteCheck[]>(
      buildApiUrl('/api/favorites/check', params),
      null,
      undefined,
      '获取收藏状态失败',
    );
  } catch {
    return [];
  }
}

/**
 * Check favorite state for many articles.
 *
 * @param articleIds - Article ids.
 * @param dbName - Database name.
 * @returns Favorite checks keyed by article id.
 */
export async function checkFavoritesBatch(
  articleIds: ArticleId[],
  dbName: string,
): Promise<Record<ArticleId, FavoriteCheck[]>> {
  if (articleIds.length === 0) {
    return {};
  }
  try {
    const data = await requestJson<FavoriteBatchCheckItem[]>(
      buildApiUrl('/api/favorites/check/batch'),
      null,
      {
        method: 'POST',
        body: JSON.stringify({ article_ids: articleIds, db_name: dbName }),
      },
      '获取收藏状态失败',
    );
    return Object.fromEntries(data.map((item) => [item.article_id, item.folders]));
  } catch {
    return {};
  }
}

/**
 * Set the tracking folder.
 *
 * @param folderId - Folder id.
 */
export async function setTrackingFolder(folderId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/favorites/tracking'),
    null,
    {
      method: 'PUT',
      body: JSON.stringify({ folder_id: folderId }),
    },
    '设置追踪文件夹失败',
  );
}
