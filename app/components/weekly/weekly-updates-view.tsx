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
  type WeeklyUpdatesSummaryResponse,
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
import { Button } from '@/components/ui/button';
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
        <div className="space-y-4 border-t border-sidebar-border pt-3">
          <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3">
            <div className="flex items-center gap-2 text-sm font-semibold text-sidebar-foreground">
              <Database className="size-4" aria-hidden="true" />
              <span>数据库</span>
            </div>
            <Select value={effectiveSelectedDb} onValueChange={onDatabaseChange}>
              <SelectTrigger aria-label="周报数据库" className="h-9 w-full bg-sidebar">
                <SelectValue placeholder="选择数据库" />
              </SelectTrigger>
              <SelectContent>
                {availableDatabases.map((dbName) => (
                  <SelectItem key={dbName} value={dbName}>
                    {dbName.replace(/\.sqlite$/, '')}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <h2 className="text-sm font-semibold text-sidebar-foreground">期刊</h2>
        </div>
      }
    >
      <div
        data-slot="sidebar-scroll-region"
        className="sidebar-scroll-gutter min-h-0 flex-1 space-y-1 overflow-y-auto overscroll-contain"
      >
        {journals.length === 0 && (
          <div className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
            当前时间窗口内没有新增期刊。
          </div>
        )}

        {journals.map((journal) => {
          const isActive = effectiveSelectedJournalId === journal.journal_id;
          return (
            <button
              key={journal.journal_id}
              type="button"
              aria-pressed={isActive}
              onClick={() => onSelectJournal(journal.journal_id)}
              className={cn(
                'motion-control min-h-11 w-full rounded-md border border-transparent px-2 py-2 text-left text-sidebar-foreground transition-colors hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-inset',
                isActive && 'border-sidebar-border bg-sidebar-accent',
              )}
            >
              <div className="flex items-center justify-between gap-2">
                <p className="line-clamp-2 min-w-0 break-words text-sm font-medium">
                  {getJournalLabel(journal)}
                </p>
                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                  {journal.new_article_count}
                </span>
              </div>
            </button>
          );
        })}
      </div>
    </WorkspaceSidebar>
  );
}

/**
 * Render weekly database and journal updates inside the shared article workspace.
 *
 * @returns Weekly-updates workspace view.
 */
export function WeeklyUpdatesView() {
  const state = useWeeklyViewState();
  const {
    weeklySummary,
    effectiveSelectedDb,
    effectiveSelectedJournalId,
    availableDatabases,
    journals,
    favoriteStateError,
    retryFavoriteChecks,
    setSelectedDb,
    setSelectedJournalId,
    weeklyState,
    articleState,
  } = state;
  const announcement = getWeeklyAnnouncement(state);
  const announcementRole =
    weeklyState === 'error' || (weeklyState === 'ready' && articleState === 'error')
      ? 'alert'
      : 'status';
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
      {favoriteStateError && (
        <div
          role="alert"
          className="flex items-center justify-between gap-3 rounded-md border border-destructive/50 p-3 text-sm"
        >
          <span>收藏状态暂时不可用，未能确认文章的收藏状态。</span>
          <Button type="button" variant="outline" size="sm" onClick={retryFavoriteChecks}>
            重试收藏状态
          </Button>
        </div>
      )}
      {renderWeeklyPresentation(state)}
    </WorkspaceShell>
  );
}

