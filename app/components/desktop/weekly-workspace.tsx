'use client';

/**
 * Desktop weekly updates workspace.
 */

import { useQuery } from '@tanstack/react-query';
import { CalendarDays, Search } from 'lucide-react';
import { useRouter, useSearchParams } from 'next/navigation';
import { useMemo, useState } from 'react';
import { ArticleCard } from '@/components/desktop/article-tools';
import { ShellConfigurator } from '@/components/desktop/shell';
import {
  Badge,
  Button,
  EmptyState,
  Field,
  Notice,
  Panel,
  SelectInput,
  Skeleton,
  TextInput,
} from '@/components/desktop/ui';
import {
  checkFavoritesBatch,
  getArticles,
  getDatabases,
  getWeeklyUpdates,
  type JournalId,
  type WeeklyArticle,
  type WeeklyDatabaseUpdate,
  type WeeklyJournalUpdate,
} from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';
import { useActiveDatabase } from '@/lib/hooks';
import { formatCount, formatDateRange } from '@/lib/format';

/**
 * Resolve the active journal from a list.
 *
 * @param journals - Weekly journals.
 * @param selectedJournalId - Current selected journal.
 * @returns Effective journal id.
 */
function resolveSelectedJournal(
  journals: WeeklyJournalUpdate[],
  selectedJournalId: JournalId | null,
): JournalId | null {
  if (journals.length === 0) {
    return null;
  }
  if (selectedJournalId && journals.some((journal) => journal.journal_id === selectedJournalId)) {
    return selectedJournalId;
  }
  return journals[0].journal_id;
}

/**
 * Get a readable journal label.
 *
 * @param journal - Weekly journal.
 * @returns Display label.
 */
function getJournalLabel(journal: WeeklyJournalUpdate): string {
  return journal.journal_title?.trim() || `期刊 ${journal.journal_id}`;
}

/**
 * Build a map of weekly databases.
 *
 * @param databases - Weekly database records.
 * @returns Database map.
 */
function buildWeeklyDatabaseMap(
  databases: WeeklyDatabaseUpdate[],
): Map<string, WeeklyDatabaseUpdate> {
  return new Map(databases.map((database) => [database.db_name, database]));
}

/**
 * Intersect backend search results with weekly articles.
 *
 * @param weeklyArticles - Weekly articles for a selected journal.
 * @param searchedArticles - Search result articles.
 * @returns Weekly articles that matched search.
 */
function intersectWeeklyArticles(
  weeklyArticles: WeeklyArticle[],
  searchedArticles: WeeklyArticle[] | undefined,
): WeeklyArticle[] {
  if (!searchedArticles) {
    return [];
  }
  const weeklyById = new Map(weeklyArticles.map((article) => [article.article_id, article]));
  return searchedArticles
    .map((article) => weeklyById.get(article.article_id))
    .filter((article): article is WeeklyArticle => Boolean(article));
}

/**
 * Render the weekly updates workspace.
 *
 * @returns Weekly workspace.
 */
