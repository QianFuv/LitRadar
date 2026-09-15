'use client';

/** Database and journal navigation over original notices evaluated by the backend. */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { parseAsString, useQueryState } from 'nuqs';
import { Database, Megaphone, Search } from 'lucide-react';

import { CfpNoticeCard, CfpSourceFooter } from '@/components/cfp/cfp-notice-card';
import { WorkspaceSidebar } from '@/components/feature/sidebar';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { StateMessage } from '@/components/ui/state-message';
import { ApiError, getDatabases, getCfpJournals, getCfpNotices } from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import { cn } from '@/lib/utils';

const DATABASE_LABELS: Readonly<Record<string, string>> = {
  'ccf_computer_journals.sqlite': 'CCF 计算机期刊',
  'chinese_journals.sqlite': '中文期刊',
  'english_journals.sqlite': '英文期刊',
};
/** Compose the existing workspace with independent CFP database and journal selections. */
export function CfpTrackingView() {
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
  const database = databases.includes(selectedDatabase) ? selectedDatabase : (databases[0] ?? '');
  const queryClient = useQueryClient();
  const cursorRecovery = useRef<string | null>(null);
  const journalsQuery = useQuery({
    queryKey: ['cfp', 'journals', database],
    queryFn: ({ signal }) => getCfpJournals(database, signal),
    enabled: Boolean(user && database),
  });
  const journals = useMemo(() => journalsQuery.data?.items ?? [], [journalsQuery.data]);
  const filteredJournals = useMemo(() => {
    const query = journalSearch.trim().toLocaleLowerCase();
    return journals.filter(
      (journal) =>
        !query ||
        [journal.title, ...journal.allIssns, ...journal.titleAliases].some((value) =>
          value.toLocaleLowerCase().includes(query),
        ),
    );
  }, [journalSearch, journals]);
  const selectedCatalog =
    journals.find(
      (journal) =>
        journal.catalogId === selectedCatalogId ||
        journal.catalogAliases.includes(selectedCatalogId),
    ) ?? journals[0];
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
  });
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
  const selected = noticesQuery.data?.pages[0]?.journal ?? selectedCatalog;
  const adaptedCount = journalsQuery.data?.summary.adaptedJournals ?? 0;
  const visibleNotices = noticesQuery.isError
    ? []
    : (noticesQuery.data?.pages.flatMap((page) => page.items) ?? []);
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

  return (
    <WorkspaceShell
      contentClassName="space-y-5 sm:space-y-6"
      sidebar={
        <WorkspaceSidebar
          headerContent={
            <div className="space-y-3 border-t border-sidebar-border pt-5">
              <div className="flex items-center gap-2 text-sm font-semibold">
                <Database className="size-4" aria-hidden="true" />
                数据库
              </div>
              {databasesQuery.isLoading ? (
                <Skeleton className="h-9 w-full" />
              ) : (
                <Select
                  value={database}
                  onValueChange={handleDatabaseChange}
                  disabled={databases.length === 0}
                >
                  <SelectTrigger aria-label="征稿数据库" className="w-full bg-sidebar">
                    <SelectValue placeholder="选择数据库" />
                  </SelectTrigger>
                  <SelectContent>
                    {databases.map((name) => (
                      <SelectItem key={name} value={name}>
                        {DATABASE_LABELS[name] ?? name.replace(/\.sqlite$/, '')}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
            </div>
          }
        >
          <section className="space-y-3" aria-label="征稿期刊">
            <div className="flex items-center justify-between">
              <h2 className="text-sm font-semibold">期刊</h2>
              <span className="text-xs tabular-nums text-muted-foreground">
                {journals.length} 本
              </span>
            </div>
            <div className="relative">
              <Search
                className="pointer-events-none absolute top-2.5 left-2.5 size-4 text-muted-foreground"
                aria-hidden="true"
              />
              <Input
                aria-label="搜索征稿期刊"
                placeholder="搜索刊名或 ISSN"
                className="h-9 bg-sidebar pl-9"
                value={journalSearch}
                onChange={(event) => setJournalSearch(event.target.value)}
              />
            </div>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] leading-5 text-muted-foreground">
              <span>
                已收录{' '}
                <span className="font-medium tabular-nums text-foreground">{adaptedCount}</span> 本
              </span>
              <span>暂未适配 {journals.length - adaptedCount} 本</span>
            </div>
            {isLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-14 w-full" />
                <Skeleton className="h-14 w-full" />
                <Skeleton className="h-14 w-full" />
              </div>
            ) : (
              <div className="space-y-1">
                {filteredJournals.map((journal) => (
                  <button
                    key={journal.catalogId}
                    type="button"
                    aria-pressed={selected?.catalogId === journal.catalogId}
                    title={journal.title}
                    onClick={() => void setSelectedCatalogId(journal.catalogId)}
                    className={cn(
                      'motion-control grid min-h-20 w-full grid-cols-[minmax(0,1fr)_auto] items-start gap-x-3 rounded-lg border border-transparent px-3 py-3 text-left transition-colors hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring/50',
                      selected?.catalogId === journal.catalogId &&
                        'border-sidebar-border bg-sidebar-accent',
                    )}
                  >
                    <span className="min-w-0">
                      <span className="line-clamp-2 text-[13px] font-medium leading-5 [overflow-wrap:anywhere]">
                        {journal.title}
                      </span>
                      <span className="mt-1 block text-[11px] leading-5 text-muted-foreground">
                        {journal.coverage === 'adapted' ? '已收录征稿' : '暂未适配'}
                      </span>
                    </span>
                    {journal.coverage === 'adapted' && (
                      <span
                        className="mt-0.5 inline-flex h-5 min-w-6 items-center justify-center rounded bg-sidebar-accent px-1.5 text-[11px] tabular-nums"
                        aria-label={`${journal.currentCount} 条征稿信息`}
                      >
                        {journal.currentCount}
                      </span>
                    )}
                  </button>
                ))}
                {filteredJournals.length === 0 && (
                  <p className="py-6 text-center text-sm text-muted-foreground">
                    {journalSearch ? '未找到匹配期刊' : '暂无期刊'}
                  </p>
                )}
              </div>
            )}
          </section>
        </WorkspaceSidebar>
      }
      sidebarOpenLabel="打开征稿筛选"
      sidebarDialogTitle="征稿筛选"
      sidebarDialogDescription="按数据库和期刊查看征稿信息。"
      toolbar={
        <div className="flex min-w-0 flex-1 items-center gap-3 md:mx-auto md:max-w-4xl">
          <Megaphone className="size-5 shrink-0" aria-hidden="true" />
          <div>
            <h1 className="text-xl font-semibold tracking-tight">征稿追踪</h1>
            <p className="mt-0.5 text-xs text-muted-foreground">按期刊发现征稿主题与投稿要求</p>
          </div>
        </div>
      }
    >
      {isLoading ? (
        <div role="status" className="space-y-4">
          <span className="sr-only">正在加载征稿期刊</span>
          <Skeleton className="h-28 w-full" />
          <Skeleton className="h-72 w-full" />
        </div>
      ) : error ? (
        <StateMessage
          tone="danger"
          title="加载征稿期刊失败"
          description={error instanceof Error ? error.message : '请稍后重试。'}
          action={
            <Button variant="outline" size="sm" onClick={handleRetry}>
              重试
            </Button>
          }
        />
      ) : !selected ? (
        <StateMessage title="暂无可用期刊" description="当前数据库还没有期刊目录。" />
      ) : (
        <>
          <section className="rounded-xl bg-card px-5 py-5 shadow-vercel-ring sm:px-6 sm:py-6">
            <div className="mb-3 flex items-center gap-2 text-xs leading-5 text-muted-foreground">
              <Database className="size-3.5 shrink-0" aria-hidden="true" />
              <span>{DATABASE_LABELS[database] ?? database.replace(/\.sqlite$/, '')}</span>
            </div>
            <h2 className="text-xl font-semibold leading-relaxed tracking-tight [overflow-wrap:anywhere]">
              {selected.title}
            </h2>
            <div className="mt-4 flex flex-wrap items-center justify-between gap-x-5 gap-y-3 border-t border-border/70 pt-4 text-xs leading-5">
              {selected.allIssns.length > 0 && (
                <span className="min-w-0 text-muted-foreground [overflow-wrap:anywhere]">
                  ISSN {selected.allIssns.join(' / ')}
                </span>
              )}
              <Badge variant="secondary" className="h-6 rounded-md px-2 leading-none tabular-nums">
                {selected.coverage === 'adapted' ? `${currentCount} 条已收录征稿` : '暂未适配'}
              </Badge>
            </div>
          </section>
          {noticesQuery.isError ? (
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
          ) : selected.coverage === 'unadapted' ? (
            <StateMessage title="暂未适配" />
          ) : (
            <section aria-label="征稿需求" className="space-y-4 sm:space-y-5">
              <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-5 sm:px-6">
                <h2 className="text-sm font-semibold">征稿需求</h2>
                <label className="flex min-h-9 shrink-0 cursor-pointer items-center gap-2 text-xs leading-5 text-muted-foreground">
                  <Checkbox
                    checked={shouldShowClosed}
                    onCheckedChange={(value) => setShouldShowClosed(value === true)}
                  />
                  显示已结束及历史
                </label>
              </div>
              {noticesQuery.isLoading ? (
                <div role="status">
                  <span className="sr-only">正在加载征稿需求</span>
                  <Skeleton className="h-72 w-full" />
                </div>
              ) : visibleNotices.length ? (
                visibleNotices.map((notice) => <CfpNoticeCard key={notice.id} notice={notice} />)
              ) : selected.sourceStatement ? (
                <div className="overflow-hidden rounded-xl bg-card shadow-vercel-ring">
                  <div className="space-y-3 px-5 py-5 sm:px-6 sm:py-6">
                    <h3 className="text-sm font-semibold leading-6">暂无已收录的征稿公告</h3>
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
              ) : (
                <StateMessage
                  title="暂无已收录的进行中征稿"
                  description="可以查看已结束的信息，或选择其他期刊。"
                />
              )}
              {noticesQuery.hasNextPage && !noticesQuery.isError && (
                <Button
                  variant="outline"
                  className="w-full"
                  disabled={noticesQuery.isFetchingNextPage}
                  onClick={() => void noticesQuery.fetchNextPage()}
                >
                  {noticesQuery.isFetchingNextPage ? '正在加载…' : '加载更多征稿'}
                </Button>
              )}
              {selected.refreshStatus === 'failed' && (
                <p
                  role="status"
                  className="rounded-lg border border-amber-200/70 bg-amber-50/50 px-5 py-3 text-xs leading-6 text-amber-800 dark:border-amber-900 dark:bg-amber-950/30 dark:text-amber-300 sm:px-6"
                >
                  最近一次更新未成功，当前显示上次核验的内容。
                </p>
              )}
            </section>
          )}
        </>
      )}
    </WorkspaceShell>
  );
}
