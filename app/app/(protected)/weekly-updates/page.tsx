'use client';

import Link from 'next/link';
import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ArrowLeft, CalendarDays, Database, FileText, Menu } from 'lucide-react';
import { parseAsString, useQueryState } from 'nuqs';

import {
  getArticles,
  getDatabases,
  getWeeklyUpdates,
  type WeeklyArticle,
  type WeeklyDatabaseUpdate,
  type WeeklyJournalUpdate,
  type JournalId,
} from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { SearchBar } from '@/components/feature/search-bar';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { cn } from '@/lib/utils';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';

const DATE_FORMATTER = new Intl.DateTimeFormat('zh-CN', {
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  timeZone: 'UTC',
});
const WEEKLY_VISIBLE_PAGE_SIZE = 25;
const WEEKLY_PREFETCH_THRESHOLD = 25;
const WEEKLY_SEARCH_PAGE_SIZE = 200;

type WeeklySearchOptions = {
  database: string;
  journalId: JournalId;
  query: string;
  windowEnd: string;
  windowStart: string;
};

function formatDate(value?: string): string {
  if (!value) {
    return '未知日期';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return DATE_FORMATTER.format(date);
}

function selectDefaultDatabase(
  databases: string[],
  currentDb: string,
  preferredDb: string,
): string {
  if (databases.length === 0) {
    return '';
  }
  if (currentDb && databases.includes(currentDb)) {
    return currentDb;
  }
  if (preferredDb && databases.includes(preferredDb)) {
    return preferredDb;
  }
  return databases[0];
}

function selectDefaultJournal(
  journals: WeeklyJournalUpdate[],
  currentJournalId: JournalId | null,
): JournalId | null {
  if (journals.length === 0) {
    return null;
  }
  if (currentJournalId === null) {
    return journals[0].journal_id;
  }
  if (journals.some((item) => item.journal_id === currentJournalId)) {
    return currentJournalId;
  }
  return journals[0].journal_id;
}

function getJournalLabel(journal: WeeklyJournalUpdate): string {
  if (journal.journal_title && journal.journal_title.trim()) {
    return journal.journal_title;
  }
  return `期刊 ${journal.journal_id}`;
}

function chunkArticles(articles: WeeklyArticle[], size: number): WeeklyArticle[][] {
  const pages: WeeklyArticle[][] = [];
  for (let index = 0; index < articles.length; index += size) {
    pages.push(articles.slice(index, index + size));
  }
  return pages;
}

/**
 * Convert a weekly ISO timestamp into an inclusive article date filter.
 *
 * @param value - Weekly window timestamp.
 * @returns UTC calendar date.
 */
function normalizeWeeklyWindowDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    throw new Error('每周更新时间窗口无效');
  }
  return date.toISOString().slice(0, 10);
}

/**
 * Fetch every article-search cursor page for one weekly journal window.
 *
 * @param options - Database, journal, query, and weekly window filters.
 * @returns All search result articles after complete pagination.
 */
async function searchWeeklyArticles(options: WeeklySearchOptions): Promise<WeeklyArticle[]> {
  const params = new URLSearchParams();
  params.append('journal_id', String(options.journalId));
  params.set('q', options.query);
  params.set('limit', String(WEEKLY_SEARCH_PAGE_SIZE));
  params.set('date_from', normalizeWeeklyWindowDate(options.windowStart));
  params.set('date_to', normalizeWeeklyWindowDate(options.windowEnd));

  const articles: WeeklyArticle[] = [];
  const seenCursors = new Set<string>();
  let cursor: string | null = null;

  while (true) {
    const page = await getArticles(params, cursor, false, options.database);
    articles.push(...page.items);
    const nextCursor = page.page.next_cursor?.trim() || null;
    if (!nextCursor) {
      if (page.page.has_more) {
        throw new Error('全文检索分页缺少下一页游标');
      }
      return articles;
    }
    if (seenCursors.has(nextCursor)) {
      throw new Error('全文检索分页游标重复');
    }
    seenCursors.add(nextCursor);
    cursor = nextCursor;
  }
}

/**
 * Intersect search results with weekly articles while retaining weekly payload order.
 *
 * @param weeklyArticles - Ordered weekly payload articles.
 * @param searchedArticles - Fully paginated article search results.
 * @returns Matching weekly articles in weekly payload order.
 */
function filterWeeklyArticlesBySearchMatches(
  weeklyArticles: WeeklyArticle[],
  searchedArticles: WeeklyArticle[],
): WeeklyArticle[] {
  const matchedArticleIds = new Set<string>();
  for (const article of searchedArticles) {
    matchedArticleIds.add(article.article_id);
  }
  return weeklyArticles.filter((article) => matchedArticleIds.has(article.article_id));
}

