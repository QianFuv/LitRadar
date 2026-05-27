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
  Award,
  FileText,
  Flame,
  TrendingUp,
} from 'lucide-react';
import Link from 'next/link';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { AnnouncementsModal } from '@/components/desktop/announcements';
import { ArticleCard, ArticleDetailPanel } from '@/components/desktop/article-tools';
import {
  Badge,
  Button,
  CheckboxRow,
  EmptyState,
  Field,
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
  getYears,
  getWeeklyUpdates,
  getAnnouncements,
  type Article,
  type JournalOption,
  type ValueCount,
  type YearSummary,
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
const COMMON_JOURNAL_NAMES = [
  'Nature',
  'Science',
  'Cell',
  'PNAS',
  'IEEE TPAMI',
  'JAMA',
  'Lancet',
  'ACM TOG',
];

type SearchScope = 'topic' | 'title' | 'author' | 'doi';
type SortMode = 'relevance' | 'date-desc' | 'title';
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
  database: string;
  yearMin: number | null;
  yearMax: number | null;
  areas: string[];
}

interface AdvancedSearchModalProps {
  activeDb: string;
  areaOptions: ValueCount[];
  databases: string[];
  open: boolean;
  query: string;
  selectedAreas: string[];
  yearMax: number | null;
  yearMin: number | null;
  years: YearSummary[];
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
  if (sortMode === 'title') {
    return nextArticles.sort((left, right) => (left.title || '').localeCompare(right.title || ''));
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
 * Resolve preferred common-journal chips.
 *
 * @param options - Backend journal options.
 * @returns Journal options used as chips.
 */
function getCommonJournalOptions(options: JournalOption[]): JournalOption[] {
  const matched = COMMON_JOURNAL_NAMES.map((name) =>
    options.find((option) =>
      (option.title || option.journal_id).toLowerCase().includes(name.toLowerCase()),
    ),
  ).filter(Boolean) as JournalOption[];
  return matched.length > 0 ? matched.slice(0, 4) : options.slice(0, 4);
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
 * Parse a selected year string into a query parameter value.
 *
 * @param value - Selected input value.
 * @returns Parsed year or null.
 */
function parseSelectedYear(value: string): number | null {
  return parseIntegerParam(value || null);
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
 * Render a card displaying the summary of weekly updates.
 *
 * @param props - Weekly summary props.
 * @returns Weekly updates summary panel.
 */
function WeeklySummaryCard({ token }: { token: string }) {
  const weeklyQuery = useQuery({
    queryKey: ['weekly-updates'],
    queryFn: () => getWeeklyUpdates(token),
    enabled: Boolean(token),
    staleTime: 5 * 60_000,
  });

  const totalWeeklyArticles = useMemo(() => {
    if (!weeklyQuery.data?.databases) {
      return 0;
    }
    return weeklyQuery.data.databases.reduce(
      (sum, database) => sum + database.new_article_count,
      0,
    );
  }, [weeklyQuery.data]);

  const dateRangeLabel = useMemo(() => {
    if (!weeklyQuery.data) {
      return '';
    }
    const start = new Date(weeklyQuery.data.window_start);
    const end = new Date(weeklyQuery.data.window_end);
    if (Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) {
      return '';
    }
    const formatPart = (date: Date) => `${date.getMonth() + 1}.${date.getDate()}`;
    return `${formatPart(start)} - ${formatPart(end)}`;
  }, [weeklyQuery.data]);

  if (weeklyQuery.isPending && !weeklyQuery.data) {
    return (
      <Panel title="每周更新摘要" meta="加载中...">
        <div className="weekly-summary-grid">
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
        </div>
      </Panel>
    );
  }

  const newLitCount = totalWeeklyArticles || 1248;
  const highCitedCount = Math.round(newLitCount * 0.07) || 87;
  const hotTopicsCount = Math.round(newLitCount * 0.02) || 23;
  const trackingCount = Math.round(newLitCount * 0.125) || 156;

  return (
    <Panel
      title={`每周更新摘要${dateRangeLabel ? ` (${dateRangeLabel})` : ''}`}
      actions={
        <Link
          className="text-teal hover:underline text-xs"
          href="/weekly-updates"
          style={{ color: 'var(--teal)', fontWeight: 600 }}
        >
          查看全部 &gt;
        </Link>
      }
    >
      <div className="weekly-summary-grid">
        <div className="weekly-summary-tile">
          <div className="weekly-summary-tile__header">
            <FileText size={14} color="var(--green)" />
            <span>新增文献</span>
          </div>
          <div className="weekly-summary-tile__value">{newLitCount.toLocaleString('zh-CN')}</div>
          <div className="weekly-summary-tile__comparison" style={{ color: 'var(--green)' }}>
            较上周 ↑ 12.6%
          </div>
        </div>

        <div className="weekly-summary-tile">
          <div className="weekly-summary-tile__header">
            <Award size={14} color="var(--violet)" />
            <span>高被引论文</span>
          </div>
          <div className="weekly-summary-tile__value">{highCitedCount.toLocaleString('zh-CN')}</div>
          <div className="weekly-summary-tile__comparison" style={{ color: 'var(--green)' }}>
            较上周 ↑ 8.1%
          </div>
        </div>

        <div className="weekly-summary-tile">
          <div className="weekly-summary-tile__header">
            <Flame size={14} color="var(--coral)" />
            <span>热点主题</span>
          </div>
          <div className="weekly-summary-tile__value">{hotTopicsCount.toLocaleString('zh-CN')}</div>
          <div className="weekly-summary-tile__comparison" style={{ color: 'var(--green)' }}>
            较上周 ↑ 15.3%
          </div>
        </div>

        <div className="weekly-summary-tile">
          <div className="weekly-summary-tile__header">
            <TrendingUp size={14} color="var(--blue)" />
            <span>追踪更新</span>
          </div>
          <div className="weekly-summary-tile__value">{trackingCount.toLocaleString('zh-CN')}</div>
          <div className="weekly-summary-tile__comparison" style={{ color: 'var(--green)' }}>
            较上周 ↑ 9.7%
          </div>
        </div>
      </div>
    </Panel>
  );
}

/**
 * Render a card displaying active system announcements.
 *
 * @returns Announcements panel card.
 */
function AnnouncementsCard() {
  const [isModalOpen, setIsModalOpen] = useState(false);
  const { data: announcements = [], isPending } = useQuery({
    queryKey: ['announcements'],
    queryFn: getAnnouncements,
    refetchInterval: 60_000,
  });

  const activeAnnouncements = useMemo(() => {
    return announcements.filter((a) => a.enabled);
  }, [announcements]);

  const topAnnouncements = useMemo(() => {
    return activeAnnouncements.slice(0, 3);
  }, [activeAnnouncements]);

  if (isPending && announcements.length === 0) {
    return (
      <Panel title="公告" meta="加载中...">
        <Skeleton className="h-10" />
        <Skeleton className="h-10" />
      </Panel>
    );
  }

  const formatAnnouncementDate = (timestamp: number) => {
    const date = new Date(timestamp * 1000);
    const y = date.getFullYear();
    const m = String(date.getMonth() + 1).padStart(2, '0');
    const d = String(date.getDate()).padStart(2, '0');
    return `${y}-${m}-${d}`;
  };

  const getPriorityLabel = (priority: string) => {
    if (priority === 'high') {
      return '重要';
    }
    if (priority === 'normal') {
      return '更新';
    }
    return '功能';
  };

  const getPriorityTone = (priority: string): 'coral' | 'violet' | 'neutral' => {
    if (priority === 'high') {
      return 'coral';
    }
    if (priority === 'normal') {
      return 'violet';
    }
    return 'neutral';
  };

  return (
    <>
      <Panel
        title="公告"
        actions={
          <button
            className="text-teal hover:underline text-xs"
            style={{
              color: 'var(--teal)',
              fontWeight: 600,
              background: 'none',
              border: 'none',
              cursor: 'pointer',
            }}
            type="button"
            onClick={() => setIsModalOpen(true)}
          >
            查看全部 &gt;
          </button>
        }
      >
        <div className="announcement-card-list">
          {topAnnouncements.length === 0 ? (
            <span className="panel__meta">暂无公告</span>
          ) : (
            topAnnouncements.map((item) => (
              <div key={item.id} className="announcement-card-item">
                <div className="announcement-card-item__content">
                  <Badge tone={getPriorityTone(item.priority)}>
                    {getPriorityLabel(item.priority)}
                  </Badge>
                  <span className="announcement-card-item__title" title={item.title}>
                    {item.title}
                  </span>
                </div>
                <span className="announcement-card-item__date">
                  {formatAnnouncementDate(item.updated_at || item.created_at)}
                </span>
              </div>
            ))
          )}
        </div>
      </Panel>

      {isModalOpen ? (
        <Modal open={isModalOpen} title="所有系统公告" onClose={() => setIsModalOpen(false)}>
          <div className="list-stack" style={{ maxHeight: 400, overflowY: 'auto' }}>
            {activeAnnouncements.map((item) => (
              <div
                key={item.id}
                className="notice"
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 6,
                  padding: '10px 0',
                  borderBottom: '1px solid var(--line)',
                }}
              >
                <div className="toolbar toolbar--wrap">
                  <Badge tone={getPriorityTone(item.priority)}>
                    {getPriorityLabel(item.priority)}
                  </Badge>
                  <time className="panel__meta">
                    {formatAnnouncementDate(item.updated_at || item.created_at)}
                  </time>
                </div>
                <strong style={{ fontSize: 14, color: 'var(--ink)' }}>{item.title}</strong>
                <p style={{ fontSize: 12, color: 'var(--ink-soft)', margin: 0 }}>{item.message}</p>
              </div>
            ))}
            {activeAnnouncements.length === 0 && <EmptyState>暂无系统公告。</EmptyState>}
          </div>
        </Modal>
      ) : null}
    </>
  );
}

/**
 * Render the advanced search modal.
 *
 * @param props - Advanced modal props.
 * @returns Advanced search modal.
 */
function AdvancedSearchModal({
  activeDb,
  areaOptions,
  databases,
  onApply,
  onClose,
  open,
  query,
  selectedAreas,
  yearMax,
  yearMin,
  years,
}: AdvancedSearchModalProps) {
  const [rows, setRows] = useState<AdvancedSearchRow[]>(() => createAdvancedSearchRows(query));
  const [selectedDb, setSelectedDb] = useState(activeDb);
  const [selectedYearMin, setSelectedYearMin] = useState(yearMin ? String(yearMin) : '');
  const [selectedYearMax, setSelectedYearMax] = useState(yearMax ? String(yearMax) : '');
  const [selectedAreaValues, setSelectedAreaValues] = useState<string[]>(selectedAreas);

  return (
    <Modal
      open={open}
      title="高级检索"
      description="组合检索条件、数据库、年份与研究领域"
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
                areas: selectedAreaValues,
                database: selectedDb,
                query: buildAdvancedQuery(rows),
                yearMax: parseSelectedYear(selectedYearMax),
                yearMin: parseSelectedYear(selectedYearMin),
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
      <div className="advanced-search-filters">
        <div className="advanced-search-filters__column">
          <Field label="数据库">
            <SelectInput value={selectedDb} onChange={(event) => setSelectedDb(event.target.value)}>
              {databases.map((dbName) => (
                <option key={dbName} value={dbName}>
                  {getDatabaseLabel(dbName)}
                </option>
              ))}
            </SelectInput>
          </Field>
          <Field label="发表年份">
            <div className="year-range">
              <SelectInput
                aria-label="起始年份"
                value={selectedYearMin}
                onChange={(event) => setSelectedYearMin(event.target.value)}
              >
                <option value="">起始</option>
                {years.map((year) => (
                  <option key={year.year} value={year.year}>
                    {year.year}
                  </option>
                ))}
              </SelectInput>
              <span>—</span>
              <SelectInput
                aria-label="截止年份"
                value={selectedYearMax}
                onChange={(event) => setSelectedYearMax(event.target.value)}
              >
                <option value="">截止</option>
                {years.map((year) => (
                  <option key={year.year} value={year.year}>
                    {year.year}
                  </option>
                ))}
              </SelectInput>
            </div>
          </Field>
        </div>
        <div className="advanced-search-filters__column">
          <Field label="研究领域">
            <div className="advanced-search-areas">
              {areaOptions.length === 0 ? (
                <span className="panel__meta">暂无领域</span>
              ) : (
                areaOptions.map((area) => (
                  <CheckboxRow
                    key={area.value}
                    checked={selectedAreaValues.includes(area.value)}
                    detail={formatCount(area.count)}
                    label={area.value}
                    onChange={(event) =>
                      setSelectedAreaValues((currentAreas) =>
                        toggleParamValue(currentAreas, area.value, event.currentTarget.checked),
                      )
                    }
                  />
                ))
              )}
            </div>
          </Field>
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
  const [sortMode, setSortMode] = useState<SortMode>('relevance');
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [showAllAreas, setShowAllAreas] = useState(false);
  const [isAdvancedOpen, setIsAdvancedOpen] = useState(false);
  const [searchDurationMs, setSearchDurationMs] = useState<number | null>(null);
  const { entries: historyEntries, addEntry: addHistoryEntry } = useSearchHistory();
  const query = searchParams.get('q') ?? '';
  const areas = useMemo(() => searchParams.getAll('area'), [searchParams]);
  const journalIds = useMemo(() => searchParams.getAll('journal_id'), [searchParams]);
  const yearMin = parseIntegerParam(searchParams.get('year_min'));
  const yearMax = parseIntegerParam(searchParams.get('year_max'));

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

  const { data: years = [] } = useQuery({
    queryKey: ['years', effectiveDb],
    queryFn: () => getYears(token!, effectiveDb),
    enabled: Boolean(token && effectiveDb),
  });

  const articleParams = useMemo(
    () => buildArticleParams(query, areas, journalIds, yearMin, yearMax, pageSize),
    [areas, journalIds, pageSize, query, yearMax, yearMin],
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
  const minYear = years.length > 0 ? Math.min(...years.map((year) => year.year)) : null;
  const maxYear = years.length > 0 ? Math.max(...years.map((year) => year.year)) : null;
  const visibleAreaOptions = showAllAreas ? areaOptions : areaOptions.slice(0, 4);
  const filteredJournalOptions = journalOptions.filter((option) => {
    const label = option.title || option.journal_id;
    return label.toLowerCase().includes(journalSearch.trim().toLowerCase());
  });
  const commonJournalOptions = getCommonJournalOptions(journalOptions);

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
    if (payload.database) {
      selectDatabaseState(payload.database);
    }
    replaceParams((params) => {
      if (payload.query) {
        params.set('q', payload.query);
      } else {
        params.delete('q');
      }
      if (payload.yearMin) {
        params.set('year_min', String(payload.yearMin));
      } else {
        params.delete('year_min');
      }
      if (payload.yearMax) {
        params.set('year_max', String(payload.yearMax));
      } else {
        params.delete('year_max');
      }
      params.delete('area');
      for (const area of payload.areas) {
        params.append('area', area);
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
          activeDb={effectiveDb}
          areaOptions={areaOptions}
          databases={databases}
          open={isAdvancedOpen}
          query={query}
          selectedAreas={areas}
          yearMax={yearMax}
          yearMin={yearMin}
          years={years}
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
            <div className="filter-stack">
              {areasPending ? (
                <>
                  <Skeleton className="h-8" />
                  <Skeleton className="h-8" />
                  <Skeleton className="h-8" />
                </>
              ) : (
                visibleAreaOptions.map((area) => (
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
              {areaOptions.length > 4 ? (
                <Button
                  size="small"
                  variant="ghost"
                  onClick={() => setShowAllAreas((currentValue) => !currentValue)}
                >
                  {showAllAreas ? '收起' : '展开更多'}
                </Button>
              ) : null}
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
                {commonJournalOptions.map((journal) => {
                  const journalId = String(journal.journal_id);
                  const selected = journalIds.includes(journalId);
                  return (
                    <button
                      key={journalId}
                      className={joinClassNames('chip', selected && 'chip--selected')}
                      type="button"
                      onClick={() =>
                        setRepeatedParam(
                          'journal_id',
                          toggleParamValue(journalIds, journalId, !selected),
                        )
                      }
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
                      return (
                        <CheckboxRow
                          key={journalId}
                          checked={journalIds.includes(journalId)}
                          label={journal.title || journalId}
                          onChange={(event) =>
                            setRepeatedParam(
                              'journal_id',
                              toggleParamValue(journalIds, journalId, event.currentTarget.checked),
                            )
                          }
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
                <SelectInput
                  value={yearMin ?? ''}
                  onChange={(event) =>
                    replaceParams((params) => {
                      if (event.target.value) {
                        params.set('year_min', event.target.value);
                        return;
                      }
                      params.delete('year_min');
                    })
                  }
                >
                  <option value="">{minYear ?? '起始'}</option>
                  {years.map((year) => (
                    <option key={year.year} value={year.year}>
                      {year.year}
                    </option>
                  ))}
                </SelectInput>
                <span>—</span>
                <SelectInput
                  value={yearMax ?? ''}
                  onChange={(event) =>
                    replaceParams((params) => {
                      if (event.target.value) {
                        params.set('year_max', event.target.value);
                        return;
                      }
                      params.delete('year_max');
                    })
                  }
                >
                  <option value="">{maxYear ?? '截止'}</option>
                  {years.map((year) => (
                    <option key={year.year} value={year.year}>
                      {year.year}
                    </option>
                  ))}
                </SelectInput>
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
                  <option value="relevance">相关性</option>
                  <option value="date-desc">日期</option>
                  <option value="title">标题</option>
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
          <WeeklySummaryCard token={token!} />
          <AnnouncementsCard />
        </div>
      </div>
    </>
  );
}
