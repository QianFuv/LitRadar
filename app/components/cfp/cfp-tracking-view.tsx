'use client';

/** Database and journal navigation over original notices evaluated by the backend. */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { parseAsString, useQueryState } from 'nuqs';
import { Database, Search } from 'lucide-react';

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
import { Skeleton } from '@/components/ui/skeleton';
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

const DATABASE_LABELS: Readonly<Record<string, string>> = {
  'ccf_computer_journals.sqlite': 'CCF 计算机期刊',
  'chinese_journals.sqlite': '中文期刊',
  'english_journals.sqlite': '英文期刊',
};
/**
 * Format a CFP database file name for display.
 *
 * @param name - Database file name.
 * @returns Human-readable database label.
 */
function formatCfpDatabase(name: string): string {
  return DATABASE_LABELS[name] ?? name.replace(/\.sqlite$/, '');
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
          <div className="mx-auto w-full max-w-[44rem] space-y-6 px-5 pt-7 pb-14 sm:px-8 xl:pt-10">
            {renderCfpWorkspace(state)}
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
  const [selectedDatabase, setSelectedDatabase] = useQueryState(
    'cfp_db',
    parseAsString.withDefault(''),
  );
  const [selectedCatalogId, setSelectedCatalogId] = useQueryState(
    'cfp_journal',
    parseAsString.withDefault(''),
  );
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
  const prefetchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (prefetchTimer.current) clearTimeout(prefetchTimer.current);
    },
    [],
  );

  /** Prefetch the first notice page of a journal the user is about to open. */
  function handleJournalIntent(catalogId: string): void {
    if (prefetchTimer.current) clearTimeout(prefetchTimer.current);
    if (!user || !database) return;
    prefetchTimer.current = setTimeout(() => {
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
    }, CFP_PREFETCH_INTENT_DELAY_MS);
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

  /** Change CFP databases atomically through nuqs batching and reset dependent journal selection. */
  function handleDatabaseChange(value: string): void {
    void setSelectedDatabase(value);
    void setSelectedCatalogId(null);
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
  if (noticesQuery.isLoading)
    return (
      <div role="status">
        <span className="sr-only">正在加载征稿需求</span>
        <Skeleton className="h-72 w-full" />
      </div>
    );
  if (visibleNotices.length)
    return visibleNotices.map((notice) => <CfpNoticeCard key={notice.id} notice={notice} />);
  if (selected.sourceStatement)
    return (
      <div className="border-t pt-7">
        <div className="space-y-3 pb-6">
          <h3 className="text-sm font-bold leading-6">暂无已收录的征稿公告</h3>
          <p className="whitespace-pre-line text-sm leading-6 text-muted-foreground [overflow-wrap:anywhere]">
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
    <section aria-label="征稿需求" className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <h2 className="text-sm font-bold">征稿需求</h2>
        <label className="flex min-h-9 shrink-0 cursor-pointer items-center gap-2 text-xs leading-5 text-muted-foreground">
          <Checkbox
            checked={shouldShowClosed}
            onCheckedChange={(value) => setShouldShowClosed(value === true)}
          />
          显示已结束及历史
        </label>
      </div>
      {renderCfpNoticeContent(state, selected)}
      {noticesQuery.hasNextPage && !noticesQuery.isError && (
        <Button
          variant="outline"
          className="w-full rounded-xl"
          disabled={noticesQuery.isFetchingNextPage}
          onClick={() => void noticesQuery.fetchNextPage()}
        >
          {noticesQuery.isFetchingNextPage ? '正在加载…' : '加载更多征稿'}
        </Button>
      )}
      {selected.refreshStatus === 'failed' && (
        <p
          role="status"
          className="rounded-xl border border-warning-border bg-warning px-5 py-3 text-xs leading-6 text-warning-foreground"
        >
          最近一次更新未成功，当前显示上次核验的内容。
        </p>
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
    <section className="pb-2">
      <div className="mb-3 flex items-center gap-2 text-xs leading-5 text-muted-foreground">
        <Database className="size-3.5 shrink-0" aria-hidden="true" />
        <span>{formatCfpDatabase(database)}</span>
      </div>
      <h2 className="text-[24px] leading-[1.35] font-bold tracking-[-0.01em] [overflow-wrap:anywhere]">
        {selected.title}
      </h2>
      <div className="mt-4 flex flex-wrap items-center justify-between gap-x-5 gap-y-3 border-t pt-4 text-xs leading-5">
        <span className="min-w-0 text-muted-foreground [overflow-wrap:anywhere]">
          {selected.allIssns.length > 0 ? `ISSN ${selected.allIssns.join(' / ')}` : '未登记 ISSN'}
        </span>
        <Badge
          variant={selected.coverage === 'adapted' && currentCount > 0 ? 'success' : 'secondary'}
          className="h-6 px-2.5 leading-none tabular-nums"
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
  if (isLoading)
    return (
      <div role="status" className="space-y-4">
        <span className="sr-only">正在加载征稿期刊</span>
        <Skeleton className="h-36 w-full rounded-2xl" />
        <Skeleton className="h-72 w-full rounded-2xl" />
      </div>
    );
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
  } = state;

  return (
    <div className="space-y-3">
      <div>
        <h1 className="text-lg leading-7 font-bold">征稿追踪</h1>
        <p className="text-xs text-muted-foreground">按期刊发现征稿主题与投稿要求</p>
      </div>
      <div className="flex items-center justify-between gap-3">
        {databasesQuery.isLoading ? (
          <Skeleton className="h-8 w-36 rounded-full" />
        ) : (
          <DatabasePill
            ariaLabel="征稿数据库"
            databases={databases}
            value={database}
            onChange={handleDatabaseChange}
            formatLabel={formatCfpDatabase}
            isDisabled={databases.length === 0}
          />
        )}
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
          {journals.length} 本期刊
        </span>
      </div>
      <div className="relative">
        <Search
          className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden="true"
        />
        <Input
          aria-label="搜索征稿期刊"
          placeholder="搜索刊名或 ISSN"
          className="h-10 rounded-xl bg-muted pl-10 !shadow-[inset_0_0_0_1px_var(--border)] focus-visible:bg-background"
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
    <div className="flex min-h-0 flex-1 flex-col px-2 pt-2 pb-[calc(4.5rem+var(--safe-area-inset-bottom,env(safe-area-inset-bottom,0px)))] md:pb-2">
      {isLoading ? (
        <div className="space-y-2 p-2" aria-hidden="true">
          <Skeleton className="h-14 w-full" />
          <Skeleton className="h-14 w-full" />
          <Skeleton className="h-14 w-full" />
        </div>
      ) : error ? (
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
            void setSelectedCatalogId(catalogId);
            onOpenReader();
          }}
        />
      )}
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
