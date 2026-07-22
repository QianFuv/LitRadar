'use client';

import { useInfiniteQuery, useQueryClient, type InfiniteData } from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf } from 'nuqs';
import { getArticles, type ArticlePage } from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { Card, CardContent, CardHeader } from '@/components/ui/card';
import { Skeleton } from '@/components/ui/skeleton';
import { useCallback, useMemo, type ReactNode } from 'react';
import { useSearchParams } from 'next/navigation';
import { useAuth } from '@/lib/auth-context';
import { getMonthRangeDateBounds } from '@/lib/article-filters';
import { createFtsHighlightPattern, parseFtsHighlightTerms } from '@/lib/fts-highlight';
import { useSelectedDatabase } from '@/lib/selected-database';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';

type ResultsListProps = {
  filterSummary?: ReactNode;
};

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
 * Reject a cursor that would revisit a page already requested in the current pagination chain.
 *
 * @param page - Newly fetched article page.
 * @param pageParam - Cursor used to fetch the page, or null for the first page.
 * @param cachedPageParams - Page parameters from the currently cached pagination chain.
 * @returns The validated article page.
 */
function validateArticlePageCursor(
  page: ArticlePage,
  pageParam: string | null,
  cachedPageParams: readonly (string | null)[],
): ArticlePage {
  const nextCursor = page.page.next_cursor;
  if (!nextCursor || pageParam === null) {
    return page;
  }

  const cachedPageIndex = cachedPageParams.indexOf(pageParam);
  const requestedPageParams =
    cachedPageIndex >= 0
      ? cachedPageParams.slice(0, cachedPageIndex + 1)
      : [...cachedPageParams, pageParam];
  if (requestedPageParams.includes(nextCursor)) {
    throw new Error('全文检索分页游标重复');
  }

  return page;
}

/**
 * Fetch and render the filtered, progressively visible article result list.
 *
 * @param props - Optional content rendered beneath the known result total.
 * @returns Search result summary, article cards, and pagination sentinels.
 */
export function ResultsList({ filterSummary }: ResultsListProps) {
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

  const dateBounds = getMonthRangeDateBounds(monthRange);
  if (dateBounds) {
    params.set('date_from', dateBounds.dateFrom);
    params.set('date_to', dateBounds.dateTo);
  }
  const paramsString = params.toString();
  const currentDb = useSelectedDatabase();
  const queryKey = ['articles', currentDb, paramsString];

  const { data, isLoading, isError, error, fetchNextPage, hasNextPage, isFetchingNextPage } =
    useInfiniteQuery<
      ArticlePage,
      Error,
      InfiniteData<ArticlePage, string | null>,
      string[],
      string | null
    >({
      queryKey,
      queryFn: async ({ pageParam }) => {
        const page = await getArticles(params, pageParam, includeTotal, currentDb);
        const cachedData =
          queryClient.getQueryData<InfiniteData<ArticlePage, string | null>>(queryKey);
        return validateArticlePageCursor(page, pageParam, cachedData?.pageParams ?? []);
      },
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
  const total = data?.pages[0]?.page.total ?? null;
  let resultContent: ReactNode;

  if (isError) {
    resultContent = (
      <div role="alert" className="p-4 text-red-500 bg-red-50 dark:bg-red-900/20 rounded-md">
        错误：{error instanceof Error ? error.message : '未知错误'}
      </div>
    );
  } else if (isLoading) {
    resultContent = (
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
  } else if (visibleArticles.length === 0) {
    resultContent = <div className="p-8 text-center text-muted-foreground">未找到文章。</div>;
  } else {
    resultContent = (
      <>
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
      </>
    );
  }

  return (
    <div className="space-y-4">
      {includeTotal && typeof total === 'number' && (
        <div className="text-sm text-muted-foreground">共找到 {total} 条结果</div>
      )}
      {filterSummary && (
        <div
          data-testid="filter-summary-slot"
          className="sticky top-0 z-20 bg-background py-2 empty:hidden"
        >
          {filterSummary}
        </div>
      )}
      {resultContent}
    </div>
  );
}