/** Own the unchanged query, URL, selection and pagination hooks in their original order. */
function useWeeklyViewState() {
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

  const weeklyArticleQueryKey = getWeeklyArticleQueryKey(
    effectiveSelectedDb,
    effectiveSelectedJournalId,
    searchQuery,
    weeklySummary,
  );
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
  const articleListKey = getWeeklyArticleListKey(
    effectiveSelectedDb,
    effectiveSelectedJournalId,
    searchQuery,
    weeklySummary,
  );
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
  const {
    favoriteChecksByArticle,
    isFavoriteStatePending,
    favoriteStateError,
    retryFavoriteChecks,
  } = useFavoriteChecks(renderedArticleIds, effectiveSelectedDb, user?.id);

  const totalDatabases = weeklySummary?.databases.length ?? 0;
  const totalArticles = useMemo(() => {
    if (!weeklySummary) {
      return 0;
    }
    return weeklySummary.databases.reduce((sum, db) => sum + db.new_article_count, 0);
  }, [weeklySummary]);

  const weeklyState = getWeeklyState(weeklyError, loadingWeekly, Boolean(weeklySummary));
  const articleState = getWeeklyArticleState(
    Boolean(selectedJournal),
    loadingArticles,
    articleError,
    renderedArticles.length,
  );
  return {
    user,
    searchQuery,
    stateTransition,
    weeklySummary,
    weeklyErrorData,
    articleErrorData,
    effectiveSelectedDb,
    effectiveSelectedJournalId,
    selectedJournal,
    availableDatabases,
    journals,
    renderedArticles,
    articleListKey,
    prefetchIndex,
    prefetchRef,
    loadMoreRef,
    favoriteChecksByArticle,
    isFavoriteStatePending,
    favoriteStateError,
    retryFavoriteChecks,
    totalDatabases,
    totalArticles,
    hasNextPage,
    isFetchingNextPage,
    visiblePageCount,
    articlePages,
    setSelectedDb,
    setSelectedJournalId,
    weeklyState,
    articleState,
  };
}

type WeeklyViewState = ReturnType<typeof useWeeklyViewState>;
type WeeklyState = 'error' | 'loading' | 'ready';
type WeeklyArticleState = 'no-journal' | 'loading' | 'error' | 'empty' | 'results';

/** Preserve weekly failure precedence over loading or missing summaries. */
function getWeeklyState(hasError: boolean, isLoading: boolean, hasSummary: boolean): WeeklyState {
  if (hasError) return 'error';
  if (isLoading || !hasSummary) return 'loading';
  return 'ready';
}

/** Preserve article selection, loading, failure and empty-result priority. */
function getWeeklyArticleState(
  hasJournal: boolean,
  isLoading: boolean,
  hasError: boolean,
  count: number,
): WeeklyArticleState {
  if (!hasJournal) return 'no-journal';
  if (isLoading) return 'loading';
  if (hasError) return 'error';
  if (count === 0) return 'empty';
  return 'results';
}

/** Format the one live announcement using the original ordered presentation states. */
function getWeeklyAnnouncement(state: WeeklyViewState): string {
  const { weeklyState, weeklyErrorData } = state;
  if (weeklyState === 'loading') return '正在加载每周更新摘要';
  if (weeklyState === 'error')
    return `加载每周更新失败：${weeklyErrorData instanceof Error ? weeklyErrorData.message : '未知错误'}`;
  return getWeeklyArticleAnnouncement(state);
}

/** Announce article progress without introducing a second live region. */
function getWeeklyArticleAnnouncement(state: WeeklyViewState): string {
  const {
    articleState,
    articleErrorData,
    selectedJournal,
    searchQuery,
    isFetchingNextPage,
    renderedArticles,
  } = state;
  if (articleState === 'no-journal') return '请选择一个期刊以查看新收录文章';
  if (articleState === 'loading')
    return `正在加载“${selectedJournal ? getJournalLabel(selectedJournal) : ''}”的本周文章`;
  if (articleState === 'error')
    return `加载本周文章失败：${articleErrorData instanceof Error ? articleErrorData.message : '未知错误'}`;
  if (articleState === 'empty')
    return searchQuery ? '该期刊中没有匹配全文检索条件的本周文章' : '该期刊暂无文章';
  if (isFetchingNextPage) return '正在加载更多本周文章';
  return `已加载 ${renderedArticles.length} 篇本周文章`;
}

