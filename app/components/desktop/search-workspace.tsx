'use client';

/**
 * Desktop search workspace for article discovery.
 */

import { useQuery } from '@tanstack/react-query';
import {
  ChevronLeft,
  ChevronRight,
  Grid2X2,
  LayoutList,
  Plus,
  RotateCcw,
  Search as SearchIcon,
  SlidersHorizontal,
  Trash2,
} from 'lucide-react';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { AnnouncementsModal } from '@/components/desktop/announcements';
import { ArticleCard, ArticleDetailPanel } from '@/components/desktop/article-tools';
import {
  Button,
  CheckboxRow,
  EmptyState,
  IconButton,
  Modal,
  Notice,
  Panel,
  SelectInput,
  Skeleton,
  TextInput,
  joinClassNames,
} from '@/components/desktop/ui';
import { ShellConfigurator } from '@/components/desktop/shell';
import {
  checkFavoritesBatch,
  getAreas,
  getArticles,
  getDatabases,
  getJournalOptions,
  type Article,
} from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';
import {
  useActiveDatabase,
  useSearchHistory,
  formatHistoryTime,
  type SearchHistoryEntry,
} from '@/lib/hooks';
import { formatCount } from '@/lib/format';

const DEFAULT_PAGE_SIZE = 20;
const PAGE_SIZE_OPTIONS = [10, 20, 50];


type SearchScope = 'topic' | 'title' | 'author' | 'doi';
type SortMode = 'date-desc' | 'date-asc';
type ViewMode = 'list' | 'grid';
type AdvancedSearchLogic = 'AND' | 'OR' | 'NOT';
type AdvancedSearchField = 'all' | 'title' | 'abstract' | 'authors' | 'doi' | 'journal_title';

interface AdvancedSearchRow {
  id: string;
  logic: AdvancedSearchLogic;
  field: AdvancedSearchField;
  value: string;
}

interface AdvancedSearchPayload {
  query: string;
}

interface AdvancedSearchModalProps {
  open: boolean;
  query: string;
  onApply: (payload: AdvancedSearchPayload) => void;
  onClose: () => void;
}

const ADVANCED_SEARCH_FIELD_OPTIONS: Array<{ value: AdvancedSearchField; label: string }> = [
  { value: 'all', label: '主题' },
  { value: 'title', label: '标题' },
  { value: 'abstract', label: '摘要' },
  { value: 'authors', label: '作者' },
  { value: 'doi', label: 'DOI' },
  { value: 'journal_title', label: '期刊' },
];
const ADVANCED_SEARCH_LOGIC_OPTIONS: AdvancedSearchLogic[] = ['AND', 'OR', 'NOT'];
let advancedSearchRowCounter = 0;

/**
 * Parse an integer query parameter.
 *
 * @param value - Query parameter value.
 * @returns Parsed number or null.
 */
function parseIntegerParam(value: string | null): number | null {
  if (!value) {
    return null;
  }
  const parsed = Number(value);
  return Number.isInteger(parsed) ? parsed : null;
}

/**
 * Build a URL with updated search params.
 *
 * @param pathname - Current pathname.
 * @param params - Updated search params.
 * @returns Route URL.
 */
function buildRoute(pathname: string, params: URLSearchParams): string {
  const query = params.toString();
  return query ? `${pathname}?${query}` : pathname;
}

/**
 * Return a query-param array with one value added or removed.
 *
 * @param values - Current values.
 * @param value - Value to toggle.
 * @param checked - Whether the value should be present.
 * @returns Updated values.
 */
function toggleParamValue(values: string[], value: string, checked: boolean): string[] {
  if (checked) {
    return values.includes(value) ? values : [...values, value];
  }
  return values.filter((item) => item !== value);
}

/**
 * Create article query params from the URL state.
 *
 * @param query - Text query.
 * @param areas - Area filters.
 * @param journalIds - Journal filters.
 * @param yearMin - Minimum year.
 * @param yearMax - Maximum year.
 * @param pageSize - Page size.
 * @returns Backend article params.
 */
