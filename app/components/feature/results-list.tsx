'use client';

import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
  type InfiniteData,
} from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf } from 'nuqs';
import {
  checkFavoritesBatch,
  getArticles,
  getCurrentDatabase,
  type ArticleId,
  type ArticlePage,
  type FavoriteCheck,
} from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { Card, CardContent, CardHeader } from '@/components/ui/card';
import { Skeleton } from '@/components/ui/skeleton';
import { useCallback, useMemo } from 'react';
import { useSearchParams } from 'next/navigation';
import { useAuth } from '@/lib/auth-context';

const MONTH_KEY_PATTERN = /^\d{4}-(0[1-9]|1[0-2])$/;
const MONTH_RANGE_SEPARATOR = '..';

/**
 * Parse the compact month range query value.
 *
 * @param value - Raw query value in YYYY-MM..YYYY-MM format.
 * @returns Ordered start and end month keys, or null when invalid.
 */
function parseMonthRange(value: string | null): [string, string] | null {
  const [startMonth = '', endMonth = ''] = (value ?? '').split(MONTH_RANGE_SEPARATOR);
  if (!MONTH_KEY_PATTERN.test(startMonth) || !MONTH_KEY_PATTERN.test(endMonth)) {
    return null;
  }
  return startMonth <= endMonth ? [startMonth, endMonth] : [endMonth, startMonth];
}

/**
 * Convert a YYYY-MM query value into the first day of that month.
 *
 * @param value - Month query value.
 * @returns ISO date string or null when invalid.
 */
function monthKeyToDateFrom(value: string | null): string | null {
  if (!value || !MONTH_KEY_PATTERN.test(value)) {
    return null;
  }
  return `${value}-01`;
}

/**
 * Convert a YYYY-MM query value into the last day of that month.
 *
 * @param value - Month query value.
 * @returns ISO date string or null when invalid.
 */
function monthKeyToDateTo(value: string | null): string | null {
  if (!value || !MONTH_KEY_PATTERN.test(value)) {
    return null;
  }
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(5, 7));
  const lastDay = new Date(year, month, 0).getDate();
  return `${value}-${String(lastDay).padStart(2, '0')}`;
}

/**
 * Resolve the cursor for the next article page.
 *
 * @param lastPage - Most recently loaded article page.
 * @returns Next cursor or undefined when pagination is complete.
 */
export function getNextArticlePageParam(lastPage: ArticlePage): string | undefined {
  return lastPage.page.next_cursor ?? undefined;
}

