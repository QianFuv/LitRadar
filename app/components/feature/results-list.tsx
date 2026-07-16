'use client';

import { useInfiniteQuery, type InfiniteData } from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf } from 'nuqs';
import { getArticles, type ArticlePage } from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { Card, CardContent, CardHeader } from '@/components/ui/card';
import { Skeleton } from '@/components/ui/skeleton';
import { useCallback, useMemo } from 'react';
import { useSearchParams } from 'next/navigation';
import { useAuth } from '@/lib/auth-context';
import { getMonthRangeDateBounds } from '@/lib/article-filters';
import { createFtsHighlightPattern, parseFtsHighlightTerms } from '@/lib/fts-highlight';
import { useSelectedDatabase } from '@/lib/selected-database';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';

/**
 * Resolve the cursor for the next article page.
 *
 * @param lastPage - Most recently loaded article page.
 * @returns Next cursor or undefined when pagination is complete.
 */
export function getNextArticlePageParam(lastPage: ArticlePage): string | undefined {
  return lastPage.page.next_cursor ?? undefined;
}

/**
 * Fetch and render the filtered, progressively visible article result list.
 *
 * @returns Search result summary, article cards, and pagination sentinels.
 */
export function ResultsList() {
  const { user } = useAuth();

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

  const dateBounds = getMonthRangeDateBounds(monthRange);
  if (dateBounds) {
    params.set('date_from', dateBounds.dateFrom);
    params.set('date_to', dateBounds.dateTo);
  }
  const paramsString = params.toString();
  const currentDb = useSelectedDatabase();

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
  const { favoriteChecksByArticle, isFavoriteStatePending } = useFavoriteChecks(
    visibleArticleIds,
    currentDb,
    user?.id,
  );

  const highlightTerms = useMemo(() => parseFtsHighlightTerms(q), [q]);

  const highlightPattern = useMemo(
    () => createFtsHighlightPattern(highlightTerms),
    [highlightTerms],
  );

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
    return <div className="p-8 text-center text-muted-foreground">未找到文章。</div>;
  }

  const total = data?.pages[0]?.page.total ?? null;

  return (
    <div className="space-y-4">
      {includeTotal && typeof total === 'number' && (
        <div className="text-sm text-muted-foreground">共找到 {total} 条结果</div>
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