function buildArticleParams(
  query: string,
  areas: string[],
  journalIds: string[],
  yearMin: number | null,
  yearMax: number | null,
  pageSize: number,
  sort: string,
): URLSearchParams {
  const params = new URLSearchParams();
  if (query.trim()) {
    params.set('q', query.trim());
  }
  for (const area of areas) {
    params.append('area', area);
  }
  for (const journalId of journalIds) {
    params.append('journal_id', journalId);
  }
  if (yearMin) {
    params.set('date_from', `${yearMin}-01-01`);
  }
  if (yearMax) {
    params.set('date_to', `${yearMax}-12-31`);
  }
  params.set('limit', String(pageSize));
  if (sort === 'date-desc') {
    params.set('sort', 'date:desc');
  } else if (sort === 'date-asc') {
    params.set('sort', 'date:asc');
  }
  return params;
}

/**
 * Build a scoped full-text query from the search bar controls.
 *
 * @param scope - Selected query scope.
 * @param query - Raw query text.
 * @returns Query string sent to the route.
 */
function buildScopedQuery(scope: SearchScope, query: string): string {
  const trimmedQuery = query.trim();
  if (!trimmedQuery || trimmedQuery.includes(':') || scope === 'topic') {
    return trimmedQuery;
  }
  if (scope === 'title') {
    return `title:${trimmedQuery}`;
  }
  if (scope === 'author') {
    return `authors:${trimmedQuery}`;
  }
  return `doi:${trimmedQuery}`;
}

/**
 * Highlight query terms inside article title or abstract.
 *
 * @param text - Text to render.
 * @param query - Search query.
 * @returns Highlighted text nodes.
 */
function renderHighlightedText(text: string | null | undefined, query: string): ReactNode {
  if (!text) {
    return null;
  }
  const terms = query
    .replace(/"([^"]+)"/g, '$1')
    .split(/\s+/)
    .map((term) => term.replace(/[():*{}]/g, '').trim())
    .filter(
      (term) => term.length > 2 && !['AND', 'OR', 'NOT', 'NEAR'].includes(term.toUpperCase()),
    );
  const uniqueTerms = Array.from(new Set(terms));
  if (uniqueTerms.length === 0) {
    return text;
  }
  const escapedTerms = uniqueTerms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const pattern = new RegExp(`(${escapedTerms.join('|')})`, 'gi');
  return text.split(pattern).map((part, index) =>
    index % 2 === 1 ? (
      <mark
        key={`${part}-${index}`}
        style={{ background: 'var(--amber-soft)', color: 'var(--amber)' }}
      >
        {part}
      </mark>
    ) : (
      part
    ),
  );
}

/**
 * Sort the current article page.
 *
 * @param articles - Current page articles.
 * @param sortMode - Selected sort mode.
 * @returns Sorted articles.
 */
function sortArticles(articles: Article[], sortMode: SortMode): Article[] {
  const nextArticles = [...articles];
  if (sortMode === 'date-desc') {
    return nextArticles.sort((left, right) => (right.date || '').localeCompare(left.date || ''));
  }
  if (sortMode === 'date-asc') {
    return nextArticles.sort((left, right) => (left.date || '').localeCompare(right.date || ''));
  }
  return nextArticles;
}

/**
 * Build visible page buttons around the current page.
 *
 * @param currentPage - Current page number.
 * @param totalPages - Total page count.
 * @returns Page numbers to render.
 */
function getVisiblePageNumbers(currentPage: number, totalPages: number): number[] {
  const pages = new Set<number>([1, currentPage, totalPages]);
  for (
    let page = Math.max(1, currentPage - 2);
    page <= Math.min(totalPages, currentPage + 2);
    page += 1
  ) {
    pages.add(page);
  }
  return Array.from(pages).sort((left, right) => left - right);
}

/**
 * Resolve a display label for a database name.
 *
 * @param dbName - Raw database name.
 * @returns User-facing database label.
 */
function getDatabaseLabel(dbName: string): string {
  return dbName.replace(/\.sqlite$/i, '').replace(/[-_]/g, ' ');
}



/**
 * Create a blank advanced search row.
 *
 * @param logic - Connector used before this row.
 * @returns New advanced search row.
 */
function createAdvancedSearchRow(logic: AdvancedSearchLogic = 'AND'): AdvancedSearchRow {
  advancedSearchRowCounter += 1;
  return {
    field: 'all',
    id: `advanced-search-row-${advancedSearchRowCounter}`,
    logic,
    value: '',
  };
}

