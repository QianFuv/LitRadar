/** Layers and journal navigation over original notices evaluated by the backend. */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useDebouncer } from '@tanstack/react-pacer';
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { useSearch } from '@tanstack/react-router';
import { Layers, Megaphone, Search } from '@/components/ui/animated-icons';

import { CfpNoticeCard, CfpSourceFooter } from '@/components/cfp/cfp-notice-card';
import {
  CfpJournalList,
  groupCfpJournals,
  type CfpJournalGroup,
} from '@/components/cfp/cfp-journal-list';
import { DatabasePill } from '@/components/feature/filter-pill';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { WORKSPACE_SEARCH_ATTRIBUTE } from '@/components/feature/article-reader-context';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import { RevealItem } from '@/components/ui/reveal';
import { Skeleton } from '@/components/ui/skeleton';
import { LIST_ITEM_VARIANTS, MotionDiv, MotionPresence } from '@/components/ui/motion';
import { StateMessage } from '@/components/ui/state-message';
import {
  ApiError,
  getDatabases,
  getCfpJournals,
  getCfpNotices,
  type CfpJournalSummary,
  type CfpNoticePage,
} from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import { useUpdateWorkspaceSearch, WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

const DATABASE_LABELS = new Map(
  Object.entries({
    'ccf_computer_journals.sqlite': 'CCF 计算机期刊',
    'chinese_journals.sqlite': '中文期刊',
    'english_journals.sqlite': '英文期刊',
  }),
);
/**
 * Format a CFP database file name for display.
 *
 * @param name - Layers file name.
 * @returns Human-readable database label.
 */
function formatCfpDatabase(name: string): string {
  return DATABASE_LABELS.get(name) ?? name.replace(/\.sqlite$/, '');
}

/** Keep CFP catalog and notice responses fresh long enough to make revisits instant. */
const CFP_STALE_TIME_MS = 5 * 60 * 1000;
/** Wait briefly on hover so passing the pointer over the list does not prefetch every journal. */
const CFP_PREFETCH_INTENT_DELAY_MS = 150;

/** Compose the journal directory and the reading desk with independent CFP selections. */
export function CfpTrackingView() {
  const state = useCfpViewState();
  const [isReaderSheetOpen, setIsReaderSheetOpen] = useState(false);

  return (
    <WorkspaceShell
      listLabel="征稿期刊"
      isListSelfScrolling
      header={renderCfpHeader(state)}
      reader={{
        content: (
          <div {...stylex.props(styles.cfpTrackingViewDiv)}>
            <RevealItem
              isReady={!state.isLoading}
              xstyle={[styles.cfpTrackingViewDivItem, styles.cfpTrackingViewRevealItem]}
              placeholder={
                <div>
                  <output
                    {...stylex.props(styles.cfpTrackingViewDivItem2, styles.cfpTrackingViewSpan)}
                  >
                    正在加载征稿期刊
                  </output>
                  <Skeleton
                    xstyle={[styles.cfpTrackingViewDivItem2, styles.cfpTrackingViewSkeleton]}
                  />
                  <Skeleton
                    xstyle={[styles.cfpTrackingViewDivItem2, styles.cfpTrackingViewSkeleton2]}
                  />
                </div>
              }
            >
              {renderCfpWorkspace(state)}
            </RevealItem>
          </div>
        ),
        description: '期刊征稿需求与时间安排',
        isSheetOpen: isReaderSheetOpen,
        onSheetOpenChange: setIsReaderSheetOpen,
        title: state.selected?.title ?? '征稿详情',
      }}
      readerPlaceholder={null}
    >
      {renderCfpJournalPanel(state, () => setIsReaderSheetOpen(true))}
    </WorkspaceShell>
  );
}

/** Own unchanged CFP hooks, requests, cursor recovery and dependent reset order. */
function useCfpViewState() {
  const { user } = useAuth();
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const updateSearch = useUpdateWorkspaceSearch();
  const selectedDatabase = search.cfp_db ?? '';
  const selectedCatalogId = search.cfp_journal ?? '';

  /**
   * Select a CFP journal in the URL, or clear the selection.
   *
   * @param catalogId - Journal catalog id, or null to clear it.
   */
  const setSelectedCatalogId = (catalogId: string | null) => {
    void updateSearch((current) => ({ ...current, cfp_journal: catalogId || undefined }));
  };
  const [journalSearch, setJournalSearch] = useState('');
  const [shouldShowClosed, setShouldShowClosed] = useState(false);
  const databasesQuery = useQuery({
    queryKey: ['meta', 'databases'],
    queryFn: getDatabases,
    enabled: Boolean(user),
  });
  const databases = databasesQuery.data ?? [];
  const database = getCfpDatabase(databases, selectedDatabase);
  const queryClient = useQueryClient();
  const cursorRecovery = useRef<string | null>(null);
  const journalsQuery = useQuery({
    queryKey: ['cfp', 'journals', database],
    queryFn: ({ signal }) => getCfpJournals(database, signal),
    enabled: Boolean(user && database),
    staleTime: CFP_STALE_TIME_MS,
  });
  const journals = useMemo(() => journalsQuery.data?.items ?? [], [journalsQuery.data]);
  const journalGroups = useMemo(() => groupCfpJournals(journals), [journals]);
  const filteredGroups = useMemo(() => {
    const query = journalSearch.trim().toLocaleLowerCase();
    return journalGroups.map((group) => ({
      ...group,
      journals: group.journals.filter(
        (journal) =>
          !query ||
          [journal.title, ...journal.allIssns, ...journal.titleAliases].some((value) =>
            value.toLocaleLowerCase().includes(query),
          ),
      ),
    }));
  }, [journalSearch, journalGroups]);
  const selectedCatalog = getCfpCatalog(journals, journalGroups, selectedCatalogId);
  const noticeQueryKey = useMemo(
    () => ['cfp', 'notices', database, selectedCatalog?.catalogId ?? '', shouldShowClosed] as const,
    [database, selectedCatalog?.catalogId, shouldShowClosed],
  );
  const noticesQuery = useInfiniteQuery({
    queryKey: noticeQueryKey,
    queryFn: ({ pageParam, signal }) =>
      getCfpNotices(
        {
          dbName: database,
          catalogId: selectedCatalog!.catalogId,
          includeClosed: shouldShowClosed,
        },
        pageParam,
        signal,
      ),
    initialPageParam: null as string | null,
    getNextPageParam: (lastPage) => lastPage.page.next_cursor ?? undefined,
    enabled: Boolean(user && database && selectedCatalog),
    retry: false,
    staleTime: CFP_STALE_TIME_MS,
  });
  const journalPrefetcher = useDebouncer(
    (catalogId: string) => {
      if (!database) return;
      void queryClient.prefetchInfiniteQuery({
        queryKey: ['cfp', 'notices', database, catalogId, shouldShowClosed] as const,
        queryFn: ({ pageParam, signal }) =>
          getCfpNotices(
            { dbName: database, catalogId, includeClosed: shouldShowClosed },
            pageParam,
            signal,
          ),
        initialPageParam: null as string | null,
        staleTime: CFP_STALE_TIME_MS,
      });
    },
    { wait: CFP_PREFETCH_INTENT_DELAY_MS },
  );

  /** Prefetch the first notice page of a journal the user is about to open. */
  function handleJournalIntent(catalogId: string): void {
    if (!user || !database) {
      journalPrefetcher.cancel();
      return;
    }
    journalPrefetcher.maybeExecute(catalogId);
  }
  const cursorRecoveryKey = JSON.stringify(noticeQueryKey);
  useEffect(() => {
    if (
      noticesQuery.error instanceof ApiError &&
      noticesQuery.error.status === 409 &&
      cursorRecovery.current !== cursorRecoveryKey
    ) {
      cursorRecovery.current = cursorRecoveryKey;
      void queryClient.resetQueries({ queryKey: noticeQueryKey, exact: true });
    } else if (noticesQuery.isSuccess) {
      cursorRecovery.current = null;
    }
  }, [cursorRecoveryKey, noticeQueryKey, noticesQuery.error, noticesQuery.isSuccess, queryClient]);
  const selected = getSelectedCfpJournal(noticesQuery.data, selectedCatalog);
  const { data: noticeData, isError: isNoticeError } = noticesQuery;
  const visibleNotices = useMemo(
    () => getCfpVisibleNotices({ data: noticeData, isError: isNoticeError }),
    [noticeData, isNoticeError],
  );
  const currentCount = selected?.currentCount ?? 0;
  const isLoading = databasesQuery.isLoading || journalsQuery.isLoading;
  const error = databasesQuery.error ?? journalsQuery.error;

  /** Change CFP databases and reset the dependent journal selection in one URL update. */
  function handleDatabaseChange(value: string): void {
    void updateSearch((current) => ({
      ...current,
      cfp_db: value || undefined,
      cfp_journal: undefined,
    }));
    setJournalSearch('');
    setShouldShowClosed(false);
  }

  /** Retry only the metadata request that failed. */
  function handleRetry(): void {
    if (databasesQuery.isError) void databasesQuery.refetch();
    else void journalsQuery.refetch();
  }

  return {
    setSelectedCatalogId,
    journalSearch,
    setJournalSearch,
    shouldShowClosed,
    setShouldShowClosed,
    databasesQuery,
    databases,
    database,
    journals,
    filteredGroups,
    noticesQuery,
    selected,
    visibleNotices,
    currentCount,
    isLoading,
    error,
    handleDatabaseChange,
    handleJournalIntent,
    handleRetry,
  };
}
type CfpViewState = ReturnType<typeof useCfpViewState>;

/** Retain notice loading, records, verified source and empty presentation priority. */
function renderCfpNoticeContent(
  state: Pick<CfpViewState, 'noticesQuery' | 'visibleNotices'>,
  selected: NonNullable<CfpViewState['selected']>,
) {
  const { noticesQuery, visibleNotices } = state;
  if (noticesQuery.isLoading) return null;
  if (visibleNotices.length)
    return (
      <MotionPresence>
        {visibleNotices.map((notice) => (
          <MotionDiv
            key={notice.id}
            variants={LIST_ITEM_VARIANTS}
            initial="hidden"
            animate="visible"
          >
            <CfpNoticeCard notice={notice} />
          </MotionDiv>
        ))}
      </MotionPresence>
    );
  if (selected.sourceStatement)
    return (
      <div {...stylex.props(styles.renderCfpNoticeContentDiv)}>
        <div {...stylex.props(styles.renderCfpNoticeContentDiv2)}>
          <h3
            {...stylex.props(styles.renderCfpNoticeContentDivItem, styles.renderCfpNoticeContentH3)}
          >
            暂无已收录的征稿公告
          </h3>
          <p
            {...stylex.props(styles.renderCfpNoticeContentDivItem, styles.renderCfpNoticeContentP)}
          >
            {selected.sourceStatement}
          </p>
        </div>
        <CfpSourceFooter
          checkedOn={selected.checkedOn}
          sourceUrl={selected.sourceUrl}
          label="查看期刊说明"
        />
      </div>
    );
  return (
    <StateMessage
      title="暂无已收录的进行中征稿"
      description="可以查看已结束的信息，或选择其他期刊。"
    />
  );
}

/** Retain historical filtering, pagination and refresh-warning placement. */
function renderCfpNotices(
  state: Pick<
    CfpViewState,
    'shouldShowClosed' | 'setShouldShowClosed' | 'noticesQuery' | 'visibleNotices'
  >,
  selected: NonNullable<CfpViewState['selected']>,
) {
  const { shouldShowClosed, setShouldShowClosed, noticesQuery } = state;

  return (
    <section aria-label="征稿需求">
      <div {...stylex.props(styles.renderCfpNoticesSectionItem, styles.renderCfpNoticesDiv)}>
        <h2 {...stylex.props(styles.renderCfpNoticesH2)}>
          <Megaphone animateOnView xstyle={styles.renderCfpNoticesMegaphone} aria-hidden="true" />
          征稿需求
        </h2>
        <label {...stylex.props(styles.renderCfpNoticesLabel)}>
          <Checkbox checked={shouldShowClosed} onCheckedChange={setShouldShowClosed} />
          显示已结束及历史
        </label>
      </div>
      <RevealItem
        isReady={!noticesQuery.isLoading}
        xstyle={[styles.renderCfpNoticesSectionItem, styles.renderCfpNoticesRevealItem]}
        placeholder={
          <div>
            <output {...stylex.props(styles.cfpTrackingViewSpan)}>正在加载征稿需求</output>
            <Skeleton xstyle={styles.renderCfpNoticesSkeleton} />
          </div>
        }
      >
        {renderCfpNoticeContent(state, selected)}
      </RevealItem>
      {noticesQuery.hasNextPage && !noticesQuery.isError && (
        <Button
          variant="outline"
          xstyle={[styles.renderCfpNoticesSectionItem, styles.renderCfpNoticesButton]}
          disabled={noticesQuery.isFetchingNextPage}
          onClick={() => void noticesQuery.fetchNextPage()}
        >
          {noticesQuery.isFetchingNextPage ? '正在加载…' : '加载更多征稿'}
        </Button>
      )}
      {selected.refreshStatus === 'failed' && (
        <output
          {...stylex.props(
            styles.statusOutput,
            styles.renderCfpNoticesSectionItem,
            styles.renderCfpNoticesP,
          )}
        >
          最近一次更新未成功，当前显示上次核验的内容。
        </output>
      )}
    </section>
  );
}

/** Retain notice-error precedence over unadapted and adapted journals. */
function renderCfpNoticeState(
  state: Pick<
    CfpViewState,
    'shouldShowClosed' | 'setShouldShowClosed' | 'noticesQuery' | 'visibleNotices'
  >,
  selected: NonNullable<CfpViewState['selected']>,
) {
  const { noticesQuery } = state;
  if (noticesQuery.isError)
    return (
      <StateMessage
        tone="danger"
        title="加载征稿需求失败"
        description={noticesQuery.error.message}
        action={
          <Button variant="outline" size="sm" onClick={() => void noticesQuery.refetch()}>
            重试
          </Button>
        }
      />
    );
  if (selected.coverage === 'unadapted') return <StateMessage title="暂未适配" />;
  return renderCfpNotices(state, selected);
}

/** Render the selected journal masthead with its catalog metadata and current count. */
function renderCfpJournalHeader(
  state: Pick<CfpViewState, 'database' | 'currentCount'>,
  selected: NonNullable<CfpViewState['selected']>,
) {
  const { database, currentCount } = state;

  return (
    <section {...stylex.props(styles.renderCfpJournalHeaderSection)}>
      <div {...stylex.props(styles.renderCfpJournalHeaderDiv)}>
        <Layers xstyle={styles.renderCfpNoticesMegaphone} aria-hidden="true" />
        <span>{formatCfpDatabase(database)}</span>
      </div>
      <h2 {...stylex.props(styles.renderCfpJournalHeaderH2)}>{selected.title}</h2>
      <div {...stylex.props(styles.renderCfpJournalHeaderDiv2)}>
        <span {...stylex.props(styles.renderCfpJournalHeaderSpan)}>
          {selected.allIssns.length > 0 ? `ISSN ${selected.allIssns.join(' / ')}` : '未登记 ISSN'}
        </span>
        <Badge
          variant={selected.coverage === 'adapted' && currentCount > 0 ? 'success' : 'secondary'}
          xstyle={styles.renderCfpJournalHeaderBadge}
        >
          {selected.coverage === 'adapted' ? `${currentCount} 条已收录征稿` : '暂未适配'}
        </Badge>
      </div>
    </section>
  );
}

/** Retain metadata loading, error, missing journal and selected workspace priority. */
function renderCfpWorkspace(state: CfpViewState) {
  const { selected, isLoading, error } = state;
  if (isLoading) return null;
  if (error)
    return (
      <ReaderPlaceholder
        title="暂时无法显示征稿详情"
        description="期刊目录加载失败，请在左侧列表中重试。"
        hints={[]}
      />
    );
  if (!selected)
    return <StateMessage title="暂无可用期刊" description="当前数据库还没有期刊目录。" />;
  return (
    <>
      {renderCfpJournalHeader(state, selected)}
      {renderCfpNoticeState(state, selected)}
    </>
  );
}

/** Render the page title, database pill, and journal search. */
function renderCfpHeader(state: CfpViewState) {
  const {
    journalSearch,
    setJournalSearch,
    databasesQuery,
    databases,
    database,
    journals,
    handleDatabaseChange,
    isLoading,
  } = state;

  return (
    <div>
      <div {...stylex.props(styles.renderCfpNoticeContentDivItem)}>
        <h1 {...stylex.props(styles.renderCfpHeaderH1)}>征稿追踪</h1>
        <p {...stylex.props(styles.renderCfpHeaderP)}>按期刊发现征稿主题与投稿要求</p>
      </div>
      <div {...stylex.props(styles.renderCfpNoticeContentDivItem, styles.renderCfpHeaderDiv)}>
        <RevealItem
          isReady={!databasesQuery.isLoading}
          xstyle={styles.renderCfpHeaderRevealItem}
          placeholder={<Skeleton xstyle={styles.renderCfpHeaderSkeleton} />}
        >
          <DatabasePill
            ariaLabel="征稿数据库"
            databases={databases}
            value={database}
            onChange={handleDatabaseChange}
            formatLabel={formatCfpDatabase}
            isDisabled={databases.length === 0}
          />
        </RevealItem>
        <RevealItem
          isReady={!isLoading}
          xstyle={styles.renderCfpHeaderRevealItem2}
          placeholder={<Skeleton xstyle={styles.renderCfpHeaderSkeleton2} />}
        >
          <span {...stylex.props(styles.renderCfpHeaderSpan)}>{journals.length} 本期刊</span>
        </RevealItem>
      </div>
      <div {...stylex.props(styles.renderCfpNoticeContentDivItem, styles.renderCfpHeaderDiv2)}>
        <Search xstyle={styles.renderCfpHeaderSearch} aria-hidden="true" />
        <Input
          aria-label="搜索征稿期刊"
          placeholder="搜索刊名或 ISSN"
          xstyle={styles.renderCfpHeaderInput}
          value={journalSearch}
          onChange={(event) => setJournalSearch(event.target.value)}
          {...{ [WORKSPACE_SEARCH_ATTRIBUTE]: '' }}
        />
      </div>
    </div>
  );
}

/** Render the grouped journal directory with its loading and error states. */
function renderCfpJournalPanel(state: CfpViewState, onOpenReader: () => void) {
  const {
    setSelectedCatalogId,
    journalSearch,
    database,
    filteredGroups,
    selected,
    isLoading,
    error,
    handleJournalIntent,
    handleRetry,
  } = state;

  return (
    <div {...stylex.props(styles.renderCfpJournalPanelDiv)}>
      <RevealItem
        isReady={!isLoading}
        xstyle={styles.renderCfpJournalPanelRevealItem}
        placeholder={
          <div {...stylex.props(styles.renderCfpJournalPanelDiv2)} aria-hidden="true">
            <Skeleton
              xstyle={[styles.renderCfpNoticesSectionItem, styles.renderCfpJournalPanelSkeleton]}
            />
            <Skeleton
              xstyle={[styles.renderCfpNoticesSectionItem, styles.renderCfpJournalPanelSkeleton]}
            />
            <Skeleton
              xstyle={[styles.renderCfpNoticesSectionItem, styles.renderCfpJournalPanelSkeleton]}
            />
          </div>
        }
      >
        {error ? (
          <StateMessage
            isLive={false}
            tone="danger"
            title="加载征稿期刊失败"
            description={error instanceof Error ? error.message : '请稍后重试。'}
            action={
              <Button variant="outline" size="sm" onClick={handleRetry}>
                重试
              </Button>
            }
          />
        ) : (
          <CfpJournalList
            key={`${database}:${Boolean(journalSearch.trim())}`}
            groups={filteredGroups}
            selectedCatalogId={selected?.catalogId}
            isSearching={Boolean(journalSearch.trim())}
            onIntent={handleJournalIntent}
            onSelect={(catalogId) => {
              setSelectedCatalogId(catalogId);
              onOpenReader();
            }}
          />
        )}
      </RevealItem>
    </div>
  );
}

/** Retain the valid CFP URL database before the first-database fallback. */
function getCfpDatabase(databases: string[], selectedDatabase: string): string {
  return databases.includes(selectedDatabase) ? selectedDatabase : (databases[0] ?? '');
}
/** Resolve catalog aliases in original journal order before the first nonempty group. */
function getCfpCatalog(
  journals: CfpJournalSummary[],
  groups: CfpJournalGroup[],
  selectedId: string,
) {
  return (
    journals.find(
      (journal) => journal.catalogId === selectedId || journal.catalogAliases.includes(selectedId),
    ) ?? groups.find((group) => group.journals.length > 0)?.journals[0]
  );
}
/** Hide stale notice pages on error, otherwise retain server page order. */
function getCfpVisibleNotices(query: { isError: boolean; data?: { pages: CfpNoticePage[] } }) {
  return query.isError ? [] : (query.data?.pages.flatMap((page) => page.items) ?? []);
}

/** Prefer the server page's evaluated journal over catalog metadata. */
function getSelectedCfpJournal(
  data: { pages: CfpNoticePage[] } | undefined,
  catalog: CfpJournalSummary | undefined,
): CfpJournalSummary | undefined {
  return data?.pages[0]?.journal ?? catalog;
}

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  cfpTrackingViewDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1.5rem',
    },
  },
  cfpTrackingViewDiv: {
    marginInline: 'auto',
    width: '100%',
    maxWidth: '44rem',
    paddingInline: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '2rem',
    },
    paddingTop: {
      default: '1.75rem',
      '@media (min-width: 80rem)': '2.5rem',
    },
    paddingBottom: '3.5rem',
  },
  cfpTrackingViewRevealItem: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1.5rem',
  },
  cfpTrackingViewDivItem2: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1rem',
    },
  },
  cfpTrackingViewSpan: {
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
  cfpTrackingViewSkeleton: {
    height: '9rem',
    width: '100%',
    borderRadius: '16px',
  },
  cfpTrackingViewSkeleton2: {
    height: '18rem',
    width: '100%',
    borderRadius: '16px',
  },
  renderCfpNoticeContentDiv: {
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingTop: '1.75rem',
  },
  renderCfpNoticeContentDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  renderCfpNoticeContentDiv2: {
    paddingBottom: '1.5rem',
  },
  renderCfpNoticeContentH3: {
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    fontWeight: 700,
  },
  renderCfpNoticeContentP: {
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    overflowWrap: 'anywhere',
    whiteSpace: 'pre-line',
    color: colors.mutedForeground,
  },
  renderCfpNoticesSectionItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderCfpNoticesDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    justifyContent: 'space-between',
    columnGap: '1rem',
    rowGap: '0.5rem',
  },
  renderCfpNoticesH2: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  renderCfpNoticesMegaphone: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
  },
  renderCfpNoticesLabel: {
    display: 'flex',
    minHeight: '2.25rem',
    flexShrink: 0,
    cursor: 'pointer',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    color: colors.mutedForeground,
  },
  renderCfpNoticesRevealItem: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.5rem',
  },
  renderCfpNoticesSkeleton: {
    height: '18rem',
    width: '100%',
  },
  renderCfpNoticesButton: {
    width: '100%',
    borderRadius: '12px',
  },
  renderCfpNoticesP: {
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: colors.warningBorder,
    backgroundColor: colors.warning,
    paddingInline: '1.25rem',
    paddingBlock: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: '1.5rem',
    color: colors.warningForeground,
  },
  renderCfpJournalHeaderSection: {
    paddingBottom: '0.5rem',
  },
  renderCfpJournalHeaderDiv: {
    marginBottom: '0.75rem',
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    color: colors.mutedForeground,
  },
  renderCfpJournalHeaderH2: {
    fontSize: '24px',
    lineHeight: 1.35,
    fontWeight: 700,
    letterSpacing: '-0.01em',
    overflowWrap: 'anywhere',
  },
  renderCfpJournalHeaderDiv2: {
    marginTop: '1rem',
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    justifyContent: 'space-between',
    columnGap: '1.25rem',
    rowGap: '0.75rem',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingTop: '1rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
  },
  renderCfpJournalHeaderSpan: {
    minWidth: '0px',
    overflowWrap: 'anywhere',
    color: colors.mutedForeground,
  },
  renderCfpJournalHeaderBadge: {
    height: '1.5rem',
    paddingInline: '0.625rem',
    lineHeight: 1,
    fontVariantNumeric: 'tabular-nums',
  },
  renderCfpHeaderH1: {
    fontSize: '1.125rem',
    lineHeight: '1.75rem',
    fontWeight: 700,
  },
  renderCfpHeaderP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderCfpHeaderDiv: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
  },
  renderCfpHeaderRevealItem: {
    minWidth: '0px',
  },
  renderCfpHeaderSkeleton: {
    height: '2rem',
    width: '9rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  renderCfpHeaderRevealItem2: {
    flexShrink: 0,
  },
  renderCfpHeaderSkeleton2: {
    height: '1rem',
    width: '4rem',
  },
  renderCfpHeaderSpan: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderCfpHeaderDiv2: {
    position: 'relative',
  },
  renderCfpHeaderSearch: {
    pointerEvents: 'none',
    position: 'absolute',
    top: 'calc(1 / 2 * 100%)',
    left: '0.875rem',
    width: '1rem',
    height: '1rem',
    translate: '0 calc(calc(1 / 2 * 100%) * -1)',
    color: colors.mutedForeground,
  },
  renderCfpHeaderInput: {
    height: '2.5rem',
    borderRadius: '12px',
    backgroundColor: {
      default: colors.muted,
      ':focus-visible': {
        default: colors.background,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.background,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.accent} 50%, transparent)`,
      },
    },
    paddingLeft: '2.5rem',
    boxShadow: {
      default: `inset 0 0 0 1px ${colors.border}`,
      ':focus-visible': {
        default: `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent), inset 0 0 0 1px ${colors.border}`,
        ':is([aria-invalid="true"])': `0 0 0 3px ${colors.destructiveRing}, inset 0 0 0 1px ${colors.border}`,
      },
    },
  },
  renderCfpJournalPanelDiv: {
    display: 'flex',
    minHeight: '0px',
    flex: '1',
    flexDirection: 'column',
    paddingInline: '0.5rem',
    paddingTop: '0.5rem',
    paddingBottom: {
      default: 'calc(4.5rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom,0px)))',
      '@media (min-width: 48rem)': '0.5rem',
    },
  },
  renderCfpJournalPanelRevealItem: {
    display: 'flex',
    minHeight: '0px',
    flex: '1',
    flexDirection: 'column',
  },
  renderCfpJournalPanelDiv2: {
    padding: '0.5rem',
  },
  renderCfpJournalPanelSkeleton: {
    height: '3.5rem',
    width: '100%',
  },
});
