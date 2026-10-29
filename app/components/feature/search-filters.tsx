'use client';

/**
 * Search facets rendered as a row of filter pills with popover or bottom-sheet panels.
 */

import { useQuery } from '@tanstack/react-query';
import { Check, Search, X } from 'lucide-react';
import { usePathname, useRouter } from 'next/navigation';
import { parseAsArrayOf, parseAsString, useQueryState } from 'nuqs';
import { useEffect, useMemo, useState } from 'react';

import { DatabasePill, FilterFacet, FilterOptionRow } from '@/components/feature/filter-pill';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ScrollArea } from '@/components/ui/scroll-area';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { getAreas, getDatabases, getJournalOptions, getJournalRatings, getYears } from '@/lib/api';
import { getAreaDisplayName } from '@/lib/area-labels';
import {
  buildMonthKey,
  buildMonthRange,
  buildRecentMonthRange,
  buildYearOptions,
  formatMonthRangeLabel,
  getYearBounds,
  MONTH_OPTIONS,
  resolveMonthRangeForYears,
} from '@/lib/article-filters';
import { useAuth } from '@/lib/auth-context';
import {
  JOURNAL_RATING_SYSTEMS,
  useJournalRatingFilters,
  type JournalRatingKey,
} from '@/lib/journal-ratings';
import {
  reconcileSelectedDatabase,
  resolveAvailableSelectedDatabase,
  setSelectedDatabase,
  useSelectedDatabase,
} from '@/lib/selected-database';
import { cn } from '@/lib/utils';

const RECENT_YEAR_OPTIONS = [1, 3, 5] as const;

type DateSegmentSelectProps = {
  ariaLabel: string;
  value: string;
  options: readonly string[];
  onChange: (value: string) => void;
};

/**
 * Render one compact year or month picker.
 *
 * @param props - Accessible label, current value, options, and change handler.
 * @returns Date segment select.
 */
