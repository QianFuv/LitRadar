/**
 * Index metadata, article, weekly update, and announcement API operations.
 */

import {
  buildApiUrl,
  buildDatabaseUrl,
  readSelectedDatabase,
  requestJson,
  resolveApiBase,
} from '@/lib/api/client';
import type {
  AnnouncementInfo,
  ArticleAccessResponse,
  Article,
  ArticleId,
  ArticlePage,
  ArticleSearchMode,
  JournalId,
  JournalOption,
  JournalRatingOptions,
  JournalRecord,
  ValueCount,
  WeeklyArticlePage,
  WeeklyUpdatesSummaryResponse,
  YearSummary,
} from '@/lib/api/types';
import { isAdvancedSearchQuery } from '@/lib/search-syntax';

const DEFAULT_ARTICLE_SEARCH_MODE: ArticleSearchMode = 'simple';

/**
 * List available index databases.
 *
 * @returns Database names.
 */
export function getDatabases(): Promise<string[]> {
  return requestJson<string[]>(buildApiUrl('/api/meta/databases'), undefined, '获取数据库失败');
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
    undefined,
    '获取领域失败',
  );
}

/**
 * Fetch available exact journal grades and journal counts for one database.
 *
 * @param dbName - Database name. Defaults to the selected database.
 * @returns Rating groups, including empty groups when no grades are available.
 */
export function getJournalRatings(dbName = readSelectedDatabase()): Promise<JournalRatingOptions> {
  return requestJson<JournalRatingOptions>(
    buildDatabaseUrl('/api/meta/ratings', dbName),
    undefined,
    '获取期刊评级失败',
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
    undefined,
    '获取期刊失败',
  );
}

/**
 * Fetch one journal record with its ISSNs, subject area, and ratings.
 *
 * @param journalId - Journal identifier.
 * @param dbName - Database name.
 * @param signal - Cancellation signal for an obsolete request.
 * @returns Journal record.
 */
export function getJournal(
  journalId: string,
  dbName: string,
  signal?: AbortSignal,
): Promise<JournalRecord> {
  return requestJson<JournalRecord>(
    buildDatabaseUrl(`/api/journals/${encodeURIComponent(journalId)}`, dbName),
    { signal },
    '获取期刊信息失败',
  );
}

/**
 * Fetch articles ranked by FTS5 similarity to one article's title.
 *
 * @param articleId - Source article identifier.
 * @param dbName - Database name.
 * @param limit - Maximum number of related articles (1–30).
 * @param signal - Cancellation signal for an obsolete request.
 * @returns Related articles, most similar first.
 */
export function getRelatedArticles(
  articleId: ArticleId,
  dbName: string,
  limit: number,
  signal?: AbortSignal,
): Promise<Article[]> {
  return requestJson<Article[]>(
    buildDatabaseUrl(
      `/api/articles/${encodeURIComponent(String(articleId))}/related`,
      dbName,
      new URLSearchParams({ limit: String(limit) }),
    ),
    { signal },
    '获取相关文章失败',
  );
}

/**
 * Fetch a cursor-paginated article page.
 *
 * @param params - Article query parameters.
 * @param pageParam - Continuation cursor or null for the initial page.
 * @param includeTotal - Whether to include total on the first page.
 * @param dbName - Database name. Defaults to the selected database.
 * @param signal - Cancellation signal for an obsolete query.
 * @returns Article page.
 */
export function getArticles(
  params: URLSearchParams,
  pageParam: string | null = null,
  includeTotal = false,
  dbName = readSelectedDatabase(),
  signal?: AbortSignal,
): Promise<ArticlePage> {
  const nextParams = new URLSearchParams(params);
  if (typeof pageParam === 'string' && pageParam.length > 0) {
    nextParams.set('cursor', pageParam);
    nextParams.delete('offset');
  }
  if (nextParams.has('q') && !nextParams.has('search_mode')) {
    nextParams.set(
      'search_mode',
      isAdvancedSearchQuery(nextParams.get('q') ?? '') ? 'advanced' : DEFAULT_ARTICLE_SEARCH_MODE,
    );
  }
  nextParams.set('include_total', includeTotal ? '1' : '0');
  return requestJson<ArticlePage>(
    buildDatabaseUrl('/api/articles', dbName, nextParams),
    { signal },
    '获取文章失败',
  );
}

/** Parameters for one bounded weekly article page. */
export type WeeklyArticlePageRequest = {
  dbName: string;
  journalId: JournalId;
  windowEnd: string;
  query?: string;
  limit?: number;
};

/**
 * Fetch weekly update counts without article bodies.
 *
 * @returns Weekly update summary and fixed pagination window.
 */
export function getWeeklyUpdatesSummary(): Promise<WeeklyUpdatesSummaryResponse> {
  return requestJson<WeeklyUpdatesSummaryResponse>(
    buildApiUrl('/api/weekly-updates/summary'),
    undefined,
    '获取每周更新摘要失败',
  );
}

/**
 * Fetch one cursor-bounded weekly article page.
 *
 * @param request - Database, journal, fixed window, search, and limit parameters.
 * @param cursor - Optional descending page cursor.
 * @param signal - Optional cancellation signal for an obsolete page request.
 * @returns Weekly article page.
 */
export function getWeeklyUpdateArticles(
  request: WeeklyArticlePageRequest,
  cursor: string | null = null,
  signal?: AbortSignal,
): Promise<WeeklyArticlePage> {
  const params = new URLSearchParams();
  params.set('db', request.dbName);
  params.set('journal_id', request.journalId);
  params.set('window_end', request.windowEnd);
  params.set('limit', String(request.limit ?? 50));
  if (request.query) {
    params.set('q', request.query);
  }
  if (cursor) {
    params.set('cursor', cursor);
  }
  return requestJson<WeeklyArticlePage>(
    buildApiUrl('/api/weekly-updates/articles', params),
    { signal },
    '获取每周更新文章失败',
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
    undefined,
    '获取公告失败',
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
 * Fetch article abstract-page and full-text access capabilities.
 *
 * @param articleId - Article id.
 * @param dbName - Database name.
 * @param signal - Cancellation signal for an obsolete access lookup.
 * @returns Article access capabilities.
 */
export function getArticleAccess(
  articleId: ArticleId,
  dbName: string,
  signal?: AbortSignal,
): Promise<ArticleAccessResponse> {
  return requestJson<ArticleAccessResponse>(
    buildDatabaseUrl(`/api/articles/${articleId}/access`, dbName),
    { signal },
    '获取文章访问状态失败',
  );
}
