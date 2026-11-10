/**
 * Weekly article updates rendered inside the shared article workspace.
 */

import { useMemo } from 'react';
import { useInfiniteQuery, useQuery, type InfiniteData } from '@tanstack/react-query';
import { FileText, Layers } from '@/components/ui/animated-icons';
import { useSearch } from '@tanstack/react-router';

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
import { useUpdateWorkspaceSearch, WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { ArticleListSkeleton } from '@/components/feature/article-list-card';
import { DatabasePill, filterPillStyles } from '@/components/feature/filter-pill';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { SearchBar } from '@/components/feature/search-bar';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import {
  LIST_ITEM_VARIANTS,
  MotionDiv,
  MotionPresence,
  PANEL_VARIANTS,
  REVEAL_VARIANTS,
} from '@/components/ui/motion';
import { RevealItem } from '@/components/ui/reveal';
import { attachHorizontalWheelScroll } from '@/lib/horizontal-wheel-scroll';
import { Skeleton } from '@/components/ui/skeleton';
import { StateMessage } from '@/components/ui/state-message';
import { Button } from '@/components/ui/button';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

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

type WeeklyJournalFacetProps = {
  journals: WeeklyJournalSummary[];
  effectiveSelectedJournalId: JournalId | null;
  onSelectJournal: (journalId: JournalId) => void;
};

/**
 * Render the single-select journal picker for the current weekly database.
 *
 * @param props - Weekly journals, current selection, and selection action.
 * @returns Journal picker pill.
 */
function WeeklyJournalFacet({
  journals,
  effectiveSelectedJournalId,
  onSelectJournal,
}: WeeklyJournalFacetProps) {
  const selectedJournal =
    journals.find((journal) => journal.journal_id === effectiveSelectedJournalId) ?? null;
  const accessibleName = selectedJournal ? `期刊：${getJournalLabel(selectedJournal)}` : '期刊';

  return (
    <Select
      value={selectedJournal ? String(selectedJournal.journal_id) : ''}
      items={journals.map((journal) => ({
        value: String(journal.journal_id),
        label: getJournalLabel(journal),
      }))}
      onValueChange={(value) => {
        const journal = journals.find((candidate) => String(candidate.journal_id) === value);
        if (journal) onSelectJournal(journal.journal_id);
      }}
      disabled={journals.length === 0}
    >
      <SelectTrigger
        aria-label={accessibleName}
        title={accessibleName}
        size="sm"
        xstyle={[
          filterPillStyles(false),
          styles.weeklyJournalTrigger,
          selectedJournal && styles.weeklyJournalTriggerSelected,
        ]}
      >
        <SelectValue placeholder="期刊" />
      </SelectTrigger>
      <SelectContent
        align="start"
        aria-label={`本周有 ${journals.length} 本期刊更新`}
        xstyle={styles.weeklyJournalFacetSelectContent}
        listXstyle={styles.weeklyJournalViewport}
      >
        {journals.map((journal) => (
          <SelectItem
            key={journal.journal_id}
            value={String(journal.journal_id)}
            detail={journal.new_article_count}
          >
            {getJournalLabel(journal)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/**
 * Render weekly database and journal updates inside the shared reading workspace.
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
    selectDatabase,
    setSelectedJournalId,
    totalDatabases,
    totalArticles,
    weeklyState,
    articleState,
  } = state;
  const announcement = getWeeklyAnnouncement(state);
  const announcementRole =
    weeklyState === 'error' || (weeklyState === 'ready' && articleState === 'error')
      ? 'alert'
      : 'status';
  const handleDatabaseChange = (value: string) => {
    selectDatabase(value);
  };

  return (
    <WorkspaceShell
      listLabel="本周文章"
      emphasis="list"
      shouldAutoSelectFirst
      header={
        <div>
          <div {...stylex.props(styles.weeklyUpdatesViewDivItem, styles.weeklyUpdatesViewDiv)}>
            <div {...stylex.props(styles.weeklyUpdatesViewDiv2)}>
              <h1 {...stylex.props(styles.weeklyUpdatesViewH1)}>期刊每周更新</h1>
              <RevealItem
                isReady={weeklyState !== 'loading'}
                placeholder={<p {...stylex.props(styles.weeklyUpdatesViewP)}>正在读取统计窗口…</p>}
              >
                <p {...stylex.props(styles.weeklyUpdatesViewP2)}>
                  {weeklySummary
                    ? `统计窗口 ${formatDate(weeklySummary.window_start)} – ${formatDate(weeklySummary.window_end)}`
                    : '正在读取统计窗口…'}
                </p>
              </RevealItem>
            </div>
            <RevealItem
              isReady={weeklyState !== 'loading'}
              xstyle={styles.weeklyUpdatesViewRevealItem}
              placeholder={<Skeleton xstyle={styles.weeklyUpdatesViewSkeleton} />}
            >
              {weeklySummary && (
                <div
                  data-weekly-summary-key={weeklySummary.window_end}
                  {...stylex.props(styles.weeklyUpdatesViewDiv3)}
                >
                  <span {...stylex.props(styles.weeklyUpdatesViewSpan)}>
                    <Layers xstyle={styles.weeklyUpdatesViewLayers} aria-hidden="true" />
                    {totalDatabases} 个数据库
                  </span>
                  <span {...stylex.props(styles.weeklyUpdatesViewSpan2)}>
                    <FileText xstyle={styles.weeklyUpdatesViewLayers} aria-hidden="true" />
                    {totalArticles} 篇新文章
                  </span>
                </div>
              )}
            </RevealItem>
          </div>
          <SearchBar
            xstyle={styles.weeklyUpdatesViewDivItem}
            queryParam="weekly_q"
            placeholder="在本周新文章中检索…"
          />
          <div
            ref={attachHorizontalWheelScroll}
            {...stylex.props(styles.weeklyUpdatesViewDivItem, styles.weeklyUpdatesViewDiv4)}
          >
            <RevealItem
              isReady={weeklyState !== 'loading'}
              xstyle={styles.weeklyUpdatesViewRevealItem}
              placeholder={<Skeleton xstyle={styles.weeklyUpdatesViewSkeleton2} />}
            >
              {availableDatabases.length > 0 && (
                <DatabasePill
                  ariaLabel="周报数据库"
                  databases={availableDatabases}
                  value={effectiveSelectedDb}
                  onChange={handleDatabaseChange}
                />
              )}
            </RevealItem>
            <RevealItem
              isReady={weeklyState !== 'loading'}
              xstyle={styles.weeklyUpdatesViewRevealItem}
              placeholder={<Skeleton xstyle={styles.weeklyUpdatesViewSkeleton3} />}
            >
              <WeeklyJournalFacet
                journals={journals}
                effectiveSelectedJournalId={effectiveSelectedJournalId}
                onSelectJournal={(journalId) => setSelectedJournalId(journalId)}
              />
            </RevealItem>
          </div>
        </div>
      }
      readerPlaceholder={
        <ReaderPlaceholder
          title="本周新文章"
          description="从左侧列表选择一篇文献查看摘要；切换期刊胶囊浏览其他期刊的更新。"
        />
      }
    >
      <p
        key={`${weeklyState}-${articleState}-${announcement}`}
        data-testid="weekly-state-announcement"
        {...stylex.props(styles.weeklyUpdatesViewP3)}
        role={announcementRole}
        aria-label={announcement}
        aria-live={announcementRole === 'alert' ? 'assertive' : 'polite'}
        aria-atomic="true"
      >
        {announcement}
      </p>
      {favoriteStateError && (
        <div role="alert" {...stylex.props(styles.weeklyUpdatesViewDiv5)}>
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
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const updateSearch = useUpdateWorkspaceSearch();
  const searchQuery = (search.weekly_q ?? '').trim();
  const selectedDb = search.db ?? '';
  const selectedJournalId = search.journal ?? null;

  /**
   * Select a weekly database and clear the journal chosen in the previous database.
   *
   * @param dbName - Database to show, or an empty string for the default.
   */
  const selectDatabase = (dbName: string) => {
    void updateSearch((current) => ({ ...current, db: dbName || undefined, journal: undefined }));
  };

  /**
   * Select a weekly journal in the URL, or show every journal.
   *
   * @param journalId - Journal to show, or null for all journals.
   */
  const setSelectedJournalId = (journalId: string | null) => {
    void updateSearch((current) => ({ ...current, journal: journalId ?? undefined }));
  };

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
    queryFn: ({ pageParam, signal }) => {
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
        signal,
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
  const renderedArticleIds = useMemo(
    () => renderedArticles.map((article) => article.article_id),
    [renderedArticles],
  );
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
    selectDatabase,
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
  if (!selectedJournal) return '选择期刊后查看本周新收录文章';
  if (!searchQuery) return `本周新增 ${selectedJournal.new_article_count} 篇文章`;
  if (articleState === 'loading') return '正在检索本周文章…';
  if (articleState === 'error') return '全文检索失败';
  return `已加载 ${renderedArticles.length} 篇匹配文章${hasNextPage ? '，继续滚动加载' : ''}`;
}
/** Render visible cards and existing pagination sentinels without changing their ownership. */
function renderWeeklyArticleResults(state: WeeklyViewState) {
  const {
    user,
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
      <MotionPresence>
        {renderedArticles.map((article, index) => (
          <MotionDiv
            key={article.article_id}
            variants={LIST_ITEM_VARIANTS}
            initial="hidden"
            animate="visible"
          >
            <ArticleDialogCard
              triggerRef={index === prefetchIndex ? prefetchRef : undefined}
              article={article}
              dbName={effectiveSelectedDb}
              initialFolderIds={
                favoriteChecksByArticle[article.article_id]?.map((item) => item.folder_id) ?? []
              }
              isFavoriteStatePending={
                Boolean(user) &&
                isFavoriteStatePending &&
                !favoriteChecksByArticle[article.article_id]
              }
              isFavoriteStateUnavailable={
                Boolean(favoriteStateError) && !favoriteChecksByArticle[article.article_id]
              }
            />
          </MotionDiv>
        ))}
      </MotionPresence>
      <MotionPresence>
        {isFetchingNextPage && (
          <MotionDiv
            key="weekly-next-page"
            aria-hidden="true"
            {...stylex.props(styles.renderWeeklyArticleResultsMotionDiv)}
            variants={REVEAL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            <LoadingText text="正在加载更多文章…" />
          </MotionDiv>
        )}
      </MotionPresence>
      {(visiblePageCount < articlePages.length || hasNextPage) && (
        <div ref={loadMoreRef} {...stylex.props(styles.renderWeeklyArticleResultsDiv)} />
      )}
    </>
  );
}

/** Render the complete keyed article presence subtree with its original state precedence. */
function renderWeeklyArticleStates(state: WeeklyViewState) {
  return (
    <RevealItem
      isReady={state.articleState !== 'loading'}
      placeholder={
        <div
          data-weekly-article-key={state.articleListKey}
          data-weekly-article-state="loading"
          aria-hidden="true"
        >
          <ArticleListSkeleton count={4} />
        </div>
      }
    >
      <MotionPresence mode="wait">{renderWeeklyArticlePanel(state)}</MotionPresence>
    </RevealItem>
  );
}

/** Select the original keyed panel before returning it directly to MotionPresence. */
function renderWeeklyArticlePanel(state: WeeklyViewState) {
  const { searchQuery, articleErrorData, articleListKey, articleState } = state;
  if (articleState === 'no-journal')
    return (
      <MotionDiv
        key="weekly-articles-no-journal"
        data-weekly-article-state="no-journal"
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        <StateMessage
          isLive={false}
          title="请选择一个期刊以查看新收录文章。"
          description="选择后会显示该期刊在当前周窗口内的新文章。"
        />
      </MotionDiv>
    );
  if (articleState === 'loading') return null;
  if (articleState === 'error')
    return (
      <MotionDiv
        key="weekly-articles-error"
        data-weekly-article-key={articleListKey}
        data-weekly-article-state="error"
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
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
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
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
      {...stylex.props(styles.weeklyUpdatesViewDiv2)}
      variants={PANEL_VARIANTS}
      initial="hidden"
      animate="visible"
      exit="exit"
    >
      {renderWeeklyArticleResults(state)}
    </MotionDiv>
  );
}

/** Render the selected journal caption above the article rows. */
function renderWeeklyJournalHeader(state: WeeklyViewState) {
  const { effectiveSelectedDb, effectiveSelectedJournalId, selectedJournal } = state;
  return (
    <MotionPresence mode="wait">
      <MotionDiv
        key={`${effectiveSelectedDb}:${effectiveSelectedJournalId ?? 'none'}`}
        data-weekly-journal={effectiveSelectedJournalId ?? 'none'}
        {...stylex.props(styles.renderWeeklyJournalHeaderMotionDiv)}
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        <div {...stylex.props(styles.weeklyUpdatesViewDiv2)}>
          <h2 {...stylex.props(styles.renderWeeklyJournalHeaderH2)}>
            {selectedJournal ? getJournalLabel(selectedJournal) : '选择期刊'}
          </h2>
          <p {...stylex.props(styles.renderWeeklyJournalHeaderP)}>
            {getWeeklyJournalDescription(state)}
          </p>
        </div>
        {selectedJournal && (
          <span {...stylex.props(styles.renderWeeklyJournalHeaderSpan)}>
            {selectedJournal.new_article_count} 篇
          </span>
        )}
      </MotionDiv>
    </MotionPresence>
  );
}

/** Render summary loading and errors before the complete ready workspace. */
function renderWeeklyPresentation(state: WeeklyViewState) {
  const { weeklySummary, weeklyErrorData, weeklyState } = state;
  return (
    <RevealItem
      isReady={weeklyState !== 'loading'}
      placeholder={
        <div data-weekly-state="loading" aria-hidden="true">
          <Skeleton xstyle={styles.renderWeeklyPresentationSkeleton} />
          <ArticleListSkeleton count={4} />
        </div>
      }
    >
      <MotionPresence mode="wait">
        {weeklyState === 'error' ? (
          <MotionDiv
            key="weekly-error"
            data-weekly-state="error"
            variants={PANEL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
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
              variants={PANEL_VARIANTS}
              initial="hidden"
              animate="visible"
              exit="exit"
            >
              <section {...stylex.props(styles.weeklyUpdatesViewDiv2)} aria-label="每周文章">
                {renderWeeklyJournalHeader(state)}

                {renderWeeklyArticleStates(state)}
              </section>
            </MotionDiv>
          )
        )}
      </MotionPresence>
    </RevealItem>
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

const styles = stylex.create({
  weeklyJournalViewport: {
    paddingRight: '0.75rem',
  },
  weeklyJournalTrigger: {
    height: {
      default: '2.75rem',
      ':is([data-size="default"])': '2.25rem',
      '@media (min-width: 48rem)': {
        default: '2rem',
        ':is([data-size="default"])': '2.25rem',
      },
    },
    boxShadow: {
      default: `inset 0 0 0 1px ${colors.border}`,
      ':focus-visible': {
        default: `0 0 0 2px ${colors.ring}, inset 0 0 0 1px ${colors.border}`,
        ':is([aria-invalid="true"])': `0 0 0 2px ${colors.destructiveRing}, inset 0 0 0 1px ${colors.border}`,
      },
    },
    color: {
      default: `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
      ':is([data-placeholder])': colors.mutedForeground,
    },
    backgroundColor: {
      default: colors.background,
      ':active': {
        default: colors.accentPressed,
        ':is([data-popup-open])': colors.accent,
        ':hover': {
          default: null,
          '@media (hover: hover)': {
            default: colors.accentPressed,
            ':is([data-popup-open])': colors.accent,
          },
        },
      },
      ':is([data-popup-open])': colors.accent,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    lineHeight: 'inherit',
  },
  weeklyJournalTriggerSelected: {
    fontWeight: 700,
    color: {
      default: colors.foreground,
      ':is([data-placeholder])': colors.mutedForeground,
    },
  },
  weeklyJournalFacetSelectContent: {
    maxHeight: 'min(60vh, 24rem)',
    width: '24rem',
    maxWidth: 'calc(100vw - 2rem)',
  },
  weeklyUpdatesViewDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  weeklyUpdatesViewDiv: {
    display: 'flex',
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    gap: '0.75rem',
  },
  weeklyUpdatesViewDiv2: {
    minWidth: '0px',
  },
  weeklyUpdatesViewH1: {
    fontSize: '1.125rem',
    lineHeight: '1.75rem',
    fontWeight: 700,
  },
  weeklyUpdatesViewP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  weeklyUpdatesViewP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  weeklyUpdatesViewRevealItem: {
    flexShrink: 0,
  },
  weeklyUpdatesViewSkeleton: {
    height: '2rem',
    width: '5rem',
  },
  weeklyUpdatesViewDiv3: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'flex-end',
    gap: '0.125rem',
    fontSize: '11px',
    lineHeight: '1rem',
    color: colors.mutedForeground,
  },
  weeklyUpdatesViewSpan: {
    display: 'inline-flex',
    alignItems: 'center',
    gap: '0.25rem',
  },
  weeklyUpdatesViewLayers: {
    width: '0.75rem',
    height: '0.75rem',
  },
  weeklyUpdatesViewSpan2: {
    display: 'inline-flex',
    alignItems: 'center',
    gap: '0.25rem',
    fontWeight: 700,
    color: colors.foreground,
  },
  weeklyUpdatesViewDiv4: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
    },
    marginBlock: '-0.25rem',
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    overflowX: 'auto',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.25rem',
    scrollbarWidth: 'none',
    '::-webkit-scrollbar': {
      display: 'none',
    },
  },
  weeklyUpdatesViewSkeleton2: {
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    width: '10rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  weeklyUpdatesViewSkeleton3: {
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    width: '8rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  weeklyUpdatesViewP3: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  weeklyUpdatesViewDiv5: {
    margin: '1rem',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.destructive} 40%, transparent)`,
    backgroundColor: `color-mix(in oklab, ${colors.destructive} 5%, transparent)`,
    padding: '0.75rem',
    fontSize: '13px',
    marginInline: {
      default: null,
      '@media (min-width: 40rem)': '1.25rem',
    },
  },
  renderWeeklyArticleResultsMotionDiv: {
    paddingBlock: '1rem',
    textAlign: 'center',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderWeeklyArticleResultsDiv: {
    height: '0.25rem',
  },
  renderWeeklyJournalHeaderMotionDiv: {
    display: 'flex',
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    gap: '0.75rem',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 60%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.625rem',
  },
  renderWeeklyJournalHeaderH2: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '13px',
    fontWeight: 700,
  },
  renderWeeklyJournalHeaderP: {
    fontSize: '11px',
    color: colors.mutedForeground,
  },
  renderWeeklyJournalHeaderSpan: {
    flexShrink: 0,
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.info,
    paddingInline: '0.5rem',
    paddingBlock: '0.125rem',
    fontSize: '11px',
    fontWeight: 700,
    color: colors.infoForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderWeeklyPresentationSkeleton: {
    margin: '1rem',
    height: '2.5rem',
    marginInline: {
      default: null,
      '@media (min-width: 40rem)': '1.25rem',
    },
  },
});
