'use client';

import { useQuery } from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf } from 'nuqs';
import Image from 'next/image';
import { useTheme } from 'next-themes';
import {
  getAreas,
  getYears,
  getJournalOptions,
  getCurrentDatabase,
  getDatabases,
  setDatabase,
} from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import { Checkbox } from '@/components/ui/checkbox';
import { Label } from '@/components/ui/label';
import { Skeleton } from '@/components/ui/skeleton';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { ScrollArea } from '@/components/ui/scroll-area';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { CalendarDays, Moon, Sun, Database } from 'lucide-react';
import { getAreaDisplayName } from '@/lib/area-labels';
import { cn } from '@/lib/utils';
import { useEffect, useMemo, useState } from 'react';

const MONTH_VALUES = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] as const;
const MONTH_KEY_PATTERN = /^\d{4}-(0[1-9]|1[0-2])$/;
const MONTH_RANGE_SEPARATOR = '..';

interface MonthPickerProps {
  label: string;
  value: string;
  minYear: number;
  maxYear: number;
  onChange: (value: string) => void;
}

/**
 * Build a stable YYYY-MM key for query state and date conversion.
 *
 * @param year - Four digit year.
 * @param month - One-based month number.
 * @returns Month key.
 */
function buildMonthKey(year: number, month: number): string {
  return `${year}-${String(month).padStart(2, '0')}`;
}

/**
 * Check whether a query value is a supported YYYY-MM month key.
 *
 * @param value - Query value to inspect.
 * @returns True when the value can be used as a month key.
 */
function isMonthKey(value: string | null): value is string {
  return typeof value === 'string' && MONTH_KEY_PATTERN.test(value);
}

/**
 * Return the year component from a month key.
 *
 * @param value - Month key.
 * @param fallback - Year to use when parsing fails.
 * @returns Parsed year or fallback.
 */
function monthKeyYear(value: string, fallback: number): number {
  const year = Number(value.slice(0, 4));
  return Number.isFinite(year) ? year : fallback;
}

/**
 * Normalize a month key into the available year range.
 *
 * @param value - Raw month key.
 * @param minYear - Earliest available year.
 * @param maxYear - Latest available year.
 * @returns Clamped month key or null when invalid.
 */
function normalizeMonthKey(value: string | null, minYear: number, maxYear: number): string | null {
  if (!isMonthKey(value)) {
    return null;
  }
  const year = monthKeyYear(value, minYear);
  if (year < minYear) {
    return buildMonthKey(minYear, 1);
  }
  if (year > maxYear) {
    return buildMonthKey(maxYear, 12);
  }
  return value;
}

/**
 * Parse the compact month range query value.
 *
 * @param value - Raw query value in YYYY-MM..YYYY-MM format.
 * @param minYear - Earliest available year.
 * @param maxYear - Latest available year.
 * @param defaultStartMonth - Default start month.
 * @param defaultEndMonth - Default end month.
 * @returns Ordered start and end month keys.
 */
function parseMonthRange(
  value: string | null,
  minYear: number,
  maxYear: number,
  defaultStartMonth: string,
  defaultEndMonth: string,
): [string, string] {
  const [rawStartMonth = '', rawEndMonth = ''] = (value ?? '').split(MONTH_RANGE_SEPARATOR);
  const startMonth = normalizeMonthKey(rawStartMonth, minYear, maxYear) ?? defaultStartMonth;
  const endMonth = normalizeMonthKey(rawEndMonth, minYear, maxYear) ?? defaultEndMonth;
  return startMonth <= endMonth ? [startMonth, endMonth] : [endMonth, startMonth];
}

/**
 * Build the compact month range query value.
 *
 * @param startMonth - Start month key.
 * @param endMonth - End month key.
 * @returns Query value in YYYY-MM..YYYY-MM format.
 */
function buildMonthRange(startMonth: string, endMonth: string): string {
  return `${startMonth}${MONTH_RANGE_SEPARATOR}${endMonth}`;
}

/**
 * Format a month key for the Chinese filter UI.
 *
 * @param value - Month key.
 * @returns Human-readable year-month label.
 */
function formatMonthLabel(value: string): string {
  return `${value.slice(0, 4)}年${value.slice(5, 7)}月`;
}

/**
 * Render a popover selector for one month bound.
 *
 * @param props - Month picker configuration.
 * @returns Month picker UI.
 */