/** Describe the selected journal with the original search loading and failure precedence. */
function getWeeklyJournalDescription(
  state: Pick<
    WeeklyViewState,
    'selectedJournal' | 'searchQuery' | 'articleState' | 'renderedArticles' | 'hasNextPage'
  >,
): string {
  const { selectedJournal, searchQuery, articleState, renderedArticles, hasNextPage } = state;
  if (!selectedJournal) return '从左侧选择期刊后查看本周新收录文章';
  if (!searchQuery) return `本周新增 ${selectedJournal.new_article_count} 篇文章`;
  if (articleState === 'loading') return '正在检索本周文章…';
  if (articleState === 'error') return '全文检索失败';
  return `已加载 ${renderedArticles.length} 篇匹配文章${hasNextPage ? '，继续滚动加载' : ''}`;
}
/** Render visible cards and existing pagination sentinels without changing their ownership. */
function renderWeeklyArticleResults(state: WeeklyViewState) {
  const {
    user,
    stateTransition,
    effectiveSelectedDb,
    renderedArticles,
    prefetchIndex,
    prefetchRef,
    loadMoreRef,
    favoriteChecksByArticle,
    isFavoriteStatePending,
    favoriteStateError,
    hasNextPage,
    isFetchingNextPage,
    visiblePageCount,
    articlePages,
  } = state;
  return (
    <>
      {renderedArticles.map((article, index) => (
        <ArticleDialogCard
          key={article.article_id}
          triggerRef={index === prefetchIndex ? prefetchRef : undefined}
          article={article}
          dbName={effectiveSelectedDb}
          initialFolderIds={
            favoriteChecksByArticle[article.article_id]?.map((item) => item.folder_id) ?? []
          }
          isFavoriteStatePending={
            Boolean(user) && isFavoriteStatePending && !favoriteChecksByArticle[article.article_id]
          }
          isFavoriteStateUnavailable={
            Boolean(favoriteStateError) && !favoriteChecksByArticle[article.article_id]
          }
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
    </>
  );
}

/** Render the complete keyed article presence subtree with its original state precedence. */
function renderWeeklyArticleStates(state: WeeklyViewState) {
  return <MotionPresence mode="wait">{renderWeeklyArticlePanel(state)}</MotionPresence>;
}

/** Select the original keyed panel before returning it directly to MotionPresence. */
function renderWeeklyArticlePanel(state: WeeklyViewState) {
  const { searchQuery, stateTransition, articleErrorData, articleListKey, articleState } = state;
  if (articleState === 'no-journal')
    return (
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
    );
  if (articleState === 'loading')
    return (
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
    );
  if (articleState === 'error')
    return (
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
            articleErrorData instanceof Error ? articleErrorData.message : '请稍后重试。'
          }
        />
      </MotionDiv>
    );
  if (articleState === 'empty')
    return (
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
          description={searchQuery ? '请尝试调整全文检索词。' : '当前周窗口内没有新收录文章。'}
        />
      </MotionDiv>
    );
  return (
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
      {renderWeeklyArticleResults(state)}
    </MotionDiv>
  );
}

/** Render the complete journal heading presence subtree with its existing identity key. */
function renderWeeklyJournalHeader(state: WeeklyViewState) {
  const { stateTransition, effectiveSelectedDb, effectiveSelectedJournalId, selectedJournal } =
    state;
  return (
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
              {getWeeklyJournalDescription(state)}
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
  );
}

/** Render the complete summary animation and unchanged search control. */
function renderWeeklySummary(
  state: Pick<WeeklyViewState, 'stateTransition' | 'totalDatabases' | 'totalArticles'>,
  weeklySummary: WeeklyUpdatesSummaryResponse,
) {
  const { stateTransition, totalDatabases, totalArticles } = state;
  return (
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
      <SearchBar className="w-full max-w-none sm:min-w-0 sm:flex-1" queryParam="weekly_q" />
    </section>
  );
}

/** Render summary loading and errors before the complete ready workspace. */
function renderWeeklyPresentation(state: WeeklyViewState) {
  const { stateTransition, weeklySummary, weeklyErrorData, weeklyState } = state;
  return (
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
            {renderWeeklySummary(state, weeklySummary)}

            <section className="min-w-0 space-y-3" aria-label="每周文章">
              {renderWeeklyJournalHeader(state)}

              {renderWeeklyArticleStates(state)}
            </section>
          </MotionDiv>
        )
      )}
    </MotionPresence>
  );
}

/** Build the unchanged database, journal, search and weekly-window query identity. */
function getWeeklyArticleQueryKey(
  database: string,
  journal: JournalId | null,
  query: string,
  summary: WeeklyUpdatesSummaryResponse | undefined,
): string[] {
  return ['weekly-update-articles', database, journal ?? '', query, summary?.window_end ?? ''];
}

/** Build the unchanged visible-page reset identity, including the no-journal marker. */
function getWeeklyArticleListKey(
  database: string,
  journal: JournalId | null,
  query: string,
  summary: WeeklyUpdatesSummaryResponse | undefined,
): string {
  return `${database}:${journal ?? 'none'}:${query}:${summary?.window_end ?? ''}`;
}