type JournalPanelProps = {
  className?: string;
  contentClassName?: string;
  availableDatabases: string[];
  effectiveSelectedDb: string;
  journals: WeeklyJournalUpdate[];
  effectiveSelectedJournalId: JournalId | null;
  onDatabaseChange: (value: string) => void;
  onSelectJournal: (journalId: JournalId) => void;
};

function JournalPanel({
  className,
  contentClassName,
  availableDatabases,
  effectiveSelectedDb,
  journals,
  effectiveSelectedJournalId,
  onDatabaseChange,
  onSelectJournal,
}: JournalPanelProps) {
  return (
    <Card className={cn('min-h-0 overflow-hidden', className)}>
      <CardHeader className="space-y-3 pb-3">
        <CardTitle className="text-base">期刊</CardTitle>
        <div className="space-y-1.5">
          <span className="text-xs font-medium text-muted-foreground">数据库</span>
          <Select value={effectiveSelectedDb} onValueChange={onDatabaseChange}>
            <SelectTrigger className="w-full">
              <SelectValue placeholder="选择数据库" />
            </SelectTrigger>
            <SelectContent>
              {availableDatabases.map((dbName) => (
                <SelectItem key={dbName} value={dbName}>
                  {dbName}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </CardHeader>
      <CardContent className={cn('space-y-2 overflow-y-auto', contentClassName)}>
        {journals.length === 0 && (
          <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
            当前时间窗口内没有新增期刊。
          </div>
        )}

        {journals.map((journal) => {
          const active = effectiveSelectedJournalId === journal.journal_id;
          return (
            <button
              key={journal.journal_id}
              type="button"
              onClick={() => onSelectJournal(journal.journal_id)}
              className={`w-full rounded-md border p-3 text-left transition-colors ${
                active ? 'border-primary bg-primary/5' : 'border-border hover:bg-muted/40'
              }`}
            >
              <div className="flex items-center justify-between gap-2">
                <p className="line-clamp-2 min-w-0 break-words text-sm font-medium">
                  {getJournalLabel(journal)}
                </p>
                <Badge variant={active ? 'default' : 'outline'}>{journal.new_article_count}</Badge>
              </div>
            </button>
          );
        })}
      </CardContent>
    </Card>
  );
}

export default function WeeklyUpdatesPage() {
  const { user } = useAuth();
  const [weeklyQuery] = useQueryState('weekly_q', parseAsString.withDefault(''));
  const searchQuery = weeklyQuery.trim();
  const [selectedDb, setSelectedDb] = useQueryState('db', parseAsString.withDefault(''));
  const [selectedJournalId, setSelectedJournalId] = useQueryState('journal', parseAsString);

  const {
    data: weeklyData,
    isLoading: loadingWeekly,
    isError: weeklyError,
    error: weeklyErrorData,
  } = useQuery({
    queryKey: ['weekly-updates'],
    queryFn: () => getWeeklyUpdates(),
    enabled: !!user,
    staleTime: 5 * 60 * 1000,
  });

  const { data: databaseOptions } = useQuery({
    queryKey: ['meta', 'databases'],
    queryFn: () => getDatabases(),
    enabled: !!user,
    staleTime: 10 * 60 * 1000,
  });

  const dbMap = useMemo(() => {
    const map = new Map<string, WeeklyDatabaseUpdate>();
    for (const item of weeklyData?.databases ?? []) {
      map.set(item.db_name, item);
    }
    return map;
  }, [weeklyData]);

  const availableDatabases = useMemo(() => {
    if (!databaseOptions || databaseOptions.length === 0) {
      return Array.from(dbMap.keys());
    }
    const merged = new Set<string>();
    for (const item of databaseOptions) {
      merged.add(item);
    }
    for (const item of dbMap.keys()) {
      merged.add(item);
    }
    return Array.from(merged);
  }, [databaseOptions, dbMap]);

  const effectiveSelectedDb = useMemo(
    () => selectDefaultDatabase(availableDatabases, selectedDb, ''),
    [availableDatabases, selectedDb],
  );

  const selectedDbData = useMemo(() => {
    if (!effectiveSelectedDb) {
      return null;
    }
    return dbMap.get(effectiveSelectedDb) ?? null;
  }, [dbMap, effectiveSelectedDb]);

  const journals = useMemo(() => selectedDbData?.journals ?? [], [selectedDbData]);

  const effectiveSelectedJournalId = useMemo(
    () => selectDefaultJournal(journals, selectedJournalId),
    [journals, selectedJournalId],
  );

  const selectedJournal = useMemo(() => {
    if (effectiveSelectedJournalId === null) {
      return null;
    }
    return journals.find((item) => item.journal_id === effectiveSelectedJournalId) ?? null;
  }, [journals, effectiveSelectedJournalId]);

  const {
    data: searchedArticles,
    isLoading: loadingSearch,
    isError: searchError,
    error: searchErrorData,
  } = useQuery({
    queryKey: [
      'weekly-search',
      effectiveSelectedDb,
      effectiveSelectedJournalId,
      searchQuery,
      weeklyData?.window_start ?? '',
      weeklyData?.window_end ?? '',
    ],
    queryFn: async () => {
      if (
        !searchQuery ||
        !effectiveSelectedDb ||
        effectiveSelectedJournalId === null ||
        !weeklyData
      ) {
        return [];
      }
      return searchWeeklyArticles({
        database: effectiveSelectedDb,
        journalId: effectiveSelectedJournalId,
        query: searchQuery,
        windowEnd: weeklyData.window_end,
        windowStart: weeklyData.window_start,
      });
    },
    enabled: Boolean(
      searchQuery && effectiveSelectedDb && effectiveSelectedJournalId !== null && weeklyData,
    ),
    staleTime: 60 * 1000,
  });

  const filteredArticles = useMemo(() => {
    const weeklyArticles = selectedJournal?.articles ?? [];
    if (!searchQuery) {
      return weeklyArticles;
    }
    if (!searchedArticles) {
      return [];
    }

    return filterWeeklyArticlesBySearchMatches(weeklyArticles, searchedArticles);
  }, [searchedArticles, searchQuery, selectedJournal]);

  const articlePages = useMemo(
    () => chunkArticles(filteredArticles, WEEKLY_VISIBLE_PAGE_SIZE),
    [filteredArticles],
  );
  const articleListKey = `${effectiveSelectedDb}:${effectiveSelectedJournalId ?? 'none'}:${searchQuery}`;
  const { visiblePages, prefetchRef, loadMoreRef } = useVisiblePageList({
    listKey: articleListKey,
    loadedPages: articlePages.length,
    scrollContainerId: 'weekly-articles-scroll-container',
  });
  const visiblePageCount = Math.min(visiblePages, articlePages.length);
  const renderedArticles = useMemo(
    () => articlePages.slice(0, visiblePageCount).flat(),
    [articlePages, visiblePageCount],
  );
  const renderedArticleIds = renderedArticles.map((article) => article.article_id);
  const prefetchIndex = Math.max(0, renderedArticles.length - WEEKLY_PREFETCH_THRESHOLD);
  const { favoriteChecksByArticle, isFavoriteStatePending } = useFavoriteChecks(
    renderedArticleIds,
    effectiveSelectedDb,
    user?.id,
  );

  const totalDatabases = weeklyData?.databases.length ?? 0;
  const totalArticles = useMemo(() => {
    if (!weeklyData) {
      return 0;
    }
    return weeklyData.databases.reduce((sum, db) => sum + db.new_article_count, 0);
  }, [weeklyData]);

  const handleDatabaseChange = (value: string) => {
    void setSelectedDb(value);
    void setSelectedJournalId(null);
  };

  return (
    <main id="main-content" className="h-screen bg-background text-foreground">
      <div className="mx-auto flex h-full w-full max-w-[1400px] flex-col px-4 py-4 sm:px-6">
        <div className="mb-4 flex items-center justify-between gap-3">
          <div className="space-y-1">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Dialog>
                <DialogTrigger asChild>
                  <Button
                    variant="outline"
                    size="icon"
                    className="shrink-0 lg:hidden"
                    aria-label="打开期刊筛选"
                  >
                    <Menu className="h-5 w-5" />
                  </Button>
                </DialogTrigger>
                <DialogContent className="left-0 top-0 h-full w-80 max-w-[calc(100vw-2rem)] translate-x-0 translate-y-0 gap-0 overflow-hidden rounded-none p-0 lg:hidden">
                  <DialogHeader className="sr-only">
                    <DialogTitle>期刊筛选</DialogTitle>
                    <DialogDescription>选择数据库和期刊以查看每周更新。</DialogDescription>
                  </DialogHeader>
                  <div className="relative h-full w-full">
                    <JournalPanel
                      className="h-full rounded-none border-0 pt-8"
                      contentClassName="h-[calc(100%-140px)]"
                      availableDatabases={availableDatabases}
                      effectiveSelectedDb={effectiveSelectedDb}
                      journals={journals}
                      effectiveSelectedJournalId={effectiveSelectedJournalId}
                      onDatabaseChange={handleDatabaseChange}
                      onSelectJournal={(journalId) => void setSelectedJournalId(journalId)}
                    />
                  </div>
                </DialogContent>
              </Dialog>
              <CalendarDays className="h-4 w-4" />
              <span>每周新文章</span>
            </div>
            <h1 className="text-xl font-semibold tracking-tight">
              期刊每周更新
              {weeklyData
                ? ` (${formatDate(weeklyData.window_start)} - ${formatDate(weeklyData.window_end)})`
                : ''}
            </h1>
          </div>
          <Button asChild variant="outline" size="sm">
            <Link href="/">
              <ArrowLeft className="mr-2 h-4 w-4" />
              返回
            </Link>
          </Button>
        </div>

        {loadingWeekly && (
          <div className="space-y-4">
            <Skeleton className="h-20 w-full" />
            <Skeleton className="h-[70vh] w-full" />
          </div>
        )}

        {weeklyError && (
          <Card>
            <CardHeader>
              <CardTitle>加载每周更新失败</CardTitle>
              <CardDescription>
                {weeklyErrorData instanceof Error ? weeklyErrorData.message : '未知错误'}
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        {!loadingWeekly && !weeklyError && weeklyData && (
          <>
            <div className="mb-4 grid grid-cols-1 gap-3 lg:grid-cols-[340px_1fr]">
              <div className="flex flex-wrap gap-2">
                <Badge variant="secondary" className="gap-1">
                  <Database className="h-3.5 w-3.5" />
                  {totalDatabases} 个数据库
                </Badge>
                <Badge variant="secondary" className="gap-1">
                  <FileText className="h-3.5 w-3.5" />
                  {totalArticles} 篇新文章
                </Badge>
              </div>
              <SearchBar className="w-full max-w-none" queryParam="weekly_q" />
            </div>

            <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 lg:grid-cols-[340px_1fr]">
              <JournalPanel
                className="hidden lg:flex lg:flex-col"
                contentClassName="h-[calc(100%-140px)]"
                availableDatabases={availableDatabases}
                effectiveSelectedDb={effectiveSelectedDb}
                journals={journals}
                effectiveSelectedJournalId={effectiveSelectedJournalId}
                onDatabaseChange={handleDatabaseChange}
                onSelectJournal={(journalId) => void setSelectedJournalId(journalId)}
              />

              <Card className="min-h-0 overflow-hidden">
                <CardHeader className="pb-3">
                  <CardTitle className="text-base">
                    {selectedJournal ? getJournalLabel(selectedJournal) : '文章'}
                  </CardTitle>
                  <CardDescription>
                    {selectedJournal
                      ? searchQuery
                        ? loadingSearch
                          ? '正在检索本周文章…'
                          : searchError
                            ? '全文检索失败'
                            : `匹配到 ${filteredArticles.length} 篇本周文章`
                        : `本周新增 ${selectedJournal.new_article_count} 篇文章`
                      : '请选择左侧期刊'}
                  </CardDescription>
                </CardHeader>
                <CardContent
                  id="weekly-articles-scroll-container"
                  className="h-[calc(100%-88px)] space-y-3 overflow-y-auto"
                >
                  {!selectedJournal && (
                    <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
                      请选择一个期刊以查看新收录文章。
                    </div>
                  )}

                  {searchQuery && loadingSearch && (
                    <div className="space-y-2">
                      <Skeleton className="h-16 w-full" />
                      <Skeleton className="h-16 w-full" />
                    </div>
                  )}

                  {searchQuery && searchError && (
                    <div
                      role="alert"
                      className="rounded-md border border-dashed p-4 text-sm text-destructive"
                    >
                      {searchErrorData instanceof Error ? searchErrorData.message : '全文检索失败'}
                    </div>
                  )}

                  {selectedJournal &&
                    !loadingSearch &&
                    !searchError &&
                    filteredArticles.length === 0 && (
                      <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
                        {searchQuery
                          ? '该期刊中没有匹配全文检索条件的本周文章。'
                          : '该期刊暂无文章。'}
                      </div>
                    )}

                  {renderedArticles.map((article, index) => (
                    <ArticleDialogCard
                      key={article.article_id}
                      triggerRef={index === prefetchIndex ? prefetchRef : undefined}
                      article={article}
                      dbName={effectiveSelectedDb}
                      initialFolderIds={
                        favoriteChecksByArticle[article.article_id]?.map(
                          (item) => item.folder_id,
                        ) ?? []
                      }
                      isFavoriteStatePending={Boolean(user) && isFavoriteStatePending}
                    />
                  ))}

                  {visiblePageCount < articlePages.length && (
                    <div ref={loadMoreRef} className="h-1" />
                  )}
                </CardContent>
              </Card>
            </div>
          </>
        )}
      </div>
    </main>
  );
}
