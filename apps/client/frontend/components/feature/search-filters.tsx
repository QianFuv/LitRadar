/**
 * Search facets rendered as a row of filter pills with popover or bottom-sheet panels.
 */

import { useQuery } from '@tanstack/react-query';
import { ChevronDown, X } from '@/components/ui/animated-icons';
import { useLocation, useNavigate } from 'react-router';
import { parseAsArrayOf, parseAsString, useQueryState } from 'nuqs';
import { useEffect, useMemo, useState } from 'react';

import {
  DatabasePill,
  FilterFacet,
  FilterPanelHeader,
  FilterPillLabel,
  getFilterPillClassName,
  getFilterPillText,
} from '@/components/feature/filter-pill';
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
  ComboboxTrigger,
} from '@/components/ui/combobox';
import {
  MultiSelect,
  MultiSelectContent,
  MultiSelectGroup,
  MultiSelectGroupLabel,
  MultiSelectItem,
  MultiSelectList,
  MultiSelectTrigger,
  MultiSelectValue,
} from '@/components/ui/multi-select';
import { RadioGroup, RadioGroupCard } from '@/components/ui/radio-group';
import { attachHorizontalWheelScroll } from '@/lib/horizontal-wheel-scroll';
import { ScrollFade } from '@animate-ui/react/primitives/effects/scroll-fade';
import { Button } from '@/components/ui/button';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { RevealItem } from '@/components/ui/reveal';
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
    <ScrollFade orientation="horizontal" shrink={0} asChild>
      <div
        ref={attachHorizontalWheelScroll}
        role="group"
        aria-label="筛选条件"
        className={cn(
          'scrollbar-none -mx-4 -my-1 flex items-center gap-2 overflow-x-auto px-4 py-1 sm:-mx-5 sm:px-5 md:mx-0 md:px-0',
          className,
        )}
      >
        {renderDatabaseFacet(state)}
        <span className="h-5 w-px shrink-0 bg-border" aria-hidden="true" />
        <TimeFacet state={state} />
        {renderRatingFacet(state)}
        <JournalFacet state={state} />
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
    </ScrollFade>
  );
}

/** Own filter queries, URL state, and change handlers for the search filter pills. */
function useSearchFilterState() {
  const { user } = useAuth();
  const navigate = useNavigate();
  const { pathname } = useLocation();

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
    navigate(pathname, { replace: true });
  };

  const handleClearRatings = () => {
    void setRatings(null);
  };

  /** Replace every rating group at once without touching journal or area selections. */
  const handleRatingsChange = (next: Record<JournalRatingKey, string[]>) => {
    void setRatings(next);
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

  const handleAreasChange = (values: string[]) => {
    void setAreas(values);
  };

  const handleJournalsChange = (ids: string[]) => {
    void setJournalIds(ids);
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
    handleRatingsChange,
    handleClearTimeFilters,
    yearBounds,
    ...monthValues,
    handleAreasChange,
    handleClearAreas: () => setAreas([]),
    handleJournalsChange,
    handleClearJournals: () => setJournalIds([]),
    handleMonthRangeCommit,
    handleRecentMonthRange,
    journalLabelMap,
    selectedJournalLabels,
  };
}

type SearchFilterState = ReturnType<typeof useSearchFilterState>;

