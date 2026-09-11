/**
 * Favorite folder, membership, export, and tracking-folder API operations.
 */

import { buildApiUrl, requestDownload, requestJson } from '@/lib/api/client';
import type {
  ArticleId,
  CitationFormat,
  FavoriteArticlePage,
  FavoriteArticleRef,
  FavoriteBatchCheckItem,
  FavoriteCheck,
  FavoriteItem,
  Folder,
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
export function getFolders(): Promise<Folder[]> {
  return requestJson<Folder[]>(buildApiUrl('/api/favorites/folders'), undefined, '获取收藏夹失败');
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
    { method: 'DELETE' },
    '删除收藏夹失败',
  );
}

/**
 * Read one stable favorite page without counting preceding rows.
 *
 * @param folderId - Owned favorite folder.
 * @param limit - Maximum returned rows.
 * @param cursor - Opaque server continuation or null for the first page.
 * @returns Favorite rows and the next cursor.
 */
export function getFolderArticlePage(
  folderId: number,
  limit: number,
  cursor: string | null = null,
): Promise<FavoriteArticlePage> {
  const params = new URLSearchParams({ limit: String(limit) });
  if (cursor !== null) {
    params.set('cursor', cursor);
  }
  return requestJson<FavoriteArticlePage>(
    buildApiUrl(`/api/favorites/folders/${folderId}/articles/page`, params),
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
    {
      method: 'POST',
      body: JSON.stringify({ target_folder_id: targetFolderId, articles }),
    },
    '批量移动收藏失败',
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
  const params = new URLSearchParams({ format });
  const download = await requestDownload(
    buildApiUrl(`/api/favorites/folders/${folderId}/export`, params),
    undefined,
    '导出引用失败',
  );
  return {
    blob: download.blob,
    filename: download.filename ?? `favorites.${CITATION_EXPORT_EXTENSIONS[format]}`,
  };
}

/**
 * Check which folders contain an article.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Favorite checks.
 */
export function checkFavorite(articleId: ArticleId, dbName: string): Promise<FavoriteCheck[]> {
  const params = new URLSearchParams({ article_id: articleId, db_name: dbName });
  return requestJson<FavoriteCheck[]>(
    buildApiUrl('/api/favorites/check', params),
    undefined,
    '获取收藏状态失败',
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
  const data = await requestJson<FavoriteBatchCheckItem[]>(
    buildApiUrl('/api/favorites/check/batch'),
    { method: 'POST', body: JSON.stringify({ article_ids: articleIds, db_name: dbName }), signal },
    '获取收藏状态失败',
  );
  return Object.fromEntries(data.map((item) => [item.article_id, item.folders]));
}

/**
 * Set the tracking folder.
 *
 * @param folderId - Folder id.
 */
export async function setTrackingFolder(folderId: number): Promise<void> {
  await requestJson<unknown>(
    buildApiUrl('/api/favorites/tracking'),
    {
      method: 'PUT',
      body: JSON.stringify({ folder_id: folderId }),
    },
    '设置追踪文件夹失败',
  );
}