function DateSegmentSelect({ ariaLabel, value, options, onChange }: DateSegmentSelectProps) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger
        aria-label={ariaLabel}
        title={`${ariaLabel}：${value}`}
        size="sm"
        className="w-full justify-between px-2.5 tabular-nums"
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent position="popper" className="max-h-64">
        {options.map((option) => (
          <SelectItem key={option} value={option} className="tabular-nums">
            {option}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/**
 * Render database selection, publication time, ratings, journals, and areas as filter pills.
 *
 * @param props - Optional layout class names.
 * @returns Search filter bar.
 */
export function SearchFilters({ className }: { className?: string }) {
  const state = useSearchFilterState();
  const { appliedFilterCount, handleClearAll } = state;

  return (
    <div
      role="group"
      aria-label="筛选条件"
      className={cn(
        'scrollbar-none fade-x-edges -mx-4 -my-1 flex items-center gap-2 overflow-x-auto px-4 py-1 sm:-mx-5 sm:px-5 md:mx-0 md:flex-wrap md:overflow-visible md:px-0 md:[mask-image:none]',
        className,
      )}
    >
      {renderDatabaseFacet(state)}
      <span className="h-5 w-px shrink-0 bg-border" aria-hidden="true" />
      <TimeFacet state={state} />
      {renderRatingFacet(state)}
      {renderJournalFacet(state)}
      <AreaFacet state={state} />
      {appliedFilterCount > 0 && (
        <button
          type="button"
          title="清空全部筛选"
          className="motion-control inline-flex h-11 shrink-0 items-center gap-1 rounded-full px-2.5 text-[13px] md:h-8 text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
          onClick={handleClearAll}
        >
          <X className="size-3.5" aria-hidden="true" />
          清除筛选
        </button>
      )}
    </div>
  );
}

/** Own filter queries, URL state, and change handlers for the search filter pills. */
function useSearchFilterState() {
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
  const [ratings, setRatings] = useJournalRatingFilters();

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

  const {
    data: ratingOptions,
    isLoading: isLoadingRatings,
    isError: isRatingError,
    refetch: refetchRatings,
  } = useQuery({
    queryKey: ['meta', 'ratings', activeDb],
    queryFn: () => getJournalRatings(activeDb),
    enabled: !!user && !!activeDb,
  });

  const handleDatabaseChange = (dbName: string) => {
    setSelectedDatabase(dbName);
    setQ(null);
    setAreas([]);
    setJournalIds([]);
    setMonthRange(null);
    void setRatings(null);
    router.replace(pathname);
    router.refresh();
  };

  const handleClearRatings = () => {
    void setRatings(null);
  };

  /** Update one rating group without discarding other journal or area selections. */
  const handleRatingChange = (key: JournalRatingKey, value: string, isChecked: boolean) => {
    void setRatings((current) => ({
      [key]: isChecked
        ? [...new Set([...current[key], value])].sort()
        : current[key].filter((item) => item !== value),
    }));
  };

  const handleClearTimeFilters = () => {
    setMonthRange(null);
  };

  const handleClearAll = () => {
    setAreas([]);
    setJournalIds([]);
    setMonthRange(null);
    void setRatings(null);
  };

  const yearBounds = useMemo(() => getYearBounds(yearData ?? []), [yearData]);
  const monthValues = getFilterMonthValues(yearBounds, monthRange);
  const { defaultStartMonth, defaultEndMonth } = monthValues;

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
      ? journalOptions.filter((option) => (option.title ?? '').toLowerCase().includes(query))
      : journalOptions;
    if (journalIds.length === 0) {
      return matchedOptions;
    }
    const selectedIds = new Set(journalIds);
    return [
      ...matchedOptions.filter((option) => selectedIds.has(String(option.journal_id))),
      ...matchedOptions.filter((option) => !selectedIds.has(String(option.journal_id))),
    ];
  }, [journalIds, journalOptions, journalSearch]);

  const journalLabelMap = useMemo(() => {
    const map = new Map<string, string>();
    journalOptions?.forEach((option) => {
      map.set(String(option.journal_id), option.title ?? String(option.journal_id));
    });
    return map;
  }, [journalOptions]);

  const selectedJournalLabels = journalIds.map((id) => journalLabelMap.get(id) ?? id);
  const ratingSelectionCount = JOURNAL_RATING_SYSTEMS.reduce(
    (sum, { key }) => sum + ratings[key].length,
    0,
  );
  const appliedFilterCount =
    areas.length + journalIds.length + ratingSelectionCount + (monthRange ? 1 : 0);

  return {
    user,
    areas,
    journalIds,
    ratings,
    monthRange,
    databases,
    loadingDatabases,
    activeDb,
    areaOptions,
    loadingAreas,
    journalOptions,
    loadingJournals,
    loadingYears,
    ratingOptions,
    isLoadingRatings,
    isRatingError,
    refetchRatings,
    ratingSelectionCount,
    appliedFilterCount,
    handleDatabaseChange,
    handleClearAll,
    handleClearRatings,
    handleRatingChange,
    handleClearTimeFilters,
    yearBounds,
    ...monthValues,
    handleAreaChange,
    handleClearAreas: () => setAreas([]),
    handleJournalChange,
    handleClearJournals: () => setJournalIds([]),
    handleMonthRangeCommit,
    handleRecentMonthRange,
    journalSearch,
    setJournalSearch,
    filteredJournalOptions,
    selectedJournalLabels,
  };
}

type SearchFilterState = ReturnType<typeof useSearchFilterState>;

/** Render database selection as the shared database pill. */
function renderDatabaseFacet(state: SearchFilterState) {
  const { databases, loadingDatabases, activeDb, handleDatabaseChange } = state;

  if (loadingDatabases) {
    return <Skeleton className="h-11 w-40 shrink-0 rounded-full md:h-8" />;
  }
  return (
    <DatabasePill
      ariaLabel="检索数据库"
      databases={databases ?? []}
      value={activeDb}
      onChange={handleDatabaseChange}
    />
  );
}

/**
 * Render publication time presets and an optional custom month range.
 *
 * @param props - Search filter state.
 * @returns Publication time filter pill.
 */
function TimeFacet({ state }: { state: SearchFilterState }) {
  const {
    user,
    loadingYears,
    monthRange,
    handleClearTimeFilters,
    yearBounds,
    selectedStartMonth,
    selectedEndMonth,
    yearOptions,
    selectedStartYearValue,
    selectedStartMonthValue,
    selectedEndYearValue,
    selectedEndMonthValue,
    handleMonthRangeCommit,
    handleRecentMonthRange,
  } = state;
  const [isCustomRequested, setIsCustomRequested] = useState(false);
  const activeRecentYears = yearBounds
    ? RECENT_YEAR_OPTIONS.find((yearCount) => {
        const [recentStart, recentEnd] = buildRecentMonthRange(
          yearCount,
          yearBounds.min,
          yearBounds.max,
        );
        return selectedStartMonth === recentStart && selectedEndMonth === recentEnd;
      })
    : undefined;
  const isUnbounded = !monthRange;
  const isCustom = isCustomRequested || (!isUnbounded && activeRecentYears === undefined);
  const summary = isUnbounded
    ? undefined
    : activeRecentYears
      ? `近 ${activeRecentYears} 年`
      : (formatMonthRangeLabel(monthRange) ?? '自定义范围');
  const choices = [
    { key: 'all', label: '不限', isChecked: isUnbounded && !isCustomRequested },
    ...RECENT_YEAR_OPTIONS.map((yearCount) => ({
      key: `recent-${yearCount}`,
      label: `近 ${yearCount} 年`,
      isChecked: !isCustomRequested && activeRecentYears === yearCount,
    })),
    { key: 'custom', label: '自定义范围', isChecked: isCustom },
  ];

  return (
    <FilterFacet
      label="发表时间"
      summary={summary}
      description="按出版年月筛选文章。"
      isActive={!isUnbounded}
      onClear={() => {
        setIsCustomRequested(false);
        handleClearTimeFilters();
      }}
    >
      {!user || loadingYears ? (
        <Skeleton className="m-2 h-24" />
      ) : !yearBounds ? (
        <p className="px-2.5 py-3 text-sm text-muted-foreground">暂无可用发表年份</p>
      ) : (
        <div className="space-y-2">
          <div role="radiogroup" aria-label="发表时间" className="grid grid-cols-2 gap-1.5 p-1">
            {choices.map((choice) => (
              <button
                key={choice.key}
                type="button"
                role="radio"
                aria-checked={choice.isChecked}
                className={cn(
                  'motion-control flex h-10 items-center justify-between gap-2 rounded-lg px-3 text-left text-[13px] shadow-[inset_0_0_0_1px_var(--border)] outline-none transition-[background-color,box-shadow] hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring md:h-9',
                  choice.isChecked &&
                    'bg-info font-bold text-info-foreground shadow-[inset_0_0_0_1px_var(--primary)] hover:bg-info',
                )}
                onClick={() => {
                  if (choice.key === 'all') {
                    setIsCustomRequested(false);
                    handleClearTimeFilters();
                  } else if (choice.key === 'custom') {
                    setIsCustomRequested(true);
                  } else {
                    setIsCustomRequested(false);
                    handleRecentMonthRange(Number(choice.key.replace('recent-', '')));
                  }
                }}
              >
                {choice.label}
                {choice.isChecked && <Check className="size-3.5" aria-hidden="true" />}
              </button>
            ))}
          </div>
          {isCustom && (
            <div className="space-y-2 rounded-lg bg-muted p-3">
              <p className="text-xs text-muted-foreground">起始</p>
              <div className="grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)] gap-2">
                <DateSegmentSelect
                  ariaLabel="起始年份"
                  value={selectedStartYearValue}
                  options={yearOptions}
                  onChange={(value) =>
                    handleMonthRangeCommit(`${value}-${selectedStartMonthValue}`, selectedEndMonth)
                  }
                />
                <DateSegmentSelect
                  ariaLabel="起始月份"
                  value={selectedStartMonthValue}
                  options={MONTH_OPTIONS}
                  onChange={(value) =>
                    handleMonthRangeCommit(`${selectedStartYearValue}-${value}`, selectedEndMonth)
                  }
                />
              </div>
              <p className="text-xs text-muted-foreground">结束</p>
              <div className="grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)] gap-2">
                <DateSegmentSelect
                  ariaLabel="结束年份"
                  value={selectedEndYearValue}
                  options={yearOptions}
                  onChange={(value) =>
                    handleMonthRangeCommit(selectedStartMonth, `${value}-${selectedEndMonthValue}`)
                  }
                />
                <DateSegmentSelect
                  ariaLabel="结束月份"
                  value={selectedEndMonthValue}
                  options={MONTH_OPTIONS}
                  onChange={(value) =>
                    handleMonthRangeCommit(selectedStartMonth, `${selectedEndYearValue}-${value}`)
                  }
                />
              </div>
            </div>
          )}
        </div>
      )}
    </FilterFacet>
  );
}

/** Render rating systems as grouped checkbox grids. */
function renderRatingFacet(state: SearchFilterState) {
  const {
    ratings,
    ratingOptions,
    isLoadingRatings,
    isRatingError,
    refetchRatings,
    handleRatingChange,
    ratingSelectionCount,
    handleClearRatings,
  } = state;
  const hasRatings = JOURNAL_RATING_SYSTEMS.some(
    ({ key }) => (ratingOptions?.[key].length ?? 0) > 0,
  );

  return (
    <FilterFacet
      label="期刊评级"
      summary={`评级 · ${ratingSelectionCount}`}
      accessibleSummary={`已选 ${ratingSelectionCount} 项`}
      description="同一体系可多选，不同体系共同筛选。"
      isActive={ratingSelectionCount > 0}
      onClear={handleClearRatings}
      contentClassName="w-[22rem]"
    >
      {isLoadingRatings ? (
        <Skeleton className="m-2 h-24" />
      ) : isRatingError ? (
        <div
          className="flex items-center justify-between px-2.5 py-2 text-xs text-muted-foreground"
          role="status"
        >
          无法加载期刊评级
          <Button variant="ghost" size="sm" onClick={() => void refetchRatings()}>
            重试
          </Button>
        </div>
      ) : hasRatings ? (
        <div className="max-h-[min(60vh,26rem)] space-y-3 overflow-y-auto p-1">
          {JOURNAL_RATING_SYSTEMS.map(({ key, label }) => {
            const options = ratingOptions?.[key] ?? [];
            return options.length > 0 ? (
              <fieldset key={key}>
                <legend className="px-1.5 pb-1 text-xs font-bold text-muted-foreground">
                  {label}
                </legend>
                <div className="grid grid-cols-2 gap-x-1">
                  {options.map((option) => (
                    <FilterOptionRow
                      key={option.value}
                      id={`rating-${key}-${option.value}`}
                      ariaLabel={`${label} ${option.value}`}
                      label={option.value}
                      count={option.count}
                      isChecked={ratings[key].includes(option.value)}
                      onCheckedChange={(isChecked) =>
                        handleRatingChange(key, option.value, isChecked)
                      }
                    />
                  ))}
                </div>
              </fieldset>
            ) : null;
          })}
        </div>
      ) : (
        <p className="px-2.5 py-3 text-xs text-muted-foreground">当前数据库暂无期刊评级信息。</p>
      )}
    </FilterFacet>
  );
}

/** Render the searchable journal checklist. */
function renderJournalFacet(state: SearchFilterState) {
  const {
    journalIds,
    journalOptions,
    loadingJournals,
    handleJournalChange,
    handleClearJournals,
    journalSearch,
    setJournalSearch,
    filteredJournalOptions,
    selectedJournalLabels,
  } = state;
  const summary =
    selectedJournalLabels.length === 1
      ? selectedJournalLabels[0]
      : `期刊 · ${selectedJournalLabels.length}`;

  return (
    <FilterFacet
      label="期刊"
      summary={summary}
      accessibleSummary={
        selectedJournalLabels.length === 1 ? undefined : `已选 ${selectedJournalLabels.length} 本`
      }
      description={`在 ${journalOptions?.length ?? 0} 本期刊中选择。`}
      isActive={journalIds.length > 0}
      onClear={handleClearJournals}
      contentClassName="w-[24rem]"
    >
      {loadingJournals ? (
        <Skeleton className="m-2 h-24" />
      ) : (
        <div className="space-y-2">
          <div className="relative px-1 pt-1">
            <Search
              className="pointer-events-none absolute top-1/2 left-3.5 mt-0.5 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              aria-label="搜索期刊"
              name="journal_search"
              autoComplete="off"
              spellCheck={false}
              value={journalSearch}
              onChange={(event) => setJournalSearch(event.target.value)}
              placeholder="搜索期刊"
              className="h-9 pl-9"
            />
          </div>
          <JournalOptionList
            filteredJournalOptions={filteredJournalOptions}
            journalIds={journalIds}
            handleJournalChange={handleJournalChange}
          />
        </div>
      )}
    </FilterFacet>
  );
}

/**
 * Construct journal option rows only while the journal panel is mounted.
 *
 * @param props - Filtered options, selected identifiers, and change handler.
 * @returns Scrollable journal checklist.
 */
function JournalOptionList({
  filteredJournalOptions,
  journalIds,
  handleJournalChange,
}: Pick<SearchFilterState, 'filteredJournalOptions' | 'journalIds' | 'handleJournalChange'>) {
  const selectedJournalIds = new Set(journalIds);

  return (
    <ScrollArea className="h-[min(50vh,18rem)] touch-pan-y">
      <div data-slot="journal-options" className="py-1 pr-3 pl-1">
        {filteredJournalOptions.map((option) => {
          const id = String(option.journal_id);
          return (
            <FilterOptionRow
              key={id}
              id={`journal-${id}`}
              title={option.title ?? id}
              label={option.title ?? id}
              isChecked={selectedJournalIds.has(id)}
              onCheckedChange={(isChecked) => handleJournalChange(id, isChecked)}
            />
          );
        })}
        {filteredJournalOptions.length === 0 && (
          <p className="px-2.5 py-3 text-xs text-muted-foreground">未找到期刊。</p>
        )}
      </div>
    </ScrollArea>
  );
}

/**
 * Render research areas with an inline filter for long lists.
 *
 * @param props - Search filter state.
 * @returns Area filter pill.
 */
function AreaFacet({ state }: { state: SearchFilterState }) {
  const { areas, areaOptions, loadingAreas, handleAreaChange, handleClearAreas } = state;
  const [areaSearch, setAreaSearch] = useState('');
  const selectedAreas = new Set(areas);
  const query = areaSearch.trim().toLowerCase();
  const options = (areaOptions ?? []).filter(
    (option) =>
      !query ||
      option.value.toLowerCase().includes(query) ||
      getAreaDisplayName(option.value).toLowerCase().includes(query),
  );
  const summary = areas.length === 1 ? getAreaDisplayName(areas[0]) : `领域 · ${areas.length}`;

  return (
    <FilterFacet
      label="领域"
      summary={summary}
      accessibleSummary={areas.length === 1 ? undefined : `已选 ${areas.length} 个`}
      description="可多选，满足任一领域即显示。"
      isActive={areas.length > 0}
      onClear={handleClearAreas}
      contentClassName="w-[22rem]"
    >
      {loadingAreas ? (
        <Skeleton className="m-2 h-24" />
      ) : (
        <div className="space-y-2">
          {(areaOptions?.length ?? 0) > 10 && (
            <div className="relative px-1 pt-1">
              <Search
                className="pointer-events-none absolute top-1/2 left-3.5 mt-0.5 size-4 -translate-y-1/2 text-muted-foreground"
                aria-hidden="true"
              />
              <Input
                aria-label="搜索领域"
                autoComplete="off"
                value={areaSearch}
                onChange={(event) => setAreaSearch(event.target.value)}
                placeholder="搜索领域"
                className="h-9 pl-9"
              />
            </div>
          )}
          <div
            data-slot="area-options"
            className="max-h-[min(55vh,22rem)] overflow-y-auto overscroll-contain py-1 pr-3 pl-1"
          >
            {options.map((option) => (
              <FilterOptionRow
                key={option.value}
                id={`area-${option.value}`}
                title={option.value}
                label={getAreaDisplayName(option.value)}
                count={option.count}
                isChecked={selectedAreas.has(option.value)}
                onCheckedChange={(isChecked) => handleAreaChange(option.value, isChecked)}
              />
            ))}
            {options.length === 0 && (
              <p className="px-2.5 py-3 text-xs text-muted-foreground">未找到领域。</p>
            )}
          </div>
        </div>
      )}
    </FilterFacet>
  );
}

/** Derive month defaults and segment values for the current year bounds. */
function getFilterMonthValues(
  yearBounds: ReturnType<typeof getYearBounds>,
  monthRange: string | null,
) {
  const defaultStartMonth = yearBounds ? buildMonthKey(yearBounds.min, 1) : null;
  const defaultEndMonth = yearBounds ? buildMonthKey(yearBounds.max, 12) : null;
  const selectedMonthRange = yearBounds
    ? resolveMonthRangeForYears(monthRange, yearBounds.min, yearBounds.max)
    : null;
  const selectedStartMonth = selectedMonthRange?.[0] ?? '';
  const selectedEndMonth = selectedMonthRange?.[1] ?? '';
  const yearOptions = yearBounds ? buildYearOptions(yearBounds) : [];

  return {
    defaultStartMonth,
    defaultEndMonth,
    selectedStartMonth,
    selectedEndMonth,
    yearOptions,
    selectedStartYearValue: selectedStartMonth.slice(0, 4),
    selectedStartMonthValue: selectedStartMonth.slice(5, 7),
    selectedEndYearValue: selectedEndMonth.slice(0, 4),
    selectedEndMonthValue: selectedEndMonth.slice(5, 7),
  };
}
