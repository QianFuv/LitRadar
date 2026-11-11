import { useInfiniteQuery, useQueryClient, type InfiniteData } from '@tanstack/react-query';
import { useLocation, useSearch } from '@tanstack/react-router';
import {
  articlePagesQueryKey,
  getArticles,
  type ArticlePage,
  type ArticleSearchQuery,
} from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { ArticleListSkeleton } from '@/components/feature/article-list-card';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import {
  LIST_ITEM_VARIANTS,
  MotionDiv,
  MotionPresence,
  PANEL_VARIANTS,
  REVEAL_VARIANTS,
} from '@/components/ui/motion';
import { RevealItem } from '@/components/ui/reveal';
import { Skeleton } from '@/components/ui/skeleton';
import { StateMessage } from '@/components/ui/state-message';
import { Button } from '@/components/ui/button';
import { useMemo, type ComponentProps, type ReactNode } from 'react';
import { useAuth } from '@/lib/auth-context';
import { getMonthRangeDateBounds } from '@/lib/article-filters';
import { useFtsHighlighter } from '@/lib/fts-highlight';
import { useSelectedDatabase } from '@/lib/selected-database';
import { useFavoriteChecks } from '@/components/feature/use-favorite-checks';
import {
  JOURNAL_RATING_SYSTEMS,
  readJournalRatings,
  type JournalRatingKey,
} from '@/lib/journal-ratings';
import { WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { describeSearchError } from '@/lib/search-syntax';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

type HighlightedArticleCardProps = Omit<
  ComponentProps<typeof ArticleDialogCard>,
  'title' | 'preview'
> & {
  highlightText: (text: string | null | undefined) => ReactNode;
};

/** Retain each article's highlights while later pages and favorite state change. */
function HighlightedArticleCard({ highlightText, ...props }: HighlightedArticleCardProps) {
  const { title, abstract } = props.article;
  const highlighted = useMemo(
    () => ({ title: highlightText(title), preview: highlightText(abstract) }),
    [title, abstract, highlightText],
  );
  return <ArticleDialogCard {...props} title={highlighted.title} preview={highlighted.preview} />;
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
 * @returns Article rows and pagination sentinels.
 */
export function ResultsList() {
  const state = useResultsViewState();
  const { favoriteStateError, retryFavoriteChecks } = state;
  const { resultContent, resultAnnouncement, resultAnnouncementRole, resultState } =
    getResultsPresentation(state);
  return (
    <div>
      <p
        key={`${resultState}-${resultAnnouncement}`}
        data-testid="results-state-announcement"
        {...stylex.props(styles.resultsListP)}
        role={resultAnnouncementRole}
        aria-label={resultAnnouncement}
        aria-live={resultAnnouncementRole === 'alert' ? 'assertive' : 'polite'}
        aria-atomic="true"
      >
        {resultAnnouncement}
      </p>
      {favoriteStateError && (
        <div role="alert" {...stylex.props(styles.resultsListDiv)}>
          <span>收藏状态暂时不可用，未能确认文章的收藏状态。</span>
          <Button type="button" variant="outline" size="sm" onClick={retryFavoriteChecks}>
            重试收藏状态
          </Button>
        </div>
      )}
      <RevealItem
        isReady={resultState !== 'loading'}
        xstyle={styles.resultsListRevealItem}
        placeholder={
          <div data-testid="results-state-loading">
            <ArticleListSkeleton />
          </div>
        }
      >
        <MotionPresence mode="wait">
          <MotionDiv
            key={resultState}
            data-testid={`results-state-${resultState}`}
            {...stylex.props(styles.resultsListRevealItem)}
            variants={{
              ...PANEL_VARIANTS,
              visible: { ...PANEL_VARIANTS.visible, pointerEvents: 'inherit' },
            }}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            {resultContent}
          </MotionDiv>
        </MotionPresence>
      </RevealItem>
    </div>
  );
}
/** Own the unchanged query, visibility, favorite and highlight hooks together. */
function useResultsViewState() {
  const { user } = useAuth();
  const queryClient = useQueryClient();

  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const q = search.q ?? null;
  const areas = search.area ?? null;
  const journalIds = search.journal_id ?? null;
  const monthRange = search.month_range ?? null;
  const ratings = readJournalRatings(search);
  const searchKey = useLocation({ select: (location) => location.searchStr });

  const articleQuery = getResultsArticleQuery(q, areas, journalIds, ratings, monthRange);
  const currentDb = useSelectedDatabase();
  const queryKey = articlePagesQueryKey(articleQuery, currentDb);

  const { data, isLoading, isError, error, fetchNextPage, hasNextPage, isFetchingNextPage } =
    useInfiniteQuery<
      ArticlePage,
      Error,
      InfiniteData<ArticlePage, string | null>,
      typeof queryKey,
      string | null
    >({
      queryKey,
      queryFn: async ({ pageParam, signal }) => {
        const page = await getArticles(articleQuery, pageParam, currentDb, signal);
        const cachedData =
          queryClient.getQueryData<InfiniteData<ArticlePage, string | null>>(queryKey);
        return validateArticlePageCursor(page, pageParam, cachedData?.pageParams ?? []);
      },
      initialPageParam: null,
      getNextPageParam: getNextArticlePageParam,
      staleTime: 5 * 60 * 1000,
      gcTime: 10 * 60 * 1000,
    });

  const pages = useMemo(() => data?.pages ?? [], [data?.pages]);
  const loadedPages = pages.length;
  const { visiblePages, prefetchRef, loadMoreRef } = useVisiblePageList({
    listKey: `${currentDb}:${searchKey}`,
    loadedPages,
    hasNextPage,
    isFetchingNextPage,
    onFetchNextPage: () => void fetchNextPage(),
    scrollContainerId: 'results-scroll-container',
  });
  const visiblePageCount = Math.min(visiblePages, loadedPages);
  const visibleArticles = useMemo(
    () => pages.slice(0, visiblePageCount).flatMap((page) => page.items),
    [pages, visiblePageCount],
  );
  const visibleArticleIds = useMemo(
    () => visibleArticles.map((article) => article.article_id),
    [visibleArticles],
  );
  const {
    favoriteChecksByArticle,
    isFavoriteStatePending,
    favoriteStateError,
    retryFavoriteChecks,
  } = useFavoriteChecks(visibleArticleIds, currentDb, user?.id);

  const highlightText = useFtsHighlighter(q);

  const prefetchThreshold = 25;
  const prefetchIndex = Math.max(0, visibleArticles.length - prefetchThreshold);

  return {
    user,
    currentDb,
    isLoading,
    isError,
    error,
    isFetchingNextPage,
    prefetchRef,
    loadMoreRef,
    visibleArticles,
    favoriteChecksByArticle,
    isFavoriteStatePending,
    favoriteStateError,
    retryFavoriteChecks,
    highlightText,
    prefetchIndex,
  };
}
type ResultsViewState = ReturnType<typeof useResultsViewState>;
/** Preserve error, loading, empty and result presentation and the single announcement. */
function getResultsPresentation(state: ResultsViewState) {
  const {
    user,
    currentDb,
    isLoading,
    isError,
    error,
    isFetchingNextPage,
    prefetchRef,
    loadMoreRef,
    visibleArticles,
    favoriteChecksByArticle,
    isFavoriteStatePending,
    favoriteStateError,
    highlightText,
    prefetchIndex,
  } = state;
  let resultContent: ReactNode;
  let resultAnnouncement: string;
  let resultAnnouncementRole: 'alert' | 'status' = 'status';
  let resultState: 'empty' | 'error' | 'loading' | 'results';

  if (isError) {
    resultState = 'error';
    resultAnnouncementRole = 'alert';
    resultAnnouncement = `无法加载文章：${error instanceof Error ? describeSearchError(error.message) : '未知错误'}`;
    resultContent = (
      <StateMessage
        isLive={false}
        tone="danger"
        title="无法加载文章"
        description={error instanceof Error ? describeSearchError(error.message) : '未知错误'}
      />
    );
  } else if (isLoading) {
    resultState = 'loading';
    resultAnnouncement = '正在加载搜索结果';
    resultContent = null;
  } else if (visibleArticles.length === 0) {
    resultState = 'empty';
    resultAnnouncement = '未找到文章。请尝试调整搜索词、数据库或筛选条件。';
    resultContent = (
      <StateMessage
        isLive={false}
        title="未找到文章。"
        description="请尝试调整搜索词、数据库或筛选条件。"
      />
    );
  } else {
    resultState = 'results';
    resultAnnouncement = isFetchingNextPage
      ? '正在加载更多文章'
      : `已加载 ${visibleArticles.length} 篇文章`;
    resultContent = (
      <>
        <MotionPresence>
          {visibleArticles.map((article, index) => (
            <MotionDiv
              key={article.article_id}
              variants={LIST_ITEM_VARIANTS}
              initial="hidden"
              animate="visible"
            >
              <HighlightedArticleCard
                triggerRef={index === prefetchIndex ? prefetchRef : undefined}
                article={article}
                dbName={currentDb}
                highlightText={highlightText}
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

        <div ref={loadMoreRef} {...stylex.props(styles.getResultsPresentationDiv)} />
        <MotionPresence>
          {isFetchingNextPage && (
            <MotionDiv
              key="next-page-loading"
              aria-hidden="true"
              {...stylex.props(styles.getResultsPresentationMotionDiv)}
              variants={REVEAL_VARIANTS}
              initial="hidden"
              animate="visible"
              exit="exit"
            >
              <Skeleton
                xstyle={[
                  styles.getResultsPresentationMotionDivItem,
                  styles.getResultsPresentationSkeleton,
                ]}
              />
              <Skeleton
                xstyle={[
                  styles.getResultsPresentationMotionDivItem,
                  styles.getResultsPresentationSkeleton2,
                ]}
              />
            </MotionDiv>
          )}
        </MotionPresence>
      </>
    );
  }

  return { resultContent, resultAnnouncement, resultAnnouncementRole, resultState };
}

/**
 * Build the article search filters for the workspace's query text and selected filters.
 *
 * @param q - Raw search text, or null.
 * @param areas - Selected subject areas, or null.
 * @param journalIds - Selected journal ids, or null.
 * @param ratings - Selected values per journal rating system.
 * @param monthRange - Selected month range, or null.
 * @returns Article search filters; empty selections are left out of the request.
 */
function getResultsArticleQuery(
  q: string | null,
  areas: string[] | null,
  journalIds: string[] | null,
  ratings: Record<JournalRatingKey, string[]>,
  monthRange: string | null,
): ArticleSearchQuery {
  const dateBounds = getMonthRangeDateBounds(monthRange);
  const ratingFilters: Partial<Record<JournalRatingKey, string[]>> = {};
  for (const { key } of JOURNAL_RATING_SYSTEMS) {
    if (ratings[key].length > 0) {
      ratingFilters[key] = ratings[key];
    }
  }
  return {
    ...(q ? { q } : {}),
    ...(areas && areas.length > 0 ? { area: areas } : {}),
    ...(journalIds && journalIds.length > 0 ? { journal_id: journalIds } : {}),
    ...ratingFilters,
    ...(dateBounds ? { date_from: dateBounds.dateFrom, date_to: dateBounds.dateTo } : {}),
  };
}

const styles = stylex.create({
  resultsListP: {
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
  resultsListDiv: {
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
  resultsListRevealItem: {
    minWidth: '0px',
  },
  getResultsPresentationDiv: {
    height: '0.25rem',
  },
  getResultsPresentationMotionDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  getResultsPresentationMotionDiv: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
  },
  getResultsPresentationSkeleton: {
    height: '1rem',
    width: 'calc(11 / 12 * 100%)',
  },
  getResultsPresentationSkeleton2: {
    height: '0.75rem',
    width: 'calc(1 / 2 * 100%)',
  },
});
