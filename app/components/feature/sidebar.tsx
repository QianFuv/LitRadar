'use client';

import { useQuery } from '@tanstack/react-query';
import { useQueryState, parseAsString, parseAsArrayOf, parseAsInteger } from 'nuqs';
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
import { Slider } from '@/components/ui/slider';
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
import { Moon, Sun, Database } from 'lucide-react';
import { cn } from '@/lib/utils';
import { useEffect, useMemo, useState } from 'react';

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
  const [yearMin, setYearMin] = useQueryState('year_min', parseAsInteger);
  const [yearMax, setYearMax] = useQueryState('year_max', parseAsInteger);

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
    setYearMin(null);
    setYearMax(null);
  };

  const minYearAvailable =
    yearData && yearData.length > 0 ? Math.min(...yearData.map((y) => y.year)) : 1900;
  const maxYearAvailable =
    yearData && yearData.length > 0
      ? Math.max(...yearData.map((y) => y.year))
      : new Date().getFullYear();

  const yearRangeKey = `${minYearAvailable}-${maxYearAvailable}-${yearMin ?? 'null'}-${yearMax ?? 'null'}`;
  const defaultYearRange: [number, number] = [
    yearMin ?? minYearAvailable,
    yearMax ?? maxYearAvailable,
  ];
  const [localYearRangeState, setLocalYearRangeState] = useState<{
    key: string;
    value: [number, number];
  }>({
    key: yearRangeKey,
    value: defaultYearRange,
  });
  const localYearRange =
    localYearRangeState.key === yearRangeKey ? localYearRangeState.value : defaultYearRange;

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

  const handleYearChange = (value: number[]) => {
    setLocalYearRangeState({
      key: yearRangeKey,
      value: [value[0], value[1]],
    });
  };

  const handleYearCommit = (value: number[]) => {
    const nextMin = value[0] === minYearAvailable ? null : value[0];
    const nextMax = value[1] === maxYearAvailable ? null : value[1];
    setYearMin(nextMin);
    setYearMax(nextMax);
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
                {areaOptions?.map((opt) => (
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
                      {opt.value}
                    </Label>
                    <span className="shrink-0 text-xs text-muted-foreground">{opt.count}</span>
                  </div>
                ))}
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
          <h3 className="font-semibold text-sm text-foreground">发表年份</h3>
          {loadingYears ? (
            <Skeleton className="h-8 w-full" />
          ) : (
            <div className="px-1 pt-2">
              <Slider
                min={minYearAvailable}
                max={maxYearAvailable}
                step={1}
                value={localYearRange}
                onValueChange={handleYearChange}
                onValueCommit={handleYearCommit}
                className="mb-6"
              />
              <div className="flex justify-between text-xs text-muted-foreground font-medium">
                <span>{localYearRange[0]}</span>
                <span>{localYearRange[1]}</span>
              </div>
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
