'use client';

import { useInfiniteQuery, useQuery, type InfiniteData } from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf, parseAsInteger } from 'nuqs';
import {
  checkFavoritesBatch,
  getArticles,
  getCurrentDatabase,
  getFullTextUrl,
  type Article,
  type ArticlePage,
} from '@/lib/api';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Skeleton } from '@/components/ui/skeleton';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogTrigger } from '@/components/ui/dialog';
import { ExternalLink, Copy, Check } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { useInView } from 'react-intersection-observer';
import { FavoriteButton } from '@/components/feature/favorite-button';
import { useAuth } from '@/lib/auth-context';

export function ResultsList() {
  const { user, token } = useAuth();
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [visiblePageState, setVisiblePageState] = useState({
    searchKey: '',
    count: 1,
  });

  const [q] = useQueryState('q', parseAsString);
  const [areas] = useQueryState('area', parseAsArrayOf(parseAsString));
  const [journalIds] = useQueryState('journal_id', parseAsArrayOf(parseAsString));
  const [yearMin] = useQueryState('year_min', parseAsInteger);
  const [yearMax] = useQueryState('year_max', parseAsInteger);
  const searchParams = useSearchParams();
  const searchKey = searchParams.toString();
  const includeTotal = true;

  const handleCopyArticleInfo = async (article: Article) => {
      const info = [
          `标题：${article.title || '暂无'}`,
          `作者：${article.authors || '暂无'}`,
          `期刊：${article.journal_title || '暂无'}`,
          `日期：${article.date || '暂无'}`,
          article.volume && `卷号：${article.volume}`,
          article.number && `期号：${article.number}`,
          article.doi && `DOI: ${article.doi}`,
          article.doi && `链接：https://doi.org/${article.doi}`
      ].filter(Boolean).join('\n');

      await navigator.clipboard.writeText(info);
      setCopyStatus(`${article.article_id}-info`);
      setTimeout(() => setCopyStatus(null), 3000);
  };

  const handleCopyTitle = async (article: Article) => {
      await navigator.clipboard.writeText(article.title || '');
      setCopyStatus(`${article.article_id}-title`);
      setTimeout(() => setCopyStatus(null), 3000);
  };

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
  const visiblePages =
    visiblePageState.searchKey === searchKey ? visiblePageState.count : 1;
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

  useEffect(() => {
    const scrollContainer = document.getElementById('results-scroll-container');
    if (scrollContainer) {
      scrollContainer.scrollTo({ top: 0 });
      return;
    }
    window.scrollTo({ top: 0 });
  }, [searchKey]);

  const handlePrefetchChange = useCallback(
    (inView: boolean) => {
      if (!inView || !hasNextPage || isFetchingNextPage) {
        return;
      }
      if (loadedPages > visiblePages) {
        return;
      }
      fetchNextPage();
    },
    [fetchNextPage, hasNextPage, isFetchingNextPage, loadedPages, visiblePages],
  );

  const handleLoadMoreChange = useCallback(
    (inView: boolean) => {
      if (!inView) {
        return;
      }
      if (visiblePages < loadedPages) {
        setVisiblePageState((current) => {
          const currentCount = current.searchKey === searchKey ? current.count : 1;
          return {
            searchKey,
            count: Math.min(currentCount + 1, loadedPages),
          };
        });
        return;
      }
      if (hasNextPage && !isFetchingNextPage) {
        fetchNextPage();
      }
    },
    [fetchNextPage, hasNextPage, isFetchingNextPage, loadedPages, searchKey, visiblePages],
  );

  const { ref: prefetchRef } = useInView({
    threshold: 0,
    onChange: handlePrefetchChange,
  });
  const { ref: loadMoreRef } = useInView({
    threshold: 0,
    onChange: handleLoadMoreChange,
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
        <div key={article.article_id}>
          {index === prefetchIndex && (
            <div ref={prefetchRef} className="h-0" />
          )}
          <Dialog>
            <DialogTrigger asChild>
                <div className="block group cursor-pointer text-left">
                    <Card className="hover:shadow-md transition-all duration-200 border-transparent hover:border-slate-200 dark:hover:border-slate-800">
                    <CardHeader>
                        <div className="flex justify-between items-start gap-4">
                            <CardTitle className="text-lg text-slate-900 dark:text-slate-100 group-hover:text-blue-600 dark:group-hover:text-blue-400 transition-colors">
                                {highlightText(article.title)}
                            </CardTitle>
                            <div className="flex gap-2 shrink-0">
                                {article.open_access === 1 && <Badge variant="secondary" className="text-xs">开放获取</Badge>}
                                {article.in_press === 1 && <Badge variant="outline" className="text-xs">预发表</Badge>}
                            </div>
                        </div>
                        <CardDescription>
                            <span>{article.journal_title}</span>
                            {(article.volume || article.number) && (
                              <span>
                                {' '}
                                •{' '}
                                {[
                                  article.volume && `第 ${article.volume} 卷`,
                                  article.number && `第 ${article.number} 期`,
                                ]
                                  .filter(Boolean)
                                  .join(', ')}
                              </span>
                            )}
                            {article.date && <span> • {article.date}</span>}
                        </CardDescription>
                    </CardHeader>
                    <CardContent>
                        <p className="text-sm text-slate-600 dark:text-slate-400 line-clamp-3 leading-relaxed">
                            {highlightText(article.abstract)}
                        </p>
                    </CardContent>
                    </Card>
                </div>
            </DialogTrigger>
            <DialogContent className="w-[calc(100%-2rem)] max-w-[calc(100%-2rem)] md:max-w-4xl max-h-[90vh] overflow-y-auto [&>button]:hidden">
                <DialogHeader>
                    <DialogTitle className="text-xl leading-snug">
                        {article.title}
                        <Button
                            variant="ghost"
                            size="sm"
                            className="h-6 w-6 p-0 ml-2 inline-flex align-middle"
                            onClick={() => handleCopyTitle(article)}
                        >
                            {copyStatus === `${article.article_id}-title` ? (
                                <Check className="h-3 w-3 text-green-600" />
                            ) : (
                                <Copy className="h-3 w-3" />
                            )}
                        </Button>
                    </DialogTitle>
                    <DialogDescription>
                        {article.journal_title}
                        {(article.volume || article.number) && ` • ${[
                            article.volume && `第 ${article.volume} 卷`,
                            article.number && `第 ${article.number} 期`
                        ].filter(Boolean).join(', ')}`}
                        {article.date && ` • ${article.date}`}
                    </DialogDescription>
                </DialogHeader>
                <div className="space-y-6 py-4">
                    {article.authors && (
                        <div>
                            <h3 className="font-semibold mb-2 text-sm text-foreground/80">作者</h3>
                            <p className="text-sm text-muted-foreground">
                                {article.authors}
                            </p>
                        </div>
                    )}
                    
                    <div>
                        <h3 className="font-semibold mb-2 text-sm text-foreground/80">摘要</h3>
                        <p className="text-sm text-muted-foreground leading-relaxed text-justify">
                            {article.abstract || "暂无摘要。"}
                        </p>
                    </div>

                    <div className="pt-4 border-t">
                        <div className="flex flex-wrap gap-4">
                            <Button
                                variant="outline"
                                size="sm"
                                onClick={() => handleCopyArticleInfo(article)}
                            >
                                {copyStatus === `${article.article_id}-info` ? (
                                    <>
                                        <Check className="mr-2 h-4 w-4 text-green-600" />
                                        已复制
                                    </>
                                ) : (
                                    <>
                                        <Copy className="mr-2 h-4 w-4" />
                                        复制信息
                                    </>
                                )}
                            </Button>
                            {(article.doi || article.platform_id) && (
                                <a
                                    href={
                                        article.doi
                                            ? `https://doi.org/${article.doi}`
                                            : getFullTextUrl(article.article_id)
                                    }
                                    target="_blank"
                                    rel="noreferrer"
                                >
                                    <Button variant="outline" size="sm">
                                        <ExternalLink className="mr-2 h-4 w-4" />
                                        查看全文
                                    </Button>
                                </a>
                            )}
                            {user && isFavoriteStatePending ? (
                              <Button variant="outline" size="sm" disabled>
                                加载收藏...
                              </Button>
                            ) : (
                              <FavoriteButton
                                articleId={article.article_id}
                                initialFolderIds={
                                  favoriteChecksByArticle[article.article_id]?.map(
                                    (item) => item.folder_id,
                                  ) ?? []
                                }
                              />
                            )}
                        </div>
                    </div>
                </div>
            </DialogContent>
          </Dialog>
        </div>
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
