/** Authenticated, database-scoped queries for backend-owned original CFP data. */

import { buildDatabaseUrl, requestJson } from '@/lib/api/client';
import type { CfpCatalogResponse, CfpNoticePage } from '@/lib/api/types';

/** Fetch the complete lightweight journal catalog without downloading notice bodies. */
export function getCfpJournals(dbName: string, signal?: AbortSignal): Promise<CfpCatalogResponse> {
  return requestJson<CfpCatalogResponse>(
    buildDatabaseUrl('/api/cfp/journals', dbName),
    { signal },
    '获取征稿期刊失败',
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
export function getCfpNotices(
  query: CfpNoticeQuery,
  cursor: string | null = null,
  signal?: AbortSignal,
): Promise<CfpNoticePage> {
  const params = new URLSearchParams({
    include_closed: String(query.includeClosed),
    limit: String(query.limit ?? 50),
  });
  if (cursor) params.set('cursor', cursor);
  return requestJson<CfpNoticePage>(
    buildDatabaseUrl(
      `/api/cfp/journals/${encodeURIComponent(query.catalogId)}/notices`,
      query.dbName,
      params,
    ),
    { signal },
    '获取征稿需求失败',
  );
}