/** Render database selection as the shared database pill. */
function renderDatabaseFacet(state: SearchFilterState) {
  const { databases, loadingDatabases, activeDb, handleDatabaseChange } = state;

  return (
    <RevealItem
      isReady={!loadingDatabases}
      className="shrink-0"
      placeholder={<Skeleton className="h-11 w-40 rounded-full md:h-8" />}
    >
      <DatabasePill
        ariaLabel="检索数据库"
        databases={databases ?? []}
        value={activeDb}
        onChange={handleDatabaseChange}
      />
    </RevealItem>
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
  const selectedChoice = isCustom
    ? 'custom'
    : activeRecentYears
      ? `recent-${activeRecentYears}`
      : 'all';
  const choices = [
    { key: 'all', label: '不限' },
    ...RECENT_YEAR_OPTIONS.map((yearCount) => ({
      key: `recent-${yearCount}`,
      label: `近 ${yearCount} 年`,
    })),
    { key: 'custom', label: '自定义范围' },
  ];

  /**
   * Apply one preset, or reveal the custom range controls.
   *
   * @param choice - Selected preset key.
   */
  const handleChoice = (choice: string) => {
    if (choice === 'all') {
      setIsCustomRequested(false);
      handleClearTimeFilters();
    } else if (choice === 'custom') {
      setIsCustomRequested(true);
    } else {
      setIsCustomRequested(false);
      handleRecentMonthRange(Number(choice.replace('recent-', '')));
    }
  };

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
      <RevealItem
        isReady={Boolean(user) && !loadingYears}
        placeholder={<Skeleton className="m-2 h-24" />}
      >
        {!yearBounds ? (
          <p className="px-2.5 py-3 text-sm text-muted-foreground">暂无可用发表年份</p>
        ) : (
          <div className="space-y-2">
            <RadioGroup
              aria-label="发表时间"
              value={selectedChoice}
              onValueChange={handleChoice}
              className="grid grid-cols-2 gap-1.5 p-1"
            >
              {choices.map((choice) => (
                <RadioGroupCard key={choice.key} value={choice.key} className="h-10 md:h-9">
                  {choice.label}
                </RadioGroupCard>
              ))}
            </RadioGroup>
            {isCustom && (
              <div className="space-y-2 rounded-lg bg-muted p-3">
                <p className="text-xs text-muted-foreground">起始</p>
                <div className="grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)] gap-2">
                  <DateSegmentSelect
                    ariaLabel="起始年份"
                    value={selectedStartYearValue}
                    options={yearOptions}
                    onChange={(value) =>
                      handleMonthRangeCommit(
                        `${value}-${selectedStartMonthValue}`,
                        selectedEndMonth,
                      )
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
                      handleMonthRangeCommit(
                        selectedStartMonth,
                        `${value}-${selectedEndMonthValue}`,
                      )
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
      </RevealItem>
    </FilterFacet>
  );
}

/** Render rating systems as one grouped multi-select. */
function renderRatingFacet(state: SearchFilterState) {
  const {
    ratings,
    ratingOptions,
    isLoadingRatings,
    isRatingError,
    refetchRatings,
    handleRatingsChange,
    ratingSelectionCount,
    handleClearRatings,
  } = state;
  const hasRatings = JOURNAL_RATING_SYSTEMS.some(
    ({ key }) => (ratingOptions?.[key].length ?? 0) > 0,
  );
  const isActive = ratingSelectionCount > 0;
  const { visibleText, accessibleName } = getFilterPillText(
    '期刊评级',
    isActive,
    `评级 · ${ratingSelectionCount}`,
    `已选 ${ratingSelectionCount} 项`,
  );

  return (
    <MultiSelect
      multiple
      value={encodeRatingSelection(ratings)}
      onValueChange={(values: string[]) => handleRatingsChange(decodeRatingSelection(values))}
    >
      <MultiSelectTrigger
        aria-label={accessibleName}
        title={accessibleName}
        data-active={isActive ? 'true' : 'false'}
        className={getFilterPillClassName(isActive)}
      >
        <span className="min-w-0 truncate">
          <MultiSelectValue>{() => visibleText}</MultiSelectValue>
        </span>
        <ChevronDown className="size-3.5 shrink-0 opacity-70" aria-hidden="true" />
      </MultiSelectTrigger>
      <MultiSelectContent aria-label="期刊评级" className="w-[22rem] max-w-[calc(100vw-2rem)]">
        <FilterPanelHeader
          label="期刊评级"
          description="同一体系可多选，不同体系共同筛选。"
          isActive={isActive}
          onClear={handleClearRatings}
        />
        <RevealItem
          isReady={!isLoadingRatings}
          className="flex min-h-0 flex-col"
          placeholder={<Skeleton className="m-2 h-24" />}
        >
          {isRatingError ? (
            <div
              className="flex items-center justify-between px-4 py-2 text-xs text-muted-foreground"
              role="status"
            >
              无法加载期刊评级
              <Button variant="ghost" size="sm" onClick={() => void refetchRatings()}>
                重试
              </Button>
            </div>
          ) : hasRatings ? (
            <MultiSelectList className="max-h-[min(60vh,26rem)]">
              {JOURNAL_RATING_SYSTEMS.map(({ key, label }) => {
                const options = ratingOptions?.[key] ?? [];
                return options.length > 0 ? (
                  <MultiSelectGroup key={key}>
                    <MultiSelectGroupLabel>{label}</MultiSelectGroupLabel>
                    {options.map((option) => (
                      <MultiSelectItem
                        key={option.value}
                        value={encodeRatingValue(key, option.value)}
                        aria-label={`${label} ${option.value}`}
                        detail={option.count}
                      >
                        {option.value}
                      </MultiSelectItem>
                    ))}
                  </MultiSelectGroup>
                ) : null;
              })}
            </MultiSelectList>
          ) : (
            <p className="px-4 py-3 text-xs text-muted-foreground">当前数据库暂无期刊评级信息。</p>
          )}
        </RevealItem>
      </MultiSelectContent>
    </MultiSelect>
  );
}

/** Shared empty option list for closed pickers. */
const NO_OPTION_IDS: string[] = [];

/**
 * Render the searchable journal multi-select; options are only built while it is open.
 *
 * @param props - Search filter state.
 * @returns Journal filter pill.
 */
function JournalFacet({ state }: { state: SearchFilterState }) {
  const {
    journalIds,
    journalOptions,
    loadingJournals,
    handleJournalsChange,
    handleClearJournals,
    journalLabelMap,
    selectedJournalLabels,
  } = state;
  const [isOpen, setIsOpen] = useState(false);
  const orderedJournalIds = useMemo(() => {
    if (!isOpen || !journalOptions) {
      return NO_OPTION_IDS;
    }
    const ids = journalOptions.map((option) => String(option.journal_id));
    const selectedIds = new Set(journalIds);
    return [
      ...ids.filter((id) => selectedIds.has(id)),
      ...ids.filter((id) => !selectedIds.has(id)),
    ];
  }, [isOpen, journalIds, journalOptions]);
  const isActive = journalIds.length > 0;
  const { visibleText, accessibleName } = getFilterPillText(
    '期刊',
    isActive,
    selectedJournalLabels.length === 1
      ? selectedJournalLabels[0]
      : `期刊 · ${selectedJournalLabels.length}`,
    selectedJournalLabels.length === 1 ? undefined : `已选 ${selectedJournalLabels.length} 本`,
  );

  return (
    <Combobox
      multiple
      open={isOpen}
      onOpenChange={setIsOpen}
      items={orderedJournalIds}
      value={journalIds}
      onValueChange={(ids: string[]) => handleJournalsChange(ids)}
      itemToStringLabel={(id: string) => journalLabelMap.get(id) ?? id}
    >
      <ComboboxTrigger
        aria-label={accessibleName}
        title={accessibleName}
        data-active={isActive ? 'true' : 'false'}
        className={getFilterPillClassName(isActive)}
      >
        <FilterPillLabel text={visibleText} />
      </ComboboxTrigger>
      <ComboboxContent aria-label="期刊" className="w-[24rem] max-w-[calc(100vw-2rem)]">
        <FilterPanelHeader
          label="期刊"
          description={`在 ${journalOptions?.length ?? 0} 本期刊中选择。`}
          isActive={isActive}
          onClear={handleClearJournals}
        />
        <RevealItem
          isReady={!loadingJournals}
          className="flex min-h-0 flex-col"
          placeholder={<Skeleton className="m-2 h-24" />}
        >
          <ComboboxInput aria-label="搜索期刊" name="journal_search" placeholder="搜索期刊" />
          <ComboboxEmpty>未找到期刊。</ComboboxEmpty>
          <ComboboxList data-slot="journal-options" className="max-h-[min(50vh,18rem)]">
            {(id: string) => (
              <ComboboxItem key={id} value={id} title={journalLabelMap.get(id) ?? id}>
                {journalLabelMap.get(id) ?? id}
              </ComboboxItem>
            )}
          </ComboboxList>
        </RevealItem>
      </ComboboxContent>
    </Combobox>
  );
}

/**
 * Match one research area by its raw value or display name.
 *
 * @param area - Area value.
 * @param query - Search text.
 * @returns Whether the area matches.
 */
function matchesArea(area: string, query: string): boolean {
  const normalizedQuery = query.trim().toLowerCase();
  return (
    !normalizedQuery ||
    area.toLowerCase().includes(normalizedQuery) ||
    getAreaDisplayName(area).toLowerCase().includes(normalizedQuery)
  );
}

/**
 * Render research areas as a multi-select that offers search for long lists.
 *
 * @param props - Search filter state.
 * @returns Area filter pill.
 */
function AreaFacet({ state }: { state: SearchFilterState }) {
  const { areas, areaOptions, loadingAreas, handleAreasChange, handleClearAreas } = state;
  const areaCounts = useMemo(
    () => new Map((areaOptions ?? []).map((option) => [option.value, option.count])),
    [areaOptions],
  );
  const areaValues = useMemo(
    () => (areaOptions ?? []).map((option) => option.value),
    [areaOptions],
  );
  const isActive = areas.length > 0;
  const { visibleText, accessibleName } = getFilterPillText(
    '领域',
    isActive,
    areas.length === 1 ? getAreaDisplayName(areas[0]) : `领域 · ${areas.length}`,
    areas.length === 1 ? undefined : `已选 ${areas.length} 个`,
  );

  return (
    <Combobox
      multiple
      items={areaValues}
      value={areas}
      onValueChange={(values: string[]) => handleAreasChange(values)}
      itemToStringLabel={getAreaDisplayName}
      filter={matchesArea}
    >
      <ComboboxTrigger
        aria-label={accessibleName}
        title={accessibleName}
        data-active={isActive ? 'true' : 'false'}
        className={getFilterPillClassName(isActive)}
      >
        <FilterPillLabel text={visibleText} />
      </ComboboxTrigger>
      <ComboboxContent aria-label="领域" className="w-[22rem] max-w-[calc(100vw-2rem)]">
        <FilterPanelHeader
          label="领域"
          description="可多选，满足任一领域即显示。"
          isActive={isActive}
          onClear={handleClearAreas}
        />
        <RevealItem
          isReady={!loadingAreas}
          className="flex min-h-0 flex-col"
          placeholder={<Skeleton className="m-2 h-24" />}
        >
          {areaValues.length > 10 && <ComboboxInput aria-label="搜索领域" placeholder="搜索领域" />}
          <ComboboxEmpty>未找到领域。</ComboboxEmpty>
          <ComboboxList data-slot="area-options" className="max-h-[min(55vh,22rem)]">
            {(area: string) => (
              <ComboboxItem key={area} value={area} title={area} detail={areaCounts.get(area)}>
                {getAreaDisplayName(area)}
              </ComboboxItem>
            )}
          </ComboboxList>
        </RevealItem>
      </ComboboxContent>
    </Combobox>
  );
}

/**
 * Encode one rating grade as a select value.
 *
 * @param key - Rating system.
 * @param value - Grade within the system.
 * @returns Combined select value.
 */
function encodeRatingValue(key: JournalRatingKey, value: string): string {
  return `${key}:${value}`;
}

/**
 * Encode every selected rating grade as select values.
 *
 * @param ratings - Selected grades per rating system.
 * @returns Combined select values.
 */
function encodeRatingSelection(ratings: Record<JournalRatingKey, string[]>): string[] {
  return JOURNAL_RATING_SYSTEMS.flatMap(({ key }) =>
    ratings[key].map((value) => encodeRatingValue(key, value)),
  );
}

/**
 * Decode select values back into sorted grades per rating system.
 *
 * @param values - Combined select values.
 * @returns Selected grades per rating system.
 */
function decodeRatingSelection(values: readonly string[]): Record<JournalRatingKey, string[]> {
  return Object.fromEntries(
    JOURNAL_RATING_SYSTEMS.map(({ key }) => [
      key,
      [
        ...new Set(
          values
            .filter((value) => value.startsWith(`${key}:`))
            .map((value) => value.slice(key.length + 1)),
        ),
      ].sort(),
    ]),
  ) as Record<JournalRatingKey, string[]>;
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
