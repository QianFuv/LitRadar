/**
 * Index metadata, article, weekly update, and announcement API operations.
 */

import { $api, apiClient, readData, readSelectedDatabase, resolveApiBase } from '@/lib/api/client';
import { apiCursorQuery, apiPathKey, apiQuery, validatedApiQuery } from '@/lib/api/query';
import type { operations } from '@/lib/generated/api-schema';
import type {
  ArticleId,
  ArticlePage,
  ArticleSearchMode,
  JournalId,
  WeeklyArticlePage,
} from '@/lib/api/types';
import { parseAnnouncementList } from '@/lib/api-contract';
import { isAdvancedSearchQuery } from '@/lib/search-syntax';

const DEFAULT_ARTICLE_SEARCH_MODE: ArticleSearchMode = 'simple';

/** Article search filters accepted by `GET /api/articles`. */
export type ArticleSearchQuery = NonNullable<operations['list_articles']['parameters']['query']>;

/**
 * Complete article search filters with the database, the total flag, and the search mode the
 * query text needs.
 *
 * @param query - Search filters.
 * @param dbName - Database name used when the filters name none.
 * @returns Filters as sent to the server, without a cursor.
 */
function normalizeArticleSearch(query: ArticleSearchQuery, dbName: string): ArticleSearchQuery {
  const textMode =
    query.q && isAdvancedSearchQuery(query.q) ? 'advanced' : DEFAULT_ARTICLE_SEARCH_MODE;
  const searchMode = query.search_mode ?? (query.q ? textMode : undefined);
  return {
    ...query,
    db: query.db ?? dbName,
    include_total: query.include_total ?? false,
    ...(searchMode ? { search_mode: searchMode } : {}),
  };
}

/**
 * Key of the cursor-paged article search for one set of filters.
 *
 * @param query - Search filters.
 * @param dbName - Database name used when the filters name none.
 * @returns openapi-react-query key of the first page.
 */
export function articlePagesQueryKey(query: ArticleSearchQuery, dbName: string) {
  return $api.queryOptions('get', '/api/articles', {
    params: { query: normalizeArticleSearch(query, dbName) },
  }).queryKey;
}

/**
 * Fetch one article page.
 *
 * @param query - Search filters.
 * @param cursor - Opaque cursor of the requested page, or null for the first page.
 * @param dbName - Database name used when the filters name none.
 * @param signal - Optional cancellation signal.
 * @returns Article page.
 */
export async function getArticles(
  query: ArticleSearchQuery,
  cursor: string | null = null,
  dbName = readSelectedDatabase(),
  signal?: AbortSignal,
): Promise<ArticlePage> {
  return readData(
    await apiClient.GET('/api/articles', {
      params: {
        query: { ...normalizeArticleSearch(query, dbName), ...(cursor ? { cursor } : {}) },
      },
      signal,
    }),
  );
}

/** Weekly article page request for one journal and update window. */
export type WeeklyArticlePageRequest = {
  dbName: string;
  journalId: JournalId;
  windowEnd: string;
  query?: string;
  limit?: number;
};

/**
 * Fetch one page of a journal's weekly update articles.
 *
 * @param request - Database, journal, window, optional filter, and page size.
 * @param cursor - Opaque cursor of the requested page, or null for the first page.
 * @param signal - Optional cancellation signal.
 * @returns Weekly article page.
 */
async function getWeeklyUpdateArticles(
  request: WeeklyArticlePageRequest,
  cursor: string | null = null,
  signal?: AbortSignal,
): Promise<WeeklyArticlePage> {
  return readData(
    await apiClient.GET('/api/weekly-updates/articles', {
      params: {
        query: {
          db: request.dbName,
          journal_id: Number(request.journalId),
          window_end: request.windowEnd,
          limit: request.limit ?? 50,
          ...(request.query ? { q: request.query } : {}),
          ...(cursor ? { cursor } : {}),
        },
      },
      signal,
    }),
  );
}

/** Article action resolved online through a stable LitRadar route. */
export type ArticleActionKind = 'abstract' | 'fulltext';

/**
 * Build an online article action URL for a specific database.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @param action - Online action kind.
 * @returns Stable LitRadar action URL.
 */