export function WeeklyWorkspace() {
  const { token, user } = useAuthSession();
  const router = useRouter();
  const searchParams = useSearchParams();
  const requestedDb = searchParams.get('db') ?? '';
  const query = searchParams.get('q') ?? '';
  const [selectedJournalId, setSelectedJournalId] = useState<JournalId | null>(null);
  const [queryDraft, setQueryDraft] = useState(query);

  const weeklyQuery = useQuery({
    queryKey: ['weekly-updates'],
    queryFn: () => getWeeklyUpdates(token!),
    enabled: Boolean(token),
    staleTime: 5 * 60_000,
  });

  const databaseQuery = useQuery({
    queryKey: ['databases'],
    queryFn: () => getDatabases(token!),
    enabled: Boolean(token),
    staleTime: 10 * 60_000,
  });

  const weeklyMap = useMemo(
    () => buildWeeklyDatabaseMap(weeklyQuery.data?.databases ?? []),
    [weeklyQuery.data?.databases],
  );
  const availableDatabases = useMemo(() => {
    const merged = new Set<string>(databaseQuery.data ?? []);
    for (const dbName of weeklyMap.keys()) {
      merged.add(dbName);
    }
    return Array.from(merged);
  }, [databaseQuery.data, weeklyMap]);
  const [effectiveDb, selectDatabaseState] = useActiveDatabase(
    availableDatabases,
    requestedDb || undefined,
  );
  const selectedDbData = weeklyMap.get(effectiveDb) ?? null;
  const journals = selectedDbData?.journals ?? [];
  const effectiveJournalId = resolveSelectedJournal(journals, selectedJournalId);
  const selectedJournal =
    journals.find((journal) => journal.journal_id === effectiveJournalId) ?? null;
  const weeklyArticles = selectedJournal?.articles ?? [];

  const searchedArticlesQuery = useQuery({
    queryKey: ['weekly-article-search', effectiveDb, effectiveJournalId, query],
    queryFn: async () => {
      const params = new URLSearchParams();
      params.set('q', query);
      params.set('limit', '200');
      if (effectiveJournalId) {
        params.append('journal_id', String(effectiveJournalId));
      }
      const page = await getArticles(token!, effectiveDb, params, null, false);
      return page.items;
    },
    enabled: Boolean(token && query && effectiveDb && effectiveJournalId),
  });

  const visibleArticles = query
    ? intersectWeeklyArticles(weeklyArticles, searchedArticlesQuery.data)
    : weeklyArticles;
  const visibleArticleIds = visibleArticles.map((article) => article.article_id);
  const favoriteQuery = useQuery({
    queryKey: ['favorite-batch', user?.id, effectiveDb, visibleArticleIds.join(',')],
    queryFn: () => checkFavoritesBatch(token!, visibleArticleIds, effectiveDb),
    enabled: Boolean(token && user && visibleArticleIds.length > 0 && effectiveDb),
  });
  const totalWeeklyArticles = weeklyQuery.data?.databases.reduce(
    (sum, database) => sum + database.new_article_count,
    0,
  );

  const setUrlState = (nextDb: string, nextQuery: string) => {
    const params = new URLSearchParams();
    if (nextDb) {
      params.set('db', nextDb);
    }
    if (nextQuery.trim()) {
      params.set('q', nextQuery.trim());
    }
    router.replace(`/weekly-updates?${params.toString()}`, { scroll: false });
  };

  return (
    <>
      <ShellConfigurator
        kicker="Weekly Digest"
        title="期刊每周更新"
        actions={
          <>
            <Badge tone="teal">
              <CalendarDays size={13} />
              {weeklyQuery.data
                ? formatDateRange(weeklyQuery.data.window_start, weeklyQuery.data.window_end)
                : '加载中'}
            </Badge>
            <Badge tone="violet">{formatCount(totalWeeklyArticles)} 篇新增</Badge>
          </>
        }
      />
      <div className="workspace-grid workspace-grid--two">
        <Panel title="更新索引" meta="按数据库和期刊切换">
          <div className="form-grid">
            <Field label="数据库">
              <SelectInput
                value={effectiveDb}
                onChange={(event) => {
                  const dbName = event.target.value;
                  selectDatabaseState(dbName);
                  setSelectedJournalId(null);
                  setUrlState(dbName, query);
                }}
              >
                {availableDatabases.map((dbName) => (
                  <option key={dbName} value={dbName}>
                    {dbName}
                  </option>
                ))}
              </SelectInput>
            </Field>
            {weeklyQuery.isPending ? (
              <>
                <Skeleton className="h-16" />
                <Skeleton className="h-16" />
              </>
            ) : weeklyQuery.isError ? (
              <Notice tone="error">{weeklyQuery.error.message}</Notice>
            ) : journals.length === 0 ? (
              <EmptyState>当前时间窗口没有新增期刊。</EmptyState>
            ) : (
              <div
                className="list-stack"
                style={{ maxHeight: 'calc(100vh - 220px)', overflow: 'auto' }}
              >
                {journals.map((journal) => {
                  const active = journal.journal_id === effectiveJournalId;
                  return (
                    <button
                      key={journal.journal_id}
                      className="article-row"
                      type="button"
                      style={{
                        borderColor: active ? 'var(--teal)' : undefined,
                        background: active ? 'var(--teal-soft)' : undefined,
                      }}
                      onClick={() => setSelectedJournalId(journal.journal_id)}
                    >
                      <div className="toolbar toolbar--wrap">
                        <strong>{getJournalLabel(journal)}</strong>
                        <Badge tone={active ? 'teal' : 'neutral'}>
                          {journal.new_article_count}
                        </Badge>
                      </div>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </Panel>

        <Panel
          title={selectedJournal ? getJournalLabel(selectedJournal) : '文章'}
          meta={
            selectedJournal
              ? query
                ? `匹配 ${visibleArticles.length} / ${weeklyArticles.length} 篇`
                : `本周新增 ${selectedJournal.new_article_count} 篇`
              : '请选择期刊'
          }
          actions={
            <div className="toolbar">
              <TextInput
                value={queryDraft}
                onChange={(event) => setQueryDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    setUrlState(effectiveDb, queryDraft);
                  }
                }}
                placeholder="在当前期刊的新文章中检索"
                style={{ width: 320 }}
              />
              <Button
                icon={<Search size={15} />}
                onClick={() => setUrlState(effectiveDb, queryDraft)}
              >
                检索
              </Button>
            </div>
          }
        >
          {!selectedJournal ? (
            <EmptyState>请选择左侧期刊。</EmptyState>
          ) : query && searchedArticlesQuery.isPending ? (
            <div className="list-stack">
              <Skeleton className="h-28" />
              <Skeleton className="h-28" />
            </div>
          ) : query && searchedArticlesQuery.isError ? (
            <Notice tone="error">{searchedArticlesQuery.error.message}</Notice>
          ) : visibleArticles.length === 0 ? (
            <EmptyState>没有匹配的本周文章。</EmptyState>
          ) : (
            <div className="list-stack scroll-region scroll-region--results">
              {visibleArticles.map((article) => (
                <ArticleCard
                  key={article.article_id}
                  article={article}
                  dbName={effectiveDb}
                  favoriteFolderIds={
                    favoriteQuery.data?.[article.article_id]?.map((item) => item.folder_id) ?? []
                  }
                  favoritePending={favoriteQuery.isPending}
                />
              ))}
            </div>
          )}
        </Panel>
      </div>
    </>
  );
}
