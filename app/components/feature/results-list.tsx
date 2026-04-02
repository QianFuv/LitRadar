'use client';

import { useInfiniteQuery, useQuery, type InfiniteData } from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf, parseAsInteger } from 'nuqs';
import {
  checkFavoritesBatch,
  getArticles,
  getCurrentDatabase,
  type ArticlePage,
} from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { Card, CardContent, CardHeader } from '@/components/ui/card';
import { Skeleton } from '@/components/ui/skeleton';
import { useCallback, useMemo } from 'react';
import { useSearchParams } from 'next/navigation';
import { useAuth } from '@/lib/auth-context';

export function ResultsList() {
  const { user, token } = useAuth();

  const [q] = useQueryState('q', parseAsString);
  const [areas] = useQueryState('area', parseAsArrayOf(parseAsString));
  const [journalIds] = useQueryState('journal_id', parseAsArrayOf(parseAsString));
  const [yearMin] = useQueryState('year_min', parseAsInteger);
  const [yearMax] = useQueryState('year_max', parseAsInteger);
  const searchParams = useSearchParams();
  const searchKey = searchParams.toString();
  const includeTotal = true;

  const params = new URLSearchParams();
  if (q) params.set('q', q);

  if (areas && areas.length > 0) {
      areas.forEach(a => params.append('area', a));
  }
  if (journalIds && journalIds.length > 0) {
      journalIds.forEach(id => params.append('journal_id', id));
  }

  if (yearMin) params.set('date_from', `${yearMin}-01-01`);
  if (yearMax) params.set('date_to', `${yearMax}-12-31`);
  const paramsString = params.toString();

  const {
    data,
    isLoading,
    isError,
    error,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = useInfiniteQuery<
    ArticlePage,
    Error,
    InfiniteData<ArticlePage, string | null>,
    string[],
    string | null
  >({
    queryKey: ['articles', paramsString],
    queryFn: ({ pageParam }) => getArticles(params, pageParam, includeTotal, token!),
    initialPageParam: null,
    getNextPageParam: (lastPage) => lastPage.page.next_cursor ?? undefined,
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
  const visibleArticleIdsKey = visibleArticleIds.join(',');
  const currentDb = getCurrentDatabase();

  const { data: favoriteChecksByArticle = {}, isPending: isFavoriteStatePending } = useQuery({
    queryKey: ['fav-check-batch', user?.id, currentDb, visibleArticleIdsKey],
    queryFn: () => checkFavoritesBatch(token!, visibleArticleIds, currentDb),
    enabled: !!token && !!user && visibleArticleIds.length > 0,
    staleTime: 5 * 60 * 1000,
  });

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
    (text: string | undefined) => {
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
          <div className="p-4 text-red-500 bg-red-50 dark:bg-red-900/20 rounded-md">
              错误：{error instanceof Error ? error.message : '未知错误'}
          </div>
      );
  }

  if (isLoading) {
      return (
          <div className="space-y-4">
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
        <div className="text-sm text-slate-500">
          共找到 {total} 条结果
        </div>
      )}
      {visibleArticles.map((article, index) => (
        <ArticleDialogCard
          key={article.article_id}
          triggerRef={index === prefetchIndex ? prefetchRef : undefined}
          article={article}
          dbName={currentDb}
          token={token!}
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
