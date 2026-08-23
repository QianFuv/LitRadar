'use client';

/**
 * Weekly article updates rendered inside the shared article workspace.
 */

import { useMemo } from 'react';
import { useInfiniteQuery, useQuery, type InfiniteData } from '@tanstack/react-query';
import { CalendarDays, Database, FileText } from 'lucide-react';
import { parseAsString, useQueryState } from 'nuqs';

import {
  getDatabases,
  getWeeklyUpdateArticles,
  getWeeklyUpdatesSummary,
  type WeeklyArticle,
  type WeeklyArticlePage,
  type WeeklyDatabaseSummary,
  type WeeklyJournalSummary,
  type JournalId,
} from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { SearchBar } from '@/components/feature/search-bar';
import { WorkspaceSidebar } from '@/components/feature/sidebar';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { Badge } from '@/components/ui/badge';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { StateMessage } from '@/components/ui/state-message';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';
import { cn } from '@/lib/utils';

const DATE_FORMATTER = new Intl.DateTimeFormat('zh-CN', {
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  timeZone: 'UTC',
});
const WEEKLY_ARTICLE_PAGE_SIZE = 50;
const WEEKLY_PREFETCH_THRESHOLD = 25;

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
  journals: WeeklyJournalSummary[],
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
function getJournalLabel(journal: WeeklyJournalSummary): string {
  if (journal.journal_title && journal.journal_title.trim()) {
    return journal.journal_title;
  }
  return `期刊 ${journal.journal_id}`;
}

/**
 * Resolve the next weekly article cursor without revisiting an existing page.
 *
 * @param lastPage - Most recently loaded weekly page.
 * @param pageParams - Cursors already used by the pagination chain.
 * @returns Next cursor or undefined when pagination is complete.
 */
function getNextWeeklyArticlePageParam(
  lastPage: WeeklyArticlePage,
  pageParams: readonly (string | null)[],
): string | undefined {
  if (!lastPage.page.has_more) {
    return undefined;
  }
  const nextCursor = lastPage.page.next_cursor?.trim();
  if (!nextCursor) {
    throw new Error('每周更新分页缺少下一页游标');
  }
  if (pageParams.includes(nextCursor)) {
    throw new Error('每周更新分页游标重复');
  }
  return nextCursor;
}

/**
 * Flatten visible server pages while protecting the list from duplicate boundary rows.
 *
 * @param pages - Loaded weekly article pages.
 * @param visiblePageCount - Number of loaded pages currently visible.
 * @returns Ordered unique weekly articles.
 */
function flattenWeeklyArticlePages(
  pages: WeeklyArticlePage[],
  visiblePageCount: number,
): WeeklyArticle[] {
  const articles: WeeklyArticle[] = [];
  const seenArticleIds = new Set<string>();
  for (const page of pages.slice(0, visiblePageCount)) {
    for (const article of page.items) {
      if (seenArticleIds.has(article.article_id)) {
        continue;
      }
      seenArticleIds.add(article.article_id);
      articles.push(article);
    }
  }
  return articles;
}