function MonthPicker({ label, value, minYear, maxYear, onChange }: MonthPickerProps) {
  const initialYear = monthKeyYear(value, maxYear);
  const [open, setOpen] = useState(false);
  const [activeYear, setActiveYear] = useState(initialYear);
  const years = useMemo(() => {
    const result: number[] = [];
    for (let year = maxYear; year >= minYear; year -= 1) {
      result.push(year);
    }
    return result;
  }, [maxYear, minYear]);

  useEffect(() => {
    setActiveYear(monthKeyYear(value, maxYear));
  }, [maxYear, value]);

  const handleMonthClick = (month: number) => {
    onChange(buildMonthKey(activeYear, month));
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          className="h-auto min-h-12 w-full justify-start gap-2 px-3 py-2"
          title={`${label}：${formatMonthLabel(value)}`}
        >
          <CalendarDays className="h-4 w-4 text-muted-foreground" />
          <span className="flex min-w-0 flex-col items-start">
            <span className="text-[11px] leading-4 text-muted-foreground">{label}</span>
            <span className="truncate text-sm">{formatMonthLabel(value)}</span>
          </span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[17rem] max-w-[calc(100vw-2rem)] p-2">
        <div className="grid grid-cols-[4.75rem_1fr] gap-2">
          <ScrollArea className="h-56 pr-1">
            <div className="space-y-1">
              {years.map((year) => (
                <Button
                  key={year}
                  type="button"
                  variant={year === activeYear ? 'secondary' : 'ghost'}
                  size="sm"
                  className="h-8 w-full justify-center px-2"
                  onClick={() => setActiveYear(year)}
                >
                  {year}
                </Button>
              ))}
            </div>
          </ScrollArea>
          <div className="space-y-2">
            <div className="px-1 text-sm font-medium">{activeYear}年</div>
            <div className="grid grid-cols-3 gap-1">
              {MONTH_VALUES.map((month) => {
                const monthKey = buildMonthKey(activeYear, month);
                return (
                  <Button
                    key={month}
                    type="button"
                    variant={monthKey === value ? 'default' : 'ghost'}
                    size="sm"
                    className="h-9 px-2"
                    onClick={() => handleMonthClick(month)}
                  >
                    {String(month).padStart(2, '0')}月
                  </Button>
                );
              })}
            </div>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function Sidebar({ className }: { className?: string }) {
  const { theme, setTheme } = useTheme();
  const { token } = useAuth();

  const [selectedDb, setSelectedDb] = useState(getCurrentDatabase());
  const [, setQ] = useQueryState('q', parseAsString);
  const [areas, setAreas] = useQueryState('area', parseAsArrayOf(parseAsString).withDefault([]));
  const [journalIds, setJournalIds] = useQueryState(
    'journal_id',
    parseAsArrayOf(parseAsString).withDefault([]),
  );
  const [monthRange, setMonthRange] = useQueryState('month_range', parseAsString);

  const { data: databases, isLoading: loadingDatabases } = useQuery({
    queryKey: ['meta', 'databases'],
    queryFn: () => getDatabases(token!),
    enabled: !!token,
  });
  const activeDb =
    databases && databases.length > 0
      ? databases.includes(selectedDb)
        ? selectedDb
        : databases[0]
      : selectedDb;

  useEffect(() => {
    if (!databases || databases.length === 0) {
      return;
    }
    if (activeDb === getCurrentDatabase()) {
      return;
    }
    setDatabase(activeDb);
  }, [activeDb, databases]);

  const { data: areaOptions, isLoading: loadingAreas } = useQuery({
    queryKey: ['meta', 'areas', activeDb],
    queryFn: () => getAreas(token!),
    enabled: !!token,
  });

  const { data: journalOptions, isLoading: loadingJournals } = useQuery({
    queryKey: ['meta', 'journals', activeDb],
    queryFn: () => getJournalOptions(token!),
    enabled: !!token,
  });

  const { data: yearData, isLoading: loadingYears } = useQuery({
    queryKey: ['meta', 'years', activeDb],
    queryFn: () => getYears(token!),
    enabled: !!token,
  });

  const handleDatabaseChange = (dbName: string) => {
    setDatabase(dbName);
    setSelectedDb(dbName);
    window.location.href = window.location.pathname;
  };

  const handleClearFilters = () => {
    setQ(null);
    setAreas([]);
    setJournalIds([]);
    setMonthRange(null);
  };

  const minYearAvailable =
    yearData && yearData.length > 0 ? Math.min(...yearData.map((y) => y.year)) : 1900;
  const maxYearAvailable =
    yearData && yearData.length > 0
      ? Math.max(...yearData.map((y) => y.year))
      : new Date().getFullYear();

  const defaultStartMonth = buildMonthKey(minYearAvailable, 1);
  const defaultEndMonth = buildMonthKey(maxYearAvailable, 12);
  const [selectedStartMonth, selectedEndMonth] = parseMonthRange(
    monthRange,
    minYearAvailable,
    maxYearAvailable,
    defaultStartMonth,
    defaultEndMonth,
  );

  const handleAreaChange = (value: string, checked: boolean) => {
    setAreas((current) => {
      if (checked) {
        return current.includes(value) ? current : [...current, value];
      }
      return current.filter((item) => item !== value);
    });
  };

  const handleJournalChange = (value: string, checked: boolean) => {
    setJournalIds((current) => {
      if (checked) {
        return current.includes(value) ? current : [...current, value];
      }
      return current.filter((item) => item !== value);
    });
  };

  const handleMonthRangeCommit = (startMonth: string, endMonth: string) => {
    const orderedStartMonth = startMonth <= endMonth ? startMonth : endMonth;
    const orderedEndMonth = startMonth <= endMonth ? endMonth : startMonth;
    setMonthRange(
      orderedStartMonth === defaultStartMonth && orderedEndMonth === defaultEndMonth
        ? null
        : buildMonthRange(orderedStartMonth, orderedEndMonth),
    );
  };

  const [journalSearch, setJournalSearch] = useState('');

  const filteredJournalOptions = useMemo(() => {
    if (!journalOptions) {
      return [];
    }
    const query = journalSearch.trim().toLowerCase();
    const matchedOptions = query
      ? journalOptions.filter((option) => {
          const title = option.title ?? '';
          return title.toLowerCase().includes(query);
        })
      : journalOptions;
    if (journalIds.length === 0) {
      return matchedOptions;
    }
    const selectedIds = new Set(journalIds);
    const selectedOptions = matchedOptions.filter((option) =>
      selectedIds.has(String(option.journal_id)),
    );
    const unselectedOptions = matchedOptions.filter(
      (option) => !selectedIds.has(String(option.journal_id)),
    );
    return [...selectedOptions, ...unselectedOptions];
  }, [journalIds, journalOptions, journalSearch]);

  const journalLabelMap = useMemo(() => {
    const map = new Map<string, string>();
    journalOptions?.forEach((option) => {
      map.set(String(option.journal_id), option.title ?? String(option.journal_id));
    });
    return map;
  }, [journalOptions]);

  const selectedJournalLabels = useMemo(() => {
    return journalIds.map((id) => journalLabelMap.get(id) ?? id);
  }, [journalIds, journalLabelMap]);

  const journalSummary =
    selectedJournalLabels.length === 0
      ? '全部期刊'
      : selectedJournalLabels.length === 1
        ? selectedJournalLabels[0]
        : `已选 ${selectedJournalLabels.length} 本期刊`;

  return (
    <aside
      className={cn('w-[19.2rem] min-w-0 flex flex-col h-full border-r bg-background', className)}
    >
      <div className="flex-1 space-y-8 p-6 overflow-y-auto">
        <div className="space-y-4">
          <div className="grid grid-cols-2 items-center gap-4">
            <div className="flex items-center justify-center">
              <Button
                variant="ghost"
                size="icon"
                onClick={handleClearFilters}
                aria-label="清空全部筛选"
                title="清空全部筛选"
                className="h-20 w-20"
              >
                <Image
                  src="https://cdn.sa.net/2026/01/29/6uRXpHqQfC89kF7.png"
                  alt="首页"
                  width={64}
                  height={64}
                  loading="eager"
                  fetchPriority="high"
                  className="h-16 w-16 object-contain"
                />
              </Button>
            </div>
            <div className="space-y-2 self-center">
              <div className="flex items-center gap-2 text-sm font-semibold text-foreground w-full">
                <Database className="h-4 w-4" />
                <span>数据库</span>
              </div>
              <div className="w-full">
                {loadingDatabases ? (
                  <Skeleton className="h-9 w-full" />
                ) : (
                  <Select value={activeDb} onValueChange={handleDatabaseChange}>
                    <SelectTrigger size="sm" className="w-full">
                      <SelectValue placeholder="选择数据库" />
                    </SelectTrigger>
                    <SelectContent>
                      {databases?.map((dbName) => (
                        <SelectItem key={dbName} value={dbName}>
                          {dbName.replace('.sqlite', '')}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
              </div>
            </div>
          </div>
        </div>

        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-sm text-foreground">期刊筛选</h3>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleClearFilters}
              className="h-6 px-2 text-xs"
              title="清空全部筛选"
            >
              清空
            </Button>
          </div>

          <div className="space-y-3">
            <h4 className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
              领域
            </h4>
            {loadingAreas ? (
              <div className="space-y-2">
                <Skeleton className="h-4 w-full" />
                <Skeleton className="h-4 w-3/4" />
              </div>
            ) : (
              <div className="space-y-2">
                {areaOptions?.map((opt) => {
                  const displayName = getAreaDisplayName(opt.value);
                  return (
                    <div key={opt.value} className="flex min-w-0 items-start gap-2">
                      <Checkbox
                        id={`area-${opt.value}`}
                        className="mt-0.5 shrink-0"
                        checked={areas.includes(opt.value)}
                        onCheckedChange={(checked: boolean | 'indeterminate') =>
                          handleAreaChange(opt.value, checked as boolean)
                        }
                      />
                      <Label
                        htmlFor={`area-${opt.value}`}
                        className="min-w-0 flex-1 cursor-pointer break-words text-sm leading-snug font-normal whitespace-normal"
                        title={opt.value}
                      >
                        {displayName}
                      </Label>
                      <span className="shrink-0 text-xs text-muted-foreground">{opt.count}</span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          <div className="space-y-3">
            <h4 className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
              期刊
            </h4>
            {loadingJournals ? (
              <Skeleton className="h-8 w-full" />
            ) : (
              <Popover>
                <PopoverTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    className="w-full justify-between"
                    title={journalSummary}
                  >
                    <span className="truncate">{journalSummary}</span>
                    {journalIds.length > 0 && (
                      <span className="text-xs text-muted-foreground">{journalIds.length}</span>
                    )}
                  </Button>
                </PopoverTrigger>
                <PopoverContent
                  align="start"
                  className="w-[var(--radix-popover-trigger-width)] max-w-[calc(100vw-2rem)] p-3"
                >
                  <Input
                    value={journalSearch}
                    onChange={(event) => setJournalSearch(event.target.value)}
                    placeholder="搜索期刊"
                    className="h-8 text-sm"
                  />
                  <ScrollArea className="mt-2 h-60 touch-pan-y">
                    <div className="space-y-2">
                      {filteredJournalOptions.map((option) => {
                        const id = String(option.journal_id);
                        return (
                          <div key={id} className="flex min-w-0 items-start gap-2">
                            <Checkbox
                              id={`journal-${id}`}
                              className="mt-0.5 shrink-0"
                              checked={journalIds.includes(id)}
                              onCheckedChange={(checked: boolean | 'indeterminate') =>
                                handleJournalChange(id, checked as boolean)
                              }
                            />
                            <Label
                              htmlFor={`journal-${id}`}
                              className="min-w-0 flex-1 cursor-pointer break-words text-sm leading-snug font-normal whitespace-normal"
                              title={option.title ?? id}
                            >
                              {option.title ?? id}
                            </Label>
                          </div>
                        );
                      })}
                      {filteredJournalOptions.length === 0 && (
                        <div className="text-xs text-muted-foreground">未找到期刊。</div>
                      )}
                    </div>
                  </ScrollArea>
                </PopoverContent>
              </Popover>
            )}
          </div>
        </div>

        <div className="space-y-4">
          <h3 className="font-semibold text-sm text-foreground">发表时间</h3>
          {loadingYears ? (
            <Skeleton className="h-8 w-full" />
          ) : (
            <div className="space-y-3">
              <div className="grid grid-cols-2 gap-2">
                <MonthPicker
                  label="起始年月"
                  value={selectedStartMonth}
                  minYear={minYearAvailable}
                  maxYear={maxYearAvailable}
                  onChange={(value) => handleMonthRangeCommit(value, selectedEndMonth)}
                />
                <MonthPicker
                  label="结束年月"
                  value={selectedEndMonth}
                  minYear={minYearAvailable}
                  maxYear={maxYearAvailable}
                  onChange={(value) => handleMonthRangeCommit(selectedStartMonth, value)}
                />
              </div>
              <div
                className="truncate text-xs font-medium text-muted-foreground"
                title={`${formatMonthLabel(selectedStartMonth)} - ${formatMonthLabel(selectedEndMonth)}`}
              >
                {formatMonthLabel(selectedStartMonth)} - {formatMonthLabel(selectedEndMonth)}
              </div>
              <Button
                variant="ghost"
                size="xs"
                className="h-7 px-2"
                onClick={() => handleMonthRangeCommit(defaultStartMonth, defaultEndMonth)}
              >
                重置时间
              </Button>
            </div>
          )}
        </div>
      </div>

      <div className="flex-shrink-0 p-4 border-t bg-background space-y-1">
        <Button
          variant="ghost"
          size="sm"
          className="w-full justify-start gap-2"
          onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
        >
          <Sun className="h-4 w-4 rotate-0 scale-100 transition-all dark:-rotate-90 dark:scale-0" />
          <Moon className="absolute h-4 w-4 rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100" />
          <span>切换主题</span>
        </Button>
      </div>
    </aside>
  );
}
