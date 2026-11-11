/** Authenticated, database-scoped queries for backend-owned original CFP data. */

import { $api, apiClient, readData } from '@/lib/api/client';
import { apiCursorQuery, apiQuery } from '@/lib/api/query';
import type { CfpCatalogResponse, CfpNoticePage } from '@/lib/api/types';

/** Fetch the complete lightweight journal catalog without downloading notice bodies. */
export async function getCfpJournals(
  dbName: string,
  signal?: AbortSignal,
): Promise<CfpCatalogResponse> {
  return readData(
    await apiClient.GET('/api/cfp/journals', { params: { query: { db: dbName } }, signal }),
  );
}

/** Scope and filter for one journal's persisted original notices. */
export interface CfpNoticeQuery {
  dbName: string;
  catalogId: string;
  includeClosed: boolean;
  limit?: number;
}

/** Fetch one server-evaluated page, preserving its opaque cursor and cancellation signal. */
export async function getCfpNotices(
  query: CfpNoticeQuery,
  cursor: string | null = null,
  signal?: AbortSignal,
): Promise<CfpNoticePage> {
  return readData(
    await apiClient.GET('/api/cfp/journals/{catalog_id}/notices', {
      params: {
        path: { catalog_id: query.catalogId },
        query: {
          db: query.dbName,
          include_closed: query.includeClosed,
          limit: query.limit ?? 50,
          ...(cursor ? { cursor } : {}),
        },
      },
      signal,
    }),
  );
}

/**
 * Query of a database's call-for-papers journal catalog.
 *
 * @param dbName - Database name.
 * @returns Query options.
 */
export function cfpJournalsQuery(dbName: string) {
  return apiQuery(
    $api.queryOptions('get', '/api/cfp/journals', { params: { query: { db: dbName } } }),
  );
}

/**
 * Cursor-paged query of one journal's call-for-papers notices.
 *
 * @param query - Database, journal, closed-notice filter, and optional page size.
 * @returns Infinite query options.
 */
export function cfpNoticesQuery(query: CfpNoticeQuery) {
  const queryKey = $api.queryOptions('get', '/api/cfp/journals/{catalog_id}/notices', {
    params: {
      path: { catalog_id: query.catalogId },
      query: { db: query.dbName, include_closed: query.includeClosed, limit: query.limit ?? 50 },
    },
  }).queryKey;
  return apiCursorQuery(
    queryKey,
    (cursor, signal) => getCfpNotices(query, cursor, signal),
    (page) => page.page.next_cursor,
  );
}