type WeeklySidebarProps = {
  availableDatabases: string[];
  effectiveSelectedDb: string;
  journals: WeeklyJournalSummary[];
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
                  className={cn(
                    'motion-control w-full rounded-md border p-3 text-left transition-[background-color,border-color,color,box-shadow]',
                    active
                      ? 'border-sidebar-border bg-sidebar-accent text-sidebar-accent-foreground shadow-vercel-ring'
                      : 'border-transparent text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground',
                  )}
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
  const stateTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);

  const {
    data: weeklySummary,
    isLoading: loadingWeekly,
    isError: weeklyError,
    error: weeklyErrorData,
  } = useQuery({
    queryKey: ['weekly-updates-summary'],
    queryFn: () => getWeeklyUpdatesSummary(),
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
    const map = new Map<string, WeeklyDatabaseSummary>();
    for (const item of weeklySummary?.databases ?? []) {
      map.set(item.db_name, item);
    }
    return map;
  }, [weeklySummary]);

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
    () =>
      selectDefaultDatabase(
        availableDatabases,
        selectedDb,
        weeklySummary?.databases[0]?.db_name ?? '',
      ),
    [availableDatabases, selectedDb, weeklySummary],
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

  const weeklyArticleQueryKey: string[] = [
    'weekly-update-articles',
    effectiveSelectedDb,
    effectiveSelectedJournalId ?? '',
    searchQuery,
    weeklySummary?.window_end ?? '',
  ];
  const {
    data: weeklyArticleData,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isPending: loadingArticles,
    isError: articleError,
    error: articleErrorData,
  } = useInfiniteQuery<
    WeeklyArticlePage,
    Error,
    InfiniteData<WeeklyArticlePage, string | null>,
    string[],
    string | null
  >({
    queryKey: weeklyArticleQueryKey,
    queryFn: ({ pageParam }) => {
      if (!effectiveSelectedDb || effectiveSelectedJournalId === null || !weeklySummary) {
        throw new Error('每周更新文章请求缺少必要筛选条件');
      }
      return getWeeklyUpdateArticles(
        {
          dbName: effectiveSelectedDb,
          journalId: effectiveSelectedJournalId,
          windowEnd: weeklySummary.window_end,
          query: searchQuery || undefined,
          limit: WEEKLY_ARTICLE_PAGE_SIZE,
        },
        pageParam,
      );
    },
    initialPageParam: null,
    getNextPageParam: (lastPage, _pages, _lastPageParam, pageParams) =>
      getNextWeeklyArticlePageParam(lastPage, pageParams),
    enabled: Boolean(
      user && effectiveSelectedDb && effectiveSelectedJournalId !== null && weeklySummary,
    ),
    staleTime: 60 * 1000,
  });

  const articlePages = useMemo(() => weeklyArticleData?.pages ?? [], [weeklyArticleData]);
  const articleListKey = `${effectiveSelectedDb}:${effectiveSelectedJournalId ?? 'none'}:${searchQuery}:${weeklySummary?.window_end ?? ''}`;
  const { visiblePages, prefetchRef, loadMoreRef } = useVisiblePageList({
    listKey: articleListKey,
    loadedPages: articlePages.length,
    hasNextPage,
    isFetchingNextPage,
    onFetchNextPage: () => {
      if (hasNextPage && !isFetchingNextPage) {
        void fetchNextPage();
      }
    },
    scrollContainerId: 'results-scroll-container',
  });
  const visiblePageCount = Math.min(visiblePages, articlePages.length);
  const renderedArticles = useMemo(
    () => flattenWeeklyArticlePages(articlePages, visiblePageCount),
    [articlePages, visiblePageCount],
  );
  const renderedArticleIds = renderedArticles.map((article) => article.article_id);
  const prefetchIndex = Math.max(0, renderedArticles.length - WEEKLY_PREFETCH_THRESHOLD);
  const { favoriteChecksByArticle, isFavoriteStatePending } = useFavoriteChecks(
    renderedArticleIds,
    effectiveSelectedDb,
    user?.id,
  );

  const totalDatabases = weeklySummary?.databases.length ?? 0;
  const totalArticles = useMemo(() => {
    if (!weeklySummary) {
      return 0;
    }
    return weeklySummary.databases.reduce((sum, db) => sum + db.new_article_count, 0);
  }, [weeklySummary]);

  const weeklyState = weeklyError ? 'error' : loadingWeekly || !weeklySummary ? 'loading' : 'ready';
  const articleState = !selectedJournal
    ? 'no-journal'
    : loadingArticles
      ? 'loading'
      : articleError
        ? 'error'
        : renderedArticles.length === 0
          ? 'empty'
          : 'results';
  const announcementRole =
    weeklyState === 'error' || (weeklyState === 'ready' && articleState === 'error')
      ? 'alert'
      : 'status';
  const announcement =
    weeklyState === 'loading'
      ? '正在加载每周更新摘要'
      : weeklyState === 'error'
        ? `加载每周更新失败：${weeklyErrorData instanceof Error ? weeklyErrorData.message : '未知错误'}`
        : articleState === 'no-journal'
          ? '请选择一个期刊以查看新收录文章'
          : articleState === 'loading'
            ? `正在加载“${selectedJournal ? getJournalLabel(selectedJournal) : ''}”的本周文章`
            : articleState === 'error'
              ? `加载本周文章失败：${articleErrorData instanceof Error ? articleErrorData.message : '未知错误'}`
              : articleState === 'empty'
                ? searchQuery
                  ? '该期刊中没有匹配全文检索条件的本周文章'
                  : '该期刊暂无文章'
                : isFetchingNextPage
                  ? '正在加载更多本周文章'
                  : `已加载 ${renderedArticles.length} 篇本周文章`;

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
              {weeklySummary
                ? ` (${formatDate(weeklySummary.window_start)} - ${formatDate(weeklySummary.window_end)})`
                : ''}
            </h1>
          </div>
        </div>
      }
    >
      <p
        key={`${weeklyState}-${articleState}-${announcement}`}
        data-testid="weekly-state-announcement"
        className="sr-only"
        role={announcementRole}
        aria-label={announcement}
        aria-live={announcementRole === 'alert' ? 'assertive' : 'polite'}
        aria-atomic="true"
      >
        {announcement}
      </p>
      <MotionPresence mode="wait">
        {weeklyState === 'loading' ? (
          <MotionDiv
            key="weekly-loading"
            data-weekly-state="loading"
            className="space-y-4"
            aria-hidden="true"
            variants={FADE_UP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
            transition={stateTransition}
          >
            <Skeleton className="h-20 w-full" />
            <Skeleton className="h-[70vh] w-full" />
          </MotionDiv>
        ) : weeklyState === 'error' ? (
          <MotionDiv
            key="weekly-error"
            data-weekly-state="error"
            variants={FADE_UP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
            transition={stateTransition}
          >
            <StateMessage
              isLive={false}
              tone="danger"
              title="加载每周更新失败"
              description={
                weeklyErrorData instanceof Error ? weeklyErrorData.message : '请稍后重试。'
              }
            />
          </MotionDiv>
        ) : (
          weeklySummary && (
            <MotionDiv
              key="weekly-ready"
              data-weekly-state="ready"
              className="space-y-3"
              variants={FADE_UP_VARIANTS}
              initial="hidden"
              animate="visible"
              exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
              transition={stateTransition}
            >
              <section className="flex flex-col gap-3 rounded-lg bg-muted/30 p-3 shadow-vercel-ring sm:flex-row sm:items-center">
                <MotionPresence mode="wait">
                  <MotionDiv
                    key={weeklySummary.window_end}
                    data-weekly-summary-key={weeklySummary.window_end}
                    className="flex shrink-0 flex-wrap gap-2"
                    variants={FADE_UP_VARIANTS}
                    initial="hidden"
                    animate="visible"
                    exit={{ opacity: 0, pointerEvents: 'none', y: -2 }}
                    transition={stateTransition}
                  >
                    <Badge variant="secondary" className="gap-1">
                      <Database className="h-3.5 w-3.5" aria-hidden="true" />
                      {totalDatabases} 个数据库
                    </Badge>
                    <Badge variant="secondary" className="gap-1">
                      <FileText className="h-3.5 w-3.5" aria-hidden="true" />
                      {totalArticles} 篇新文章
                    </Badge>
                  </MotionDiv>
                </MotionPresence>
                <SearchBar
                  className="w-full max-w-none sm:min-w-0 sm:flex-1"
                  queryParam="weekly_q"
                />
              </section>

              <section className="min-w-0 space-y-3" aria-label="每周文章">
                <MotionPresence mode="wait">
                  <MotionDiv
                    key={`${effectiveSelectedDb}:${effectiveSelectedJournalId ?? 'none'}`}
                    data-weekly-journal={effectiveSelectedJournalId ?? 'none'}
                    className="rounded-lg bg-card px-4 py-3 shadow-vercel-ring"
                    variants={FADE_UP_VARIANTS}
                    initial="hidden"
                    animate="visible"
                    exit={{ opacity: 0, pointerEvents: 'none', y: -3 }}
                    transition={stateTransition}
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <h2 className="truncate text-lg font-semibold tracking-tight">
                          {selectedJournal ? getJournalLabel(selectedJournal) : '选择期刊'}
                        </h2>
                        <p className="mt-1 text-sm text-muted-foreground">
                          {selectedJournal
                            ? searchQuery
                              ? articleState === 'loading'
                                ? '正在检索本周文章…'
                                : articleState === 'error'
                                  ? '全文检索失败'
                                  : `已加载 ${renderedArticles.length} 篇匹配文章${hasNextPage ? '，继续滚动加载' : ''}`
                              : `本周新增 ${selectedJournal.new_article_count} 篇文章`
                            : '从左侧选择期刊后查看本周新收录文章'}
                        </p>
                      </div>
                      {selectedJournal && (
                        <Badge variant="secondary" className="shrink-0">
                          {selectedJournal.new_article_count} 篇
                        </Badge>
                      )}
                    </div>
                  </MotionDiv>
                </MotionPresence>

                <MotionPresence mode="wait">
                  {articleState === 'no-journal' ? (
                    <MotionDiv
                      key="weekly-articles-no-journal"
                      data-weekly-article-state="no-journal"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      <StateMessage
                        isLive={false}
                        title="请选择一个期刊以查看新收录文章。"
                        description="选择后会显示该期刊在当前周窗口内的新文章。"
                      />
                    </MotionDiv>
                  ) : articleState === 'loading' ? (
                    <MotionDiv
                      key="weekly-articles-loading"
                      data-weekly-article-key={articleListKey}
                      data-weekly-article-state="loading"
                      className="space-y-2"
                      aria-hidden="true"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      <Skeleton className="h-28 w-full" />
                      <Skeleton className="h-28 w-full" />
                    </MotionDiv>
                  ) : articleState === 'error' ? (
                    <MotionDiv
                      key="weekly-articles-error"
                      data-weekly-article-key={articleListKey}
                      data-weekly-article-state="error"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      <StateMessage
                        isLive={false}
                        tone="danger"
                        title="加载本周文章失败"
                        description={
                          articleErrorData instanceof Error
                            ? articleErrorData.message
                            : '请稍后重试。'
                        }
                      />
                    </MotionDiv>
                  ) : articleState === 'empty' ? (
                    <MotionDiv
                      key="weekly-articles-empty"
                      data-weekly-article-key={articleListKey}
                      data-weekly-article-state="empty"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      <StateMessage
                        isLive={false}
                        title={searchQuery ? '没有匹配文章' : '该期刊暂无文章'}
                        description={
                          searchQuery ? '请尝试调整全文检索词。' : '当前周窗口内没有新收录文章。'
                        }
                      />
                    </MotionDiv>
                  ) : (
                    <MotionDiv
                      key="weekly-articles-results"
                      data-weekly-article-key={articleListKey}
                      data-weekly-article-state="results"
                      className="space-y-3"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
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

                      <MotionPresence>
                        {isFetchingNextPage && (
                          <MotionDiv
                            key="weekly-next-page"
                            aria-hidden="true"
                            className="py-2 text-center text-sm text-muted-foreground"
                            variants={FADE_UP_VARIANTS}
                            initial="hidden"
                            animate="visible"
                            exit={{ opacity: 0, pointerEvents: 'none', y: -2 }}
                            transition={stateTransition}
                          >
                            正在加载更多文章…
                          </MotionDiv>
                        )}
                      </MotionPresence>

                      {(visiblePageCount < articlePages.length || hasNextPage) && (
                        <div ref={loadMoreRef} className="h-1" />
                      )}
                    </MotionDiv>
                  )}
                </MotionPresence>
              </section>
            </MotionDiv>
          )
        )}
      </MotionPresence>
    </WorkspaceShell>
  );
}
