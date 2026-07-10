/**
 * Index metadata, article, weekly update, and announcement API operations.
 */

import {
  DEFAULT_DATABASE,
  buildApiUrl,
  buildDatabaseUrl,
  readSelectedDatabase,
  requestJson,
  resolveApiBase,
} from '@/lib/api/client';
import type {
  AnnouncementInfo,
  Article,
  ArticleAccessResponse,
  ArticleId,
  ArticlePage,
  JournalOption,
  ValueCount,
  WeeklyUpdatesResponse,
  YearSummary,
} from '@/lib/api/types';

/**
 * List available index databases.
 *
 * @returns Database names.
 */
export async function getDatabases(): Promise<string[]> {
  try {
    return await requestJson<string[]>(
      buildApiUrl('/api/meta/databases'),
      null,
      undefined,
      '获取数据库失败',
    );
  } catch {
    return [DEFAULT_DATABASE];
  }
}

/**
 * Fetch metadata areas for a database.
 *
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Area counts.
 */
export function getAreas(dbName = readSelectedDatabase()): Promise<ValueCount[]> {
  return requestJson<ValueCount[]>(
    buildDatabaseUrl('/api/meta/areas', dbName),
    null,
    undefined,
    '获取领域失败',
  );
}

/**
 * Fetch indexed year summaries for a database.
 *
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Year summaries.
 */
export function getYears(dbName = readSelectedDatabase()): Promise<YearSummary[]> {
  return requestJson<YearSummary[]>(
    buildDatabaseUrl('/api/years', dbName),
    null,
    undefined,
    '获取年份失败',
  );
}

/**
 * Fetch journal filter options for a database.
 *
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Journal options.
 */
export function getJournalOptions(dbName = readSelectedDatabase()): Promise<JournalOption[]> {
  return requestJson<JournalOption[]>(
    buildDatabaseUrl('/api/meta/journals', dbName),
    null,
    undefined,
    '获取期刊失败',
  );
}

/**
 * Fetch a cursor-paginated article page.
 *
 * @param params - Article query parameters.
 * @param pageParam - Cursor or offset page parameter.
 * @param includeTotal - Whether to include total on the first page.
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Article page.
 */
export function getArticles(
  params: URLSearchParams,
  pageParam: string | number | null = null,
  includeTotal = false,
  dbName = readSelectedDatabase(),
): Promise<ArticlePage> {
  const nextParams = new URLSearchParams(params);
  if (typeof pageParam === 'string' && pageParam.length > 0) {
    nextParams.set('cursor', pageParam);
    nextParams.delete('offset');
  }
  if (typeof pageParam === 'number') {
    nextParams.set('offset', String(pageParam));
  }
  nextParams.set('include_total', includeTotal ? '1' : '0');
  return requestJson<ArticlePage>(
    buildDatabaseUrl('/api/articles', dbName, nextParams),
    null,
    undefined,
    '获取文章失败',
  );
}

/**
 * Fetch weekly update data.
 *
 * @returns Weekly update response.
 */
export function getWeeklyUpdates(): Promise<WeeklyUpdatesResponse> {
  return requestJson<WeeklyUpdatesResponse>(
    buildApiUrl('/api/weekly-updates'),
    null,
    undefined,
    '获取每周更新失败',
  );
}

/**
 * Fetch enabled announcements.
 *
 * @returns Announcements.
 */
export function getAnnouncements(): Promise<AnnouncementInfo[]> {
  return requestJson<AnnouncementInfo[]>(
    buildApiUrl('/api/announcements'),
    null,
    undefined,
    '获取公告失败',
  );
}

/**
 * Build the full-text redirect URL for an article.
 *
 * @param articleId - Article id.
 * @returns Full-text URL.
 */
export function getFullTextUrl(articleId: ArticleId): string {
  return getFullTextUrlForDatabase(articleId, readSelectedDatabase());
}

/**
 * Build the full-text redirect URL for a specific database.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Full-text URL.
 */
export function getFullTextUrlForDatabase(articleId: ArticleId, dbName: string): string {
  const url = new URL(`/api/articles/${articleId}/fulltext`, resolveApiBase());
  url.searchParams.set('db', dbName);
  return url.toString();
}

/**
 * Fetch article detail and full-text access capabilities.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Article access capabilities.
 */
export function getArticleAccess(
  articleId: ArticleId,
  dbName: string,
): Promise<ArticleAccessResponse> {
  return requestJson<ArticleAccessResponse>(
    buildDatabaseUrl(`/api/articles/${articleId}/access`, dbName),
    null,
    undefined,
    '获取文章访问状态失败',
  );
}

/**
 * Fetch one article by id from a database.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Article record.
 */
export function getArticleById(articleId: ArticleId, dbName: string): Promise<Article> {
  return requestJson<Article>(
    buildDatabaseUrl(`/api/articles/${articleId}`, dbName),
    null,
    undefined,
    '获取文章详情失败',
  );
}