/**
 * Create modal rows from the current URL query.
 *
 * @param query - Current query string.
 * @returns Initial advanced search rows.
 */
function createAdvancedSearchRows(query: string): AdvancedSearchRow[] {
  return [{ ...createAdvancedSearchRow(), value: query.trim() }];
}

/**
 * Escape a phrase for SQLite FTS5 double-quoted syntax.
 *
 * @param value - Raw phrase value.
 * @returns Escaped phrase value.
 */
function escapeFtsPhrase(value: string): string {
  return value.replace(/"/g, '""');
}

/**
 * Normalize a user-entered value into an FTS5 term or phrase.
 *
 * @param value - Raw row value.
 * @returns FTS5-ready value.
 */
function normalizeFtsValue(value: string): string {
  const trimmedValue = value.trim().replace(/\s+/g, ' ');
  if (!trimmedValue || /^".*"$/.test(trimmedValue)) {
    return trimmedValue;
  }
  return /\s/.test(trimmedValue) ? `"${escapeFtsPhrase(trimmedValue)}"` : trimmedValue;
}

/**
 * Build one FTS5 clause from an advanced row.
 *
 * @param row - Advanced search row.
 * @returns FTS5 clause.
 */
function buildAdvancedClause(row: AdvancedSearchRow): string {
  const value = normalizeFtsValue(row.value);
  if (!value) {
    return '';
  }
  return row.field === 'all' ? value : `${row.field}:${value}`;
}

/**
 * Build an advanced query from dynamic modal rows.
 *
 * @param rows - Advanced search rows.
 * @returns Query string.
 */
function buildAdvancedQuery(rows: AdvancedSearchRow[]): string {
  return rows
    .map((row) => ({ clause: buildAdvancedClause(row), logic: row.logic }))
    .filter((row) => row.clause)
    .map((row, index) => (index === 0 ? row.clause : `${row.logic} ${row.clause}`))
    .join(' ');
}



/**
 * Render the search-history block shown in the sidebar.
 *
 * @param props - History props.
 * @returns History panel.
 */
function SearchHistoryPanel({
  entries,
  onSelect,
}: {
  entries: SearchHistoryEntry[];
  onSelect: (query: string) => void;
}) {
  return (
    <section className="sidebar-history">
      <div className="sidebar-history__header">
        <strong>检索历史</strong>
        <span>更多</span>
      </div>
      <div className="sidebar-history__list">
        {entries.length === 0 ? (
          <span className="panel__meta">暂无历史</span>
        ) : (
          entries.slice(0, 5).map((entry) => (
            <button
              key={`${entry.query}-${entry.timestamp}`}
              type="button"
              onClick={() => onSelect(entry.query)}
            >
              <span>{entry.query}</span>
              <span>{formatHistoryTime(entry.timestamp)}</span>
            </button>
          ))
        )}
      </div>
    </section>
  );
}



/**
 * Render the advanced search modal.
 *
 * @param props - Advanced modal props.
 * @returns Advanced search modal.
 */
function AdvancedSearchModal({ onApply, onClose, open, query }: AdvancedSearchModalProps) {
  const [rows, setRows] = useState<AdvancedSearchRow[]>(() => createAdvancedSearchRows(query));

  return (
    <Modal
      open={open}
      title="高级检索"
      description="组合多个检索条件进行精确匹配"
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            icon={<SearchIcon size={15} />}
            variant="primary"
            onClick={() => {
              onApply({
                query: buildAdvancedQuery(rows),
              });
              onClose();
            }}
          >
            应用检索
          </Button>
        </>
      }
    >
      <div className="advanced-search-rows">
        {rows.map((row, index) => (
          <div key={row.id} className="advanced-search-row">
            {index === 0 ? (
              <span className="advanced-search-row__first-label">首项</span>
            ) : (
              <SelectInput
                aria-label="组合方式"
                value={row.logic}
                onChange={(event) =>
                  setRows((currentRows) =>
                    currentRows.map((currentRow) =>
                      currentRow.id === row.id
                        ? {
                            ...currentRow,
                            logic: event.target.value as AdvancedSearchLogic,
                          }
                        : currentRow,
                    ),
                  )
                }
              >
                {ADVANCED_SEARCH_LOGIC_OPTIONS.map((logic) => (
                  <option key={logic} value={logic}>
                    {logic}
                  </option>
                ))}
              </SelectInput>
            )}
            <SelectInput
              aria-label="检索字段"
              value={row.field}
              onChange={(event) =>
                setRows((currentRows) =>
                  currentRows.map((currentRow) =>
                    currentRow.id === row.id
                      ? {
                          ...currentRow,
                          field: event.target.value as AdvancedSearchField,
                        }
                      : currentRow,
                  ),
                )
              }
            >
              {ADVANCED_SEARCH_FIELD_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </SelectInput>
            <TextInput
              value={row.value}
              onChange={(event) =>
                setRows((currentRows) =>
                  currentRows.map((currentRow) =>
                    currentRow.id === row.id
                      ? {
                          ...currentRow,
                          value: event.target.value,
                        }
                      : currentRow,
                  ),
                )
              }
              placeholder="deep learning"
            />
            <IconButton
              aria-label="删除条件"
              disabled={rows.length === 1}
              title="删除条件"
              onClick={() =>
                setRows((currentRows) =>
                  currentRows.filter((currentRow) => currentRow.id !== row.id),
                )
              }
            >
              <Trash2 size={15} />
            </IconButton>
          </div>
        ))}
        <div className="advanced-search-actions">
          <Button
            icon={<Plus size={14} />}
            size="small"
            variant="ghost"
            onClick={() => setRows((currentRows) => [...currentRows, createAdvancedSearchRow()])}
          >
            添加条件
          </Button>
        </div>
      </div>
    </Modal>
  );
}

/**
 * Render the desktop article search workspace.
 *
 * @returns Search workspace.
 */
export function SearchWorkspace() {
  const { token, user } = useAuthSession();
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();
  const [selectedArticleId, setSelectedArticleId] = useState<string | null>(null);
  const [isDetailVisible, setIsDetailVisible] = useState(true);
  const [queryDraft, setQueryDraft] = useState(searchParams.get('q') ?? '');
  const [journalSearch, setJournalSearch] = useState('');
  const [searchScope, setSearchScope] = useState<SearchScope>('topic');
  const [sortMode, setSortMode] = useState<SortMode>('date-desc');
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [isAdvancedOpen, setIsAdvancedOpen] = useState(false);
  const [searchDurationMs, setSearchDurationMs] = useState<number | null>(null);
  const { entries: historyEntries, addEntry: addHistoryEntry } = useSearchHistory();
  const query = searchParams.get('q') ?? '';
  const areas = useMemo(() => searchParams.getAll('area'), [searchParams]);
  const journalIds = useMemo(() => searchParams.getAll('journal_id'), [searchParams]);
  const yearMin = parseIntegerParam(searchParams.get('year_min'));
  const yearMax = parseIntegerParam(searchParams.get('year_max'));

  const [journalHistory, setJournalHistory] = useState<{ journal_id: string; title: string }[]>(
    () => {
      if (typeof window === 'undefined') {
        return [];
      }
      try {
        const stored = window.localStorage.getItem('paper_scanner_journal_history');
        return stored ? JSON.parse(stored) : [];
      } catch {
        return [];
      }
    },
  );

  const addToJournalHistory = (journal: { journal_id: string; title: string }) => {
    setJournalHistory((prev) => {
      const filtered = prev.filter((item) => item.journal_id !== journal.journal_id);
      const next = [journal, ...filtered].slice(0, 5);
      window.localStorage.setItem('paper_scanner_journal_history', JSON.stringify(next));
      return next;
    });
  };

  const { data: databases = [] } = useQuery({
    queryKey: ['databases'],
    queryFn: () => getDatabases(token!),
    enabled: Boolean(token),
  });

  const [effectiveDb, selectDatabaseState] = useActiveDatabase(databases);

  useEffect(() => {
    setQueryDraft(query);
  }, [query]);

  useEffect(() => {
    setPage(1);
  }, [effectiveDb, query, searchParams, pageSize]);

  const { data: areaOptions = [], isPending: areasPending } = useQuery({
    queryKey: ['areas', effectiveDb],
    queryFn: () => getAreas(token!, effectiveDb),
    enabled: Boolean(token && effectiveDb),
  });

  const { data: journalOptions = [], isPending: journalsPending } = useQuery({
    queryKey: ['journals', effectiveDb],
    queryFn: () => getJournalOptions(token!, effectiveDb),
    enabled: Boolean(token && effectiveDb),
  });



  const articleParams = useMemo(
    () => buildArticleParams(query, areas, journalIds, yearMin, yearMax, pageSize, sortMode),
    [areas, journalIds, pageSize, query, yearMax, yearMin, sortMode],
  );
  const articleParamString = articleParams.toString();
  const articleOffset = (page - 1) * pageSize;

  const articleQuery = useQuery({
    queryKey: ['articles', effectiveDb, articleParamString, articleOffset],
    queryFn: async () => {
      const startedAt = performance.now();
      const articlePage = await getArticles(
        token!,
        effectiveDb,
        articleParams,
        articleOffset,
        true,
      );
      setSearchDurationMs(Math.max(1, Math.round(performance.now() - startedAt)));
      return articlePage;
    },
    enabled: Boolean(token && effectiveDb),
  });

  const sortedArticles = useMemo(
    () => sortArticles(articleQuery.data?.items ?? [], sortMode),
    [articleQuery.data?.items, sortMode],
  );
  const selectedIndex = sortedArticles.findIndex(
    (article) => article.article_id === selectedArticleId,
  );
  const selectedArticle =
    isDetailVisible && selectedIndex >= 0 ? sortedArticles[selectedIndex] : null;
  const articleIds = sortedArticles.map((article) => article.article_id);
  const articleIdsKey = articleIds.join(',');
  const favoriteQuery = useQuery({
    queryKey: ['favorite-batch', user?.id, effectiveDb, articleIdsKey],
    queryFn: () => checkFavoritesBatch(token!, articleIds, effectiveDb),
    enabled: Boolean(token && user && effectiveDb && articleIds.length > 0),
  });
  const total = articleQuery.data?.page.total ?? null;
  const totalPages = total ? Math.max(1, Math.ceil(total / pageSize)) : Math.max(1, page);
  const visiblePages = getVisiblePageNumbers(page, totalPages);

  const filteredJournalOptions = journalOptions.filter((option) => {
    const label = option.title || option.journal_id;
    return label.toLowerCase().includes(journalSearch.trim().toLowerCase());
  });

  useEffect(() => {
    if (sortedArticles.length === 0) {
      setSelectedArticleId(null);
      return;
    }
    if (
      !selectedArticleId ||
      !sortedArticles.some((article) => article.article_id === selectedArticleId)
    ) {
      setSelectedArticleId(sortedArticles[0].article_id);
    }
  }, [selectedArticleId, sortedArticles]);

  const replaceParams = (updater: (params: URLSearchParams) => void) => {
    const nextParams = new URLSearchParams(searchParams.toString());
    updater(nextParams);
    router.replace(buildRoute(pathname, nextParams), { scroll: false });
  };

  const setRepeatedParam = (name: string, values: string[]) => {
    replaceParams((params) => {
      params.delete(name);
      for (const value of values) {
        params.append(name, value);
      }
    });
  };

  const submitSearch = (nextQuery = queryDraft) => {
    const scopedQuery = buildScopedQuery(searchScope, nextQuery);
    replaceParams((params) => {
      if (scopedQuery) {
        params.set('q', scopedQuery);
        return;
      }
      params.delete('q');
    });
    addHistoryEntry(scopedQuery);
    setIsDetailVisible(true);
  };

  /**
   * Apply the advanced modal payload to database state and URL filters.
   *
   * @param payload - Advanced search values.
   */
  const applyAdvancedSearch = (payload: AdvancedSearchPayload) => {
    replaceParams((params) => {
      if (payload.query) {
        params.set('q', payload.query);
      } else {
        params.delete('q');
      }
    });
    setQueryDraft(payload.query);
    addHistoryEntry(payload.query);
    setIsDetailVisible(true);
  };

  const clearFilters = () => {
    router.replace(pathname, { scroll: false });
    setQueryDraft('');
    setIsDetailVisible(true);
  };

  const selectYearPreset = (yearsBack: number | null) => {
    replaceParams((params) => {
      if (!yearsBack) {
        params.delete('year_min');
        params.delete('year_max');
        return;
      }
      const currentYear = new Date().getFullYear();
      params.set('year_min', String(currentYear - yearsBack + 1));
      params.set('year_max', String(currentYear));
    });
  };

  const selectDatabase = (dbName: string) => {
    selectDatabaseState(dbName);
    setIsDetailVisible(true);
  };

  const selectArticleByIndex = (nextIndex: number) => {
    const nextArticle = sortedArticles[nextIndex];
    if (nextArticle) {
      setSelectedArticleId(nextArticle.article_id);
      setIsDetailVisible(true);
    }
  };

  return (
    <>
      <ShellConfigurator
        title="文献检索工作台"
        sidebarExtra={
          <SearchHistoryPanel
            entries={historyEntries}
            onSelect={(historyQuery) => {
              setQueryDraft(historyQuery);
              submitSearch(historyQuery);
            }}
          />
        }
      />
      <AnnouncementsModal />
      {isAdvancedOpen ? (
        <AdvancedSearchModal
          open={isAdvancedOpen}
          query={query}
          onClose={() => setIsAdvancedOpen(false)}
          onApply={applyAdvancedSearch}
        />
      ) : null}
      <div className="workspace-grid workspace-grid--search">
        <Panel title="数据库">
          <div className="filter-stack">
            {databases.map((dbName) => (
              <CheckboxRow
                key={dbName}
                checked={effectiveDb === dbName}
                detail={
                  effectiveDb === dbName && typeof total === 'number' ? formatCount(total) : '可选'
                }
                label={getDatabaseLabel(dbName)}
                onChange={() => selectDatabase(dbName)}
              />
            ))}
            <Button
              size="small"
              variant="ghost"
              onClick={() => databases[0] && selectDatabase(databases[0])}
            >
              全部选择
            </Button>
          </div>
          <div className="filter-divider" />
          <Panel flush title="研究领域">
            <div className="filter-stack filter-stack--scroll">
              {areasPending ? (
                <>
                  <Skeleton className="h-8" />
                  <Skeleton className="h-8" />
                  <Skeleton className="h-8" />
                </>
              ) : (
                areaOptions.map((area) => (
                  <CheckboxRow
                    key={area.value}
                    checked={areas.includes(area.value)}
                    detail={formatCount(area.count)}
                    label={area.value}
                    onChange={(event) =>
                      setRepeatedParam(
                        'area',
                        toggleParamValue(areas, area.value, event.currentTarget.checked),
                      )
                    }
                  />
                ))
              )}
            </div>
          </Panel>
          <div className="filter-divider" />
          <Panel flush title="期刊检索">
            <div className="form-grid">
              <div className="search-field">
                <TextInput
                  value={journalSearch}
                  onChange={(event) => setJournalSearch(event.target.value)}
                  placeholder="输入期刊名称或ISSN"
                />
                <SearchIcon size={15} />
              </div>
              <div className="chip-list">
                {journalHistory.map((journal) => {
                  const journalId = String(journal.journal_id);
                  const selected = journalIds.includes(journalId);
                  return (
                    <button
                      key={journalId}
                      className={joinClassNames('chip', selected && 'chip--selected')}
                      type="button"
                      onClick={() => {
                        setRepeatedParam(
                          'journal_id',
                          toggleParamValue(journalIds, journalId, !selected),
                        );
                        if (!selected) {
                          addToJournalHistory({
                            journal_id: journal.journal_id,
                            title: journal.title,
                          });
                        }
                      }}
                    >
                      {journal.title || journalId}
                    </button>
                  );
                })}
              </div>
              {journalSearch.trim() ? (
                <div className="filter-stack filter-stack--scroll">
                  {journalsPending ? (
                    <Skeleton className="h-8" />
                  ) : (
                    filteredJournalOptions.slice(0, 40).map((journal) => {
                      const journalId = String(journal.journal_id);
                      const selected = journalIds.includes(journalId);
                      return (
                        <CheckboxRow
                          key={journalId}
                          checked={selected}
                          label={journal.title || journalId}
                          onChange={(event) => {
                            const isChecked = event.currentTarget.checked;
                            setRepeatedParam(
                              'journal_id',
                              toggleParamValue(journalIds, journalId, isChecked),
                            );
                            if (isChecked) {
                              addToJournalHistory({
                                journal_id: journal.journal_id,
                                title: journal.title || String(journal.journal_id),
                              });
                            }
                          }}
                        />
                      );
                    })
                  )}
                </div>
              ) : null}
            </div>
          </Panel>
          <div className="filter-divider" />
          <Panel flush title="发表年份">
            <div className="form-grid">
              <div className="year-range">
                <TextInput
                  type="number"
                  placeholder="起始"
                  value={yearMin ?? ''}
                  onChange={(event) =>
                    replaceParams((params) => {
                      const val = event.target.value.trim();
                      if (val) {
                        params.set('year_min', val);
                      } else {
                        params.delete('year_min');
                      }
                    })
                  }
                />
                <span>—</span>
                <TextInput
                  type="number"
                  placeholder="截止"
                  value={yearMax ?? ''}
                  onChange={(event) =>
                    replaceParams((params) => {
                      const val = event.target.value.trim();
                      if (val) {
                        params.set('year_max', val);
                      } else {
                        params.delete('year_max');
                      }
                    })
                  }
                />
              </div>
              <div className="chip-list">
                <button
                  className={joinClassNames(
                    'chip',
                    yearMin === new Date().getFullYear() &&
                      yearMax === new Date().getFullYear() &&
                      'chip--selected',
                  )}
                  type="button"
                  onClick={() => selectYearPreset(1)}
                >
                  近 1 年
                </button>
                <button
                  className={joinClassNames(
                    'chip',
                    yearMin === new Date().getFullYear() - 2 &&
                      yearMax === new Date().getFullYear() &&
                      'chip--selected',
                  )}
                  type="button"
                  onClick={() => selectYearPreset(3)}
                >
                  近 3 年
                </button>
                <button
                  className={joinClassNames(
                    'chip',
                    yearMin === new Date().getFullYear() - 4 &&
                      yearMax === new Date().getFullYear() &&
                      'chip--selected',
                  )}
                  type="button"
                  onClick={() => selectYearPreset(5)}
                >
                  近 5 年
                </button>
                <button
                  className={joinClassNames(
                    'chip',
                    !(
                      (yearMin === new Date().getFullYear() &&
                        yearMax === new Date().getFullYear()) ||
                      (yearMin === new Date().getFullYear() - 2 &&
                        yearMax === new Date().getFullYear()) ||
                      (yearMin === new Date().getFullYear() - 4 &&
                        yearMax === new Date().getFullYear())
                    ) && 'chip--selected',
                  )}
                  type="button"
                  onClick={() => selectYearPreset(null)}
                >
                  自定义
                </button>
              </div>
            </div>
          </Panel>
          <div style={{ marginTop: 12 }}>
            <Button icon={<RotateCcw size={15} />} onClick={clearFilters} variant="primary" wide>
              重置筛选条件
            </Button>
          </div>
        </Panel>

        <div className="center-column">
          <div className="search-command">
            <SelectInput
              value={searchScope}
              onChange={(event) => setSearchScope(event.target.value as SearchScope)}
            >
              <option value="topic">主题</option>
              <option value="title">标题</option>
              <option value="author">作者</option>
              <option value="doi">DOI</option>
            </SelectInput>
            <TextInput
              value={queryDraft}
              onChange={(event) => setQueryDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  submitSearch();
                }
              }}
              placeholder="large language models AND reasoning"
            />
            <Button
              icon={<SlidersHorizontal size={14} />}
              variant="ghost"
              onClick={() => setIsAdvancedOpen(true)}
            >
              高级检索
            </Button>
            <Button variant="primary" onClick={() => submitSearch()}>
              搜索
            </Button>
          </div>

          <Panel
            className="results-panel"
            title={
              typeof total === 'number'
                ? `检索结果 ${formatCount(total)} 条${
                    articleQuery.isFetching
                      ? ' (加载中)'
                      : searchDurationMs
                        ? ` (用时 ${(searchDurationMs / 1000).toFixed(2)} 秒)`
                        : ' (按相关性排序)'
                  }`
                : articleQuery.isFetching
                  ? '检索结果加载中'
                  : '检索结果'
            }
            actions={
              <>
                <span className="panel__meta">排序:</span>
                <SelectInput
                  value={sortMode}
                  onChange={(event) => setSortMode(event.target.value as SortMode)}
                >
                  <option value="date-desc">最新</option>
                  <option value="date-asc">最旧</option>
                </SelectInput>
                <IconButton
                  aria-label="列表视图"
                  className={viewMode === 'list' ? 'icon-btn--active' : ''}
                  title="列表视图"
                  onClick={() => setViewMode('list')}
                >
                  <LayoutList size={16} />
                </IconButton>
                <IconButton
                  aria-label="网格视图"
                  className={viewMode === 'grid' ? 'icon-btn--active' : ''}
                  title="网格视图"
                  onClick={() => setViewMode('grid')}
                >
                  <Grid2X2 size={16} />
                </IconButton>
              </>
            }
          >
            {articleQuery.isPending ? (
              <div className="list-stack">
                <Skeleton className="h-32" />
                <Skeleton className="h-32" />
                <Skeleton className="h-32" />
              </div>
            ) : articleQuery.isError ? (
              <Notice tone="error">{articleQuery.error.message}</Notice>
            ) : sortedArticles.length === 0 ? (
              <EmptyState>未找到文章。</EmptyState>
            ) : (
              <>
                <div
                  className={joinClassNames(
                    'result-list',
                    viewMode === 'grid' && 'result-list--grid',
                  )}
                >
                  {sortedArticles.map((article, index) => (
                    <ArticleCard
                      key={article.article_id}
                      article={article}
                      dbName={effectiveDb}
                      favoriteFolderIds={
                        favoriteQuery.data?.[article.article_id]?.map((item) => item.folder_id) ??
                        []
                      }
                      favoritePending={favoriteQuery.isPending}
                      leading={<span className="result-rank">{articleOffset + index + 1}</span>}
                      selected={selectedArticle?.article_id === article.article_id}
                      onSelect={(nextArticle) => {
                        setSelectedArticleId(nextArticle.article_id);
                        setIsDetailVisible(true);
                      }}
                      preview={renderHighlightedText(article.abstract, query)}
                      title={renderHighlightedText(article.title, query)}
                    />
                  ))}
                </div>
                <div className="pagination-bar">
                  <IconButton
                    aria-label="上一页"
                    disabled={page <= 1}
                    title="上一页"
                    onClick={() => setPage((currentPage) => Math.max(1, currentPage - 1))}
                  >
                    <ChevronLeft size={16} />
                  </IconButton>
                  {visiblePages.map((pageNumber, index) => (
                    <button
                      key={pageNumber}
                      className={joinClassNames(
                        'page-button',
                        pageNumber === page && 'page-button--active',
                      )}
                      type="button"
                      onClick={() => setPage(pageNumber)}
                    >
                      {index > 0 && pageNumber - visiblePages[index - 1] > 1 ? '...' : pageNumber}
                    </button>
                  ))}
                  <IconButton
                    aria-label="下一页"
                    disabled={Boolean(total && page >= totalPages)}
                    title="下一页"
                    onClick={() => setPage((currentPage) => currentPage + 1)}
                  >
                    <ChevronRight size={16} />
                  </IconButton>
                  <SelectInput
                    value={pageSize}
                    onChange={(event) => setPageSize(Number(event.target.value))}
                  >
                    {PAGE_SIZE_OPTIONS.map((option) => (
                      <option key={option} value={option}>
                        {option} 条/页
                      </option>
                    ))}
                  </SelectInput>
                </div>
              </>
            )}
          </Panel>
        </div>

        <div className="right-column">
          <ArticleDetailPanel
            article={selectedArticle}
            dbName={effectiveDb}
            favoriteFolderIds={
              selectedArticle
                ? (favoriteQuery.data?.[selectedArticle.article_id]?.map(
                    (item) => item.folder_id,
                  ) ?? [])
                : []
            }
            favoritePending={favoriteQuery.isPending}
            previousDisabled={selectedIndex <= 0}
            nextDisabled={selectedIndex < 0 || selectedIndex >= sortedArticles.length - 1}
            onPrevious={() => selectArticleByIndex(selectedIndex - 1)}
            onNext={() => selectArticleByIndex(selectedIndex + 1)}
            onClose={() => setIsDetailVisible(false)}
          />
        </div>
      </div>
    </>
  );
}
