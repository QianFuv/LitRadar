'use client';

import { useQuery } from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf } from 'nuqs';
import Image from 'next/image';
import Link from 'next/link';
import { usePathname, useRouter } from 'next/navigation';
import { useTheme } from 'next-themes';
import { getAreas, getYears, getJournalOptions, getDatabases } from '@/lib/api';
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
import { Database, Moon, Sun } from 'lucide-react';
import { getAreaDisplayName } from '@/lib/area-labels';
import {
  buildMonthKey,
  buildMonthRange,
  buildRecentMonthRange,
  buildYearOptions,
  formatMonthLabel,
  getYearBounds,
  MONTH_OPTIONS,
  resolveMonthRangeForYears,
} from '@/lib/article-filters';
import { cn } from '@/lib/utils';
import {
  reconcileSelectedDatabase,
  resolveAvailableSelectedDatabase,
  setSelectedDatabase,
  useSelectedDatabase,
} from '@/lib/selected-database';
import { useEffect, useMemo, useState } from 'react';

interface DateSegmentSelectProps {
  ariaLabel: string;
  value: string;
  options: readonly string[];
  triggerClassName?: string;
  contentClassName?: string;
  onChange: (value: string) => void;
}

/**
 * Render one underlined date segment dropdown.
 *
 * @param props - Date segment select configuration.
 * @returns Date segment dropdown UI.
 */
function DateSegmentSelect({
  ariaLabel,
  value,
  options,
  triggerClassName,
  contentClassName,
  onChange,
}: DateSegmentSelectProps) {
  const [open, setOpen] = useState(false);
  const handleSelect = (nextValue: string) => {
    onChange(nextValue);
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          aria-label={ariaLabel}
          title={`${ariaLabel}：${value}`}
          className={cn('h-8 px-2 text-sm', triggerClassName)}
        >
          {value}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className={cn('max-w-[calc(100vw-2rem)] p-2', contentClassName)}
      >
        <ScrollArea className="h-60 touch-pan-y">
          <div className="space-y-1">
            {options.map((option) => (
              <Button
                key={option}
                type="button"
                variant={option === value ? 'secondary' : 'ghost'}
                size="sm"
                className="h-8 w-full justify-center px-2"
                onClick={() => handleSelect(option)}
              >
                {option}
              </Button>
            ))}
          </div>
        </ScrollArea>
      </PopoverContent>
    </Popover>
  );
}

/**
 * Render database selection, article filters, and theme controls.
 *
 * @param props - Optional layout class names.
 * @returns Sidebar filter UI.
 */