export function ResultsList() {
  const { user } = useAuth();
  const queryClient = useQueryClient();

  const [q] = useQueryState('q', parseAsString);
  const [areas] = useQueryState('area', parseAsArrayOf(parseAsString));
  const [journalIds] = useQueryState('journal_id', parseAsArrayOf(parseAsString));
  const [monthRange] = useQueryState('month_range', parseAsString);
  const searchParams = useSearchParams();
  const searchKey = searchParams.toString();
  const includeTotal = true;

  const params = new URLSearchParams();
  if (q) params.set('q', q);

  if (areas && areas.length > 0) {
    areas.forEach((a) => params.append('area', a));
  }
  if (journalIds && journalIds.length > 0) {
    journalIds.forEach((id) => params.append('journal_id', id));
  }

  const parsedMonthRange = parseMonthRange(monthRange);
  const dateFrom = parsedMonthRange ? monthKeyToDateFrom(parsedMonthRange[0]) : null;
  const dateTo = parsedMonthRange ? monthKeyToDateTo(parsedMonthRange[1]) : null;
  if (dateFrom) params.set('date_from', dateFrom);
  if (dateTo) params.set('date_to', dateTo);
  const paramsString = params.toString();
  const currentDb = getCurrentDatabase();

  const { data, isLoading, isError, error, fetchNextPage, hasNextPage, isFetchingNextPage } =
    useInfiniteQuery<
      ArticlePage,
      Error,
      InfiniteData<ArticlePage, string | null>,
      string[],
      string | null
    >({
      queryKey: ['articles', currentDb, paramsString],
      queryFn: ({ pageParam }) => getArticles(params, pageParam, includeTotal, currentDb),
      initialPageParam: null,
      getNextPageParam: getNextArticlePageParam,
      staleTime: 5 * 60 * 1000,
      gcTime: 10 * 60 * 1000,
    });

  const pages = data?.pages ?? [];
  const loadedPages = pages.length;
  const { visiblePages, prefetchRef, loadMoreRef } = useVisiblePageList({
    listKey: searchKey,
    loadedPages,
    hasNextPage,
    isFetchingNextPage,
    onFetchNextPage: () => void fetchNextPage(),
    scrollContainerId: 'results-scroll-container',
  });
  const visiblePageCount = Math.min(visiblePages, loadedPages);
  const visibleArticles = pages.slice(0, visiblePageCount).flatMap((page) => page.items);
  const visibleArticleIds = visibleArticles.map((article) => article.article_id);
  const favoriteBatchBaseKey = useMemo(
    () => ['fav-check-batch', user?.id, currentDb] as const,
    [currentDb, user?.id],
  );
  const cachedFavoriteChecksByArticle = queryClient
    .getQueriesData<Record<ArticleId, FavoriteCheck[]>>({
      queryKey: favoriteBatchBaseKey,
    })
    .reduce<Record<ArticleId, FavoriteCheck[]>>((merged, [, checks]) => {
      if (!checks) {
        return merged;
      }
      return { ...merged, ...checks };
    }, {});
  const missingFavoriteArticleIds = visibleArticleIds.filter(
    (articleId) => !(articleId in cachedFavoriteChecksByArticle),
  );
  const missingFavoriteArticleIdsKey = missingFavoriteArticleIds.join(',');

  const { data: fetchedFavoriteChecksByArticle = {}, isPending: isMissingFavoriteStatePending } =
    useQuery({
      queryKey: [...favoriteBatchBaseKey, 'missing', missingFavoriteArticleIdsKey],
      queryFn: () => checkFavoritesBatch(missingFavoriteArticleIds, currentDb),
      enabled: !!user && missingFavoriteArticleIds.length > 0,
      staleTime: 5 * 60 * 1000,
    });
  const favoriteChecksByArticle = useMemo(
    () => ({ ...cachedFavoriteChecksByArticle, ...fetchedFavoriteChecksByArticle }),
    [cachedFavoriteChecksByArticle, fetchedFavoriteChecksByArticle],
  );
  const isFavoriteStatePending =
    missingFavoriteArticleIds.length > 0 && isMissingFavoriteStatePending;

  const highlightTerms = useMemo(() => {
    if (!q) return [];
    const isCjk = (value: string) => /[\u4e00-\u9fff]/.test(value);
    const meetsLength = (value: string) => (isCjk(value) ? value.length >= 2 : value.length > 2);
    const terms: string[] = [];
    const phraseRegex = /"([^"]+)"/g;
    let match = phraseRegex.exec(q);
    while (match) {
      const phrase = match[1].trim();
      if (meetsLength(phrase)) {
        terms.push(phrase);
      }
      match = phraseRegex.exec(q);
    }

    const stripped = q.replace(phraseRegex, ' ');
    const tokens = stripped.split(/\s+/).filter(Boolean);
    for (const token of tokens) {
      const upper = token.toUpperCase();
      if (upper === 'AND' || upper === 'OR' || upper === 'NOT' || upper === 'NEAR') {
        continue;
      }
      let cleaned = token.replace(/[()]/g, '');
      if (!cleaned) {
        continue;
      }
      if ((cleaned.includes('{') || cleaned.includes('}')) && !cleaned.includes(':')) {
        continue;
      }
      const colonIndex = cleaned.indexOf(':');
      if (colonIndex >= 0) {
        cleaned = cleaned.slice(colonIndex + 1);
      }
      cleaned = cleaned.replace(/\*+$/, '');
      if (meetsLength(cleaned)) {
        terms.push(cleaned);
      }
    }

    return Array.from(new Set(terms));
  }, [q]);

  const highlightPattern = useMemo(() => {
    if (highlightTerms.length === 0) return null;
    const escaped = highlightTerms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    return new RegExp(`(${escaped.join('|')})`, 'gi');
  }, [highlightTerms]);

  const highlightText = useCallback(
    (text: string | null | undefined) => {
      if (!text) return null;
      if (!highlightPattern) return text;

      try {
        return text.split(highlightPattern).map((part, index) =>
          index % 2 === 1 ? (
            <span
              key={index}
              className="text-blue-600 font-bold bg-blue-50 dark:text-blue-400 dark:bg-blue-950/30 rounded-xs"
            >
              {part}
            </span>
          ) : (
            part
          ),
        );
      } catch {
        return text;
      }
    },
    [highlightPattern],
  );

  const prefetchThreshold = 25;
  const prefetchIndex = Math.max(0, visibleArticles.length - prefetchThreshold);

  if (isError) {
    return (
      <div role="alert" className="p-4 text-red-500 bg-red-50 dark:bg-red-900/20 rounded-md">
        错误：{error instanceof Error ? error.message : '未知错误'}
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="space-y-4" role="status" aria-label="正在加载搜索结果">
        {Array.from({ length: 5 }).map((_, i) => (
          <Card key={i}>
            <CardHeader>
              <Skeleton className="h-6 w-3/4" />
              <Skeleton className="h-4 w-1/4 mt-2" />
            </CardHeader>
            <CardContent>
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-full mt-2" />
            </CardContent>
          </Card>
        ))}
      </div>
    );
  }

  if (visibleArticles.length === 0) {
    return <div className="text-center p-8 text-slate-500">未找到文章。</div>;
  }

  const total = data?.pages[0]?.page.total ?? null;

  return (
    <div className="space-y-4">
      {includeTotal && typeof total === 'number' && (
        <div className="text-sm text-slate-500">共找到 {total} 条结果</div>
      )}
      {visibleArticles.map((article, index) => (
        <ArticleDialogCard
          key={article.article_id}
          triggerRef={index === prefetchIndex ? prefetchRef : undefined}
          article={article}
          dbName={currentDb}
          title={highlightText(article.title)}
          preview={highlightText(article.abstract)}
          initialFolderIds={
            favoriteChecksByArticle[article.article_id]?.map((item) => item.folder_id) ?? []
          }
          isFavoriteStatePending={Boolean(user) && isFavoriteStatePending}
        />
      ))}

      <div ref={loadMoreRef} className="h-1" />
      {isFetchingNextPage && (
        <div className="py-4 flex justify-center">
          <Skeleton className="h-8 w-48" />
        </div>
      )}
    </div>
  );
}