export function getArticleActionUrlForDatabase(
  articleId: ArticleId,
  dbName: string,
  action: ArticleActionKind,
): string {
  const url = new URL(`/api/articles/${articleId}/${action}`, resolveApiBase());
  url.searchParams.set('db', dbName);
  return url.toString();
}

/**
 * Query of the available index databases.
 *
 * @returns Query options.
 */
export function databasesQuery() {
  return apiQuery($api.queryOptions('get', '/api/meta/databases'));
}

/**
 * Query of a database's subject areas.
 *
 * @param dbName - Database name.
 * @returns Query options.
 */
export function areasQuery(dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/meta/areas', { params: { query: { db: dbName } } }),
  );
}

/**
 * Query of a database's journal filter options.
 *
 * @param dbName - Database name.
 * @returns Query options.
 */
export function journalOptionsQuery(dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/meta/journals', { params: { query: { db: dbName } } }),
  );
}

/**
 * Query of a database's publication years.
 *
 * @param dbName - Database name.
 * @returns Query options.
 */
export function yearsQuery(dbName: string) {
  return apiQuery($api.queryOptions('get', '/api/years', { params: { query: { db: dbName } } }));
}

/**
 * Query of a database's journal rating options.
 *
 * @param dbName - Database name.
 * @returns Query options.
 */
export function journalRatingsQuery(dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/meta/ratings', { params: { query: { db: dbName } } }),
  );
}

/**
 * Query of one journal record.
 *
 * @param journalId - Journal id.
 * @param dbName - Database name.
 * @returns Query options.
 */
export function journalQuery(journalId: JournalId, dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/journals/{journal_id}', {
      params: { path: { journal_id: Number(journalId) }, query: { db: dbName } },
    }),
  );
}

/**
 * Query of one article.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Query options.
 */
export function articleQuery(articleId: ArticleId, dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/articles/{article_id}', {
      params: { path: { article_id: Number(articleId) }, query: { db: dbName } },
    }),
  );
}

/**
 * Query of the articles related to one article.
 *
 * @param articleId - Source article id.
 * @param dbName - Database name.
 * @param limit - Maximum returned articles.
 * @returns Query options.
 */
export function relatedArticlesQuery(articleId: ArticleId, dbName: string, limit: number) {
  return apiQuery(
    $api.queryOptions('get', '/api/articles/{article_id}/related', {
      params: { path: { article_id: Number(articleId) }, query: { db: dbName, limit } },
    }),
  );
}

/**
 * Query of an article's abstract-page and full-text access capabilities.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @returns Query options.
 */
export function articleAccessQuery(articleId: ArticleId, dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/articles/{article_id}/access', {
      params: { path: { article_id: Number(articleId) }, query: { db: dbName } },
    }),
  );
}

/** Key prefix shared by every article access query. */
export const ARTICLE_ACCESS_QUERY_PREFIX = apiPathKey('get', '/api/articles/{article_id}/access');

/**
 * Query of the weekly updates summary across databases.
 *
 * @returns Query options.
 */
export function weeklyUpdatesSummaryQuery() {
  return apiQuery($api.queryOptions('get', '/api/weekly-updates/summary'));
}

/**
 * Cursor-paged query of one journal's weekly update articles.
 *
 * @param request - Database, journal, window, optional filter, and page size.
 * @returns Infinite query options.
 */
export function weeklyUpdateArticlesQuery(request: WeeklyArticlePageRequest) {
  const queryKey = $api.queryOptions('get', '/api/weekly-updates/articles', {
    params: {
      query: {
        db: request.dbName,
        journal_id: Number(request.journalId),
        window_end: request.windowEnd,
        limit: request.limit ?? 50,
        ...(request.query ? { q: request.query } : {}),
      },
    },
  }).queryKey;
  return apiCursorQuery(
    queryKey,
    (cursor, signal) => getWeeklyUpdateArticles(request, cursor, signal),
    (page) => page.page.next_cursor,
  );
}

/**
 * Query of the enabled announcements, with priorities validated.
 *
 * @returns Query options.
 */
export function announcementsQuery() {
  return validatedApiQuery($api.queryOptions('get', '/api/announcements'), parseAnnouncementList);
}