export function Sidebar({ className }: { className?: string }) {
  const { theme, setTheme } = useTheme();
  const { user } = useAuth();
  const router = useRouter();
  const pathname = usePathname();

  const selectedDb = useSelectedDatabase();
  const [, setQ] = useQueryState('q', parseAsString);
  const [areas, setAreas] = useQueryState('area', parseAsArrayOf(parseAsString).withDefault([]));
  const [journalIds, setJournalIds] = useQueryState(
    'journal_id',
    parseAsArrayOf(parseAsString).withDefault([]),
  );
  const [monthRange, setMonthRange] = useQueryState('month_range', parseAsString);

  const { data: databases, isLoading: loadingDatabases } = useQuery({
    queryKey: ['meta', 'databases'],
    queryFn: () => getDatabases(),
    enabled: !!user,
  });
  const activeDb = resolveAvailableSelectedDatabase(selectedDb, databases ?? []);

  useEffect(() => {
    reconcileSelectedDatabase(selectedDb, databases ?? []);
  }, [databases, selectedDb]);

  const { data: areaOptions, isLoading: loadingAreas } = useQuery({
    queryKey: ['meta', 'areas', activeDb],
    queryFn: () => getAreas(activeDb),
    enabled: !!user,
  });

  const { data: journalOptions, isLoading: loadingJournals } = useQuery({
    queryKey: ['meta', 'journals', activeDb],
    queryFn: () => getJournalOptions(activeDb),
    enabled: !!user,
  });

  const { data: yearData, isLoading: loadingYears } = useQuery({
    queryKey: ['meta', 'years', activeDb],
    queryFn: () => getYears(activeDb),
    enabled: !!user,
  });

  const handleDatabaseChange = (dbName: string) => {
    setSelectedDatabase(dbName);
    setQ(null);
    setAreas([]);
    setJournalIds([]);
    setMonthRange(null);
    router.replace(pathname);
    router.refresh();
  };

  const handleClearFilters = () => {
    setQ(null);
    setAreas([]);
    setJournalIds([]);
    setMonthRange(null);
  };

  const handleClearJournalFilters = () => {
    setAreas([]);
    setJournalIds([]);
  };

  const handleClearTimeFilters = () => {
    setMonthRange(null);
  };

  const yearBounds = useMemo(() => getYearBounds(yearData ?? []), [yearData]);
  const defaultStartMonth = yearBounds ? buildMonthKey(yearBounds.min, 1) : null;
  const defaultEndMonth = yearBounds ? buildMonthKey(yearBounds.max, 12) : null;
  const selectedMonthRange = yearBounds
    ? resolveMonthRangeForYears(monthRange, yearBounds.min, yearBounds.max)
    : null;
  const selectedStartMonth = selectedMonthRange?.[0] ?? '';
  const selectedEndMonth = selectedMonthRange?.[1] ?? '';
  const yearOptions = yearBounds ? buildYearOptions(yearBounds) : [];
  const selectedStartYearValue = selectedStartMonth.slice(0, 4);
  const selectedStartMonthValue = selectedStartMonth.slice(5, 7);
  const selectedEndYearValue = selectedEndMonth.slice(0, 4);
  const selectedEndMonthValue = selectedEndMonth.slice(5, 7);

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
    if (!yearBounds || !defaultStartMonth || !defaultEndMonth) {
      return;
    }
    const [orderedStartMonth, orderedEndMonth] = resolveMonthRangeForYears(
      buildMonthRange(startMonth, endMonth),
      yearBounds.min,
      yearBounds.max,
    );
    setMonthRange(
      orderedStartMonth === defaultStartMonth && orderedEndMonth === defaultEndMonth
        ? null
        : buildMonthRange(orderedStartMonth, orderedEndMonth),
    );
  };

  const handleRecentMonthRange = (yearCount: number) => {
    if (!yearBounds) {
      return;
    }
    const [startMonth, endMonth] = buildRecentMonthRange(yearCount, yearBounds.min, yearBounds.max);
    setMonthRange(buildMonthRange(startMonth, endMonth));
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
                className="h-20 w-20"
                aria-label="首页"
                title="首页"
                asChild
              >
                <Link href="/">
                  <Image
                    src="https://cdn.sa.net/2026/01/29/6uRXpHqQfC89kF7.png"
                    alt=""
                    width={64}
                    height={64}
                    loading="eager"
                    fetchPriority="high"
                    className="h-16 w-16 object-contain"
                  />
                </Link>
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
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="w-full"
            onClick={handleClearFilters}
          >
            重置筛选
          </Button>
        </div>

        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-sm text-foreground">期刊筛选</h3>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleClearJournalFilters}
              className="h-6 px-2 text-xs"
              title="清空期刊筛选"
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
                    <div
                      key={opt.value}
                      className="flex min-w-0 items-start gap-2 [content-visibility:auto] [contain-intrinsic-size:0_32px]"
                    >
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
                    aria-label="搜索期刊"
                    name="journal_search"
                    autoComplete="off"
                    spellCheck={false}
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
                          <div
                            key={id}
                            className="flex min-w-0 items-start gap-2 [content-visibility:auto] [contain-intrinsic-size:0_32px]"
                          >
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
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-sm text-foreground">发表时间</h3>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleClearTimeFilters}
              className="h-6 px-2 text-xs"
              title="清空时间筛选"
            >
              清空
            </Button>
          </div>
          {!user || loadingYears ? (
            <Skeleton className="h-8 w-full" />
          ) : !yearBounds ? (
            <p className="text-sm text-muted-foreground">暂无可用发表年份</p>
          ) : (
            <>
              <div
                className="grid grid-cols-[minmax(0,1fr)_minmax(0,0.78fr)_auto_minmax(0,1fr)_minmax(0,0.78fr)] items-end gap-1"
                title={`${formatMonthLabel(selectedStartMonth)} - ${formatMonthLabel(selectedEndMonth)}`}
              >
                <DateSegmentSelect
                  ariaLabel="起始年份"
                  value={selectedStartYearValue}
                  options={yearOptions}
                  triggerClassName="w-full"
                  contentClassName="w-[4.75rem]"
                  onChange={(value) =>
                    handleMonthRangeCommit(`${value}-${selectedStartMonthValue}`, selectedEndMonth)
                  }
                />
                <DateSegmentSelect
                  ariaLabel="起始月份"
                  value={selectedStartMonthValue}
                  options={MONTH_OPTIONS}
                  triggerClassName="w-full"
                  contentClassName="w-16"
                  onChange={(value) =>
                    handleMonthRangeCommit(`${selectedStartYearValue}-${value}`, selectedEndMonth)
                  }
                />
                <span className="text-center text-sm text-muted-foreground">-</span>
                <DateSegmentSelect
                  ariaLabel="结束年份"
                  value={selectedEndYearValue}
                  options={yearOptions}
                  triggerClassName="w-full"
                  contentClassName="w-[4.75rem]"
                  onChange={(value) =>
                    handleMonthRangeCommit(selectedStartMonth, `${value}-${selectedEndMonthValue}`)
                  }
                />
                <DateSegmentSelect
                  ariaLabel="结束月份"
                  value={selectedEndMonthValue}
                  options={MONTH_OPTIONS}
                  triggerClassName="w-full"
                  contentClassName="w-16"
                  onChange={(value) =>
                    handleMonthRangeCommit(selectedStartMonth, `${selectedEndYearValue}-${value}`)
                  }
                />
              </div>
              <div className="grid grid-cols-3 gap-2">
                {[1, 3, 5].map((yearCount) => (
                  <Button
                    key={yearCount}
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => handleRecentMonthRange(yearCount)}
                  >
                    近 {yearCount} 年
                  </Button>
                ))}
              </div>
            </>
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
          <Sun className="h-4 w-4 rotate-0 scale-100 transition-transform dark:-rotate-90 dark:scale-0" />
          <Moon className="absolute h-4 w-4 rotate-90 scale-0 transition-transform dark:rotate-0 dark:scale-100" />
          <span>切换主题</span>
        </Button>
      </div>
    </aside>
  );
}
