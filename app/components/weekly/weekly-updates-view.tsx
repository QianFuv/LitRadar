'use client';

/**
 * Weekly article updates rendered inside the shared article workspace.
 */

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { CalendarDays, Database, FileText } from 'lucide-react';
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
import { WorkspaceSidebar } from '@/components/feature/sidebar';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
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

/**
 * Format a weekly-window timestamp for the Chinese interface.
 *
 * @param value - ISO timestamp or date value.
 * @returns Formatted UTC date or a safe fallback.
 */
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

/**
 * Select an available database while retaining a valid current or preferred value.
 *
 * @param databases - Available database names.
 * @param currentDb - Current URL selection.
 * @param preferredDb - Optional preferred fallback.
 * @returns Effective database name or an empty string.
 */
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

/**
 * Select an available journal while retaining a valid current value.
 *
 * @param journals - Journals in the selected weekly database.
 * @param currentJournalId - Current URL selection.
 * @returns Effective journal identifier or null.
 */
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

/**
 * Resolve a human-readable weekly journal label.
 *
 * @param journal - Weekly journal payload.
 * @returns Journal title or identifier fallback.
 */
function getJournalLabel(journal: WeeklyJournalUpdate): string {
  if (journal.journal_title && journal.journal_title.trim()) {
    return journal.journal_title;
  }
  return `期刊 ${journal.journal_id}`;
}

/**
 * Split weekly articles into progressively visible client-side pages.
 *
 * @param articles - Ordered weekly articles.
 * @param size - Maximum items per visible page.
 * @returns Ordered article page chunks.
 */
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

type WeeklySidebarProps = {
  availableDatabases: string[];
  effectiveSelectedDb: string;
  journals: WeeklyJournalUpdate[];
  effectiveSelectedJournalId: JournalId | null;
  onDatabaseChange: (value: string) => void;
  onSelectJournal: (journalId: JournalId) => void;
};

/**
 * Render database and journal selection inside the shared workspace sidebar frame.
 *
 * @param props - Weekly database/journal state and selection actions.
 * @returns Weekly workspace sidebar.
 */
function WeeklySidebar({
  availableDatabases,
  effectiveSelectedDb,
  journals,
  effectiveSelectedJournalId,
  onDatabaseChange,
  onSelectJournal,
}: WeeklySidebarProps) {
  return (
    <WorkspaceSidebar
      headerContent={
        <div className="space-y-4 border-t border-sidebar-border pt-4">
          <div className="space-y-1.5">
            <div className="flex items-center gap-2 text-sm font-semibold text-sidebar-foreground">
              <Database className="size-4" aria-hidden="true" />
              <span>数据库</span>
            </div>
            <Select value={effectiveSelectedDb} onValueChange={onDatabaseChange}>
              <SelectTrigger className="w-full bg-sidebar">
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

          <div className="space-y-2">
            <h2 className="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
              期刊
            </h2>
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
                  aria-pressed={active}
                  onClick={() => onSelectJournal(journal.journal_id)}
                  className={`w-full rounded-md border p-3 text-left transition-colors ${
                    active ? 'border-primary bg-primary/5' : 'border-border hover:bg-muted/40'
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <p className="line-clamp-2 min-w-0 break-words text-sm font-medium">
                      {getJournalLabel(journal)}
                    </p>
                    <Badge variant={active ? 'default' : 'outline'}>
                      {journal.new_article_count}
                    </Badge>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      }
    />
  );
}

/**
 * Render weekly database and journal updates inside the shared article workspace.
 *
 * @returns Weekly-updates workspace view.
 */
export function WeeklyUpdatesView() {
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
    scrollContainerId: 'results-scroll-container',
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
    <WorkspaceShell
      sidebar={
        <WeeklySidebar
          availableDatabases={availableDatabases}
          effectiveSelectedDb={effectiveSelectedDb}
          journals={journals}
          effectiveSelectedJournalId={effectiveSelectedJournalId}
          onDatabaseChange={handleDatabaseChange}
          onSelectJournal={(journalId) => void setSelectedJournalId(journalId)}
        />
      }
      sidebarOpenLabel="打开期刊筛选"
      sidebarDialogTitle="期刊筛选"
      sidebarDialogDescription="选择数据库和期刊以查看每周更新。"
      toolbar={
        <div className="flex min-w-0 flex-1 items-center gap-3 md:mx-auto md:max-w-4xl">
          <CalendarDays className="size-5 shrink-0" aria-hidden="true" />
          <div className="min-w-0">
            <p className="text-xs text-muted-foreground">每周新文章</p>
            <h1 className="truncate text-xl font-semibold tracking-tight">
              期刊每周更新
              {weeklyData
                ? ` (${formatDate(weeklyData.window_start)} - ${formatDate(weeklyData.window_end)})`
                : ''}
            </h1>
          </div>
        </div>
      }
    >
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
          <div className="flex flex-col gap-3 rounded-lg border bg-card p-4 sm:flex-row sm:items-center">
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
            <SearchBar className="w-full max-w-none sm:min-w-0 sm:flex-1" queryParam="weekly_q" />
          </div>

          <Card className="min-w-0">
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
            <CardContent className="space-y-3">
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
                    {searchQuery ? '该期刊中没有匹配全文检索条件的本周文章。' : '该期刊暂无文章。'}
                  </div>
                )}

              {renderedArticles.map((article, index) => (
                <ArticleDialogCard
                  key={article.article_id}
                  triggerRef={index === prefetchIndex ? prefetchRef : undefined}
                  article={article}
                  dbName={effectiveSelectedDb}
                  initialFolderIds={
                    favoriteChecksByArticle[article.article_id]?.map((item) => item.folder_id) ?? []
                  }
                  isFavoriteStatePending={Boolean(user) && isFavoriteStatePending}
                />
              ))}

              {visiblePageCount < articlePages.length && <div ref={loadMoreRef} className="h-1" />}
            </CardContent>
          </Card>
        </>
      )}
    </WorkspaceShell>
  );
}
