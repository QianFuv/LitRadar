/**
 * Search facets rendered as a row of filter pills with popover or bottom-sheet panels.
 */

import { monthKey } from '@/lib/date';
import { useQuery } from '@tanstack/react-query';
import { ChevronDown, X } from '@/components/ui/animated-icons';
import { useSearch } from '@tanstack/react-router';
import { useVirtualizer, type Virtualizer } from '@tanstack/react-virtual';
import { useImperativeHandle, useRef, useState, type RefObject } from 'react';

import {
  DatabasePill,
  FilterFacet,
  FilterPanelHeader,
  FilterPillLabel,
  filterPillStyles,
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
  useComboboxFilteredItems,
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
import {
  areasQuery,
  databasesQuery,
  journalOptionsQuery,
  journalRatingsQuery,
  yearsQuery,
} from '@/lib/api';
import { getAreaDisplayName } from '@/lib/area-labels';
import {
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
  readJournalRatings,
  withJournalRatings,
  type JournalRatingKey,
} from '@/lib/journal-ratings';
import { useUpdateWorkspaceSearch, WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { setSelectedDatabase, useActiveDatabase } from '@/lib/selected-database';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

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
        xstyle={styles.dateSegmentSelectSelectTrigger}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent xstyle={styles.dateSegmentSelectSelectContent}>
        {options.map((option) => (
          <SelectItem key={option} value={option} xstyle={styles.dateSegmentSelectSelectItem}>
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
 * @param props - Optional layout styles.
 * @returns Search filter bar.
 */
export function SearchFilters({ xstyle }: { xstyle?: stylex.StyleXStyles }) {
  const state = useSearchFilterState();
  const { appliedFilterCount, handleClearAll } = state;

  return (
    <fieldset aria-label="筛选条件" {...stylex.props(styles.layoutFreeGroup)}>
      <ScrollFade orientation="horizontal" shrink={0} asChild>
        <div ref={attachHorizontalWheelScroll} {...stylex.props(styles.searchFiltersDiv, xstyle)}>
          {renderDatabaseFacet(state)}
          <span {...stylex.props(styles.searchFiltersSpan)} aria-hidden="true" />
          <TimeFacet state={state} />
          {renderRatingFacet(state)}
          <JournalFacet state={state} />
          <AreaFacet state={state} />
          {appliedFilterCount > 0 && (
            <button
              type="button"
              title="清空全部筛选"
              {...stylex.props(styles.searchFiltersButton)}
              onClick={handleClearAll}
            >
              <X xstyle={styles.searchFiltersX} aria-hidden="true" />
              清除筛选
            </button>
          )}
        </div>
      </ScrollFade>
    </fieldset>
  );
}

/** Own filter queries, URL state, and change handlers for the search filter pills. */
function useSearchFilterState() {
  const { user } = useAuth();
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const updateSearch = useUpdateWorkspaceSearch();

  const areas = search.area ?? [];
  const journalIds = search.journal_id ?? [];
  const monthRange = search.month_range ?? null;
  const ratings = readJournalRatings(search);

  /**
   * Replace or remove the month-range filter.
   *
   * @param value - Month range, or null to remove it.
   */
  const setMonthRange = (value: string | null) => {
    void updateSearch((current) => ({ ...current, month_range: value ?? undefined }));
  };

  const { data: databases, isLoading: loadingDatabases } = useQuery({
    ...databasesQuery(),
    enabled: !!user,
  });
  const activeDb = useActiveDatabase() ?? '';
  const canQueryDatabase = !!user && activeDb !== '';

  const { data: areaOptions, isLoading: loadingAreas } = useQuery({
    ...areasQuery(activeDb),
    enabled: canQueryDatabase,
  });

  const { data: journalOptions, isLoading: loadingJournals } = useQuery({
    ...journalOptionsQuery(activeDb),
    enabled: canQueryDatabase,
  });

  const { data: yearData, isLoading: loadingYears } = useQuery({
    ...yearsQuery(activeDb),
    enabled: canQueryDatabase,
  });

  const {
    data: ratingOptions,
    isLoading: isLoadingRatings,
    isError: isRatingError,
    refetch: refetchRatings,
  } = useQuery({
    ...journalRatingsQuery(activeDb),
    enabled: canQueryDatabase,
  });

  const handleDatabaseChange = (dbName: string) => {
    setSelectedDatabase(dbName);
    void updateSearch(() => ({}));
  };

  const handleClearRatings = () => {
    void updateSearch((current) => withJournalRatings(current, null));
  };

  /** Replace every rating group at once without touching journal or area selections. */
  const handleRatingsChange = (next: Record<JournalRatingKey, string[]>) => {
    void updateSearch((current) => withJournalRatings(current, next));
  };

  const handleClearTimeFilters = () => {
    setMonthRange(null);
  };

  const handleClearAll = () => {
    void updateSearch((current) => ({
      ...withJournalRatings(current, null),
      area: undefined,
      journal_id: undefined,
      month_range: undefined,
    }));
  };

  const yearBounds = getYearBounds(yearData ?? []);
  const monthValues = getFilterMonthValues(yearBounds, monthRange);
  const { defaultStartMonth, defaultEndMonth } = monthValues;

  const handleAreasChange = (values: string[]) => {
    void updateSearch((current) => ({ ...current, area: values }));
  };

  const handleJournalsChange = (ids: string[]) => {
    void updateSearch((current) => ({ ...current, journal_id: ids }));
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

  const journalLabelMap = new Map<string, string>();
  journalOptions?.forEach((option) => {
    journalLabelMap.set(String(option.journal_id), option.title ?? String(option.journal_id));
  });

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
    loadingAreas: loadingAreas || !activeDb,
    journalOptions,
    loadingJournals: loadingJournals || !activeDb,
    loadingYears: loadingYears || !activeDb,
    ratingOptions,
    isLoadingRatings: isLoadingRatings || !activeDb,
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
    handleClearAreas: () => handleAreasChange([]),
    handleJournalsChange,
    handleClearJournals: () => handleJournalsChange([]),
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
      xstyle={styles.renderDatabaseFacetRevealItem}
      placeholder={<Skeleton xstyle={styles.renderDatabaseFacetSkeleton} />}
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
        placeholder={<Skeleton xstyle={styles.timeFacetSkeleton} />}
      >
        {!yearBounds ? (
          <p {...stylex.props(styles.timeFacetP)}>暂无可用发表年份</p>
        ) : (
          <div>
            <RadioGroup
              aria-label="发表时间"
              value={selectedChoice}
              onValueChange={handleChoice}
              xstyle={[styles.timeFacetDivItem, styles.timeFacetRadioGroup]}
            >
              {choices.map((choice) => (
                <RadioGroupCard
                  key={choice.key}
                  value={choice.key}
                  xstyle={styles.timeFacetRadioGroupCard}
                >
                  {choice.label}
                </RadioGroupCard>
              ))}
            </RadioGroup>
            {isCustom && (
              <div {...stylex.props(styles.timeFacetDivItem, styles.timeFacetDiv)}>
                <p {...stylex.props(styles.timeFacetDivItem, styles.timeFacetP2)}>起始</p>
                <div {...stylex.props(styles.timeFacetDivItem, styles.timeFacetDiv2)}>
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
                <p {...stylex.props(styles.timeFacetDivItem, styles.timeFacetP2)}>结束</p>
                <div {...stylex.props(styles.timeFacetDivItem, styles.timeFacetDiv2)}>
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
        {...stylex.props(filterPillStyles(isActive))}
      >
        <span {...stylex.props(styles.renderRatingFacetSpan)}>
          <MultiSelectValue>{() => visibleText}</MultiSelectValue>
        </span>
        <ChevronDown xstyle={styles.renderRatingFacetChevronDown} aria-hidden="true" />
      </MultiSelectTrigger>
      <MultiSelectContent aria-label="期刊评级" xstyle={styles.renderRatingFacetMultiSelectContent}>
        <FilterPanelHeader
          label="期刊评级"
          description="同一体系可多选，不同体系共同筛选。"
          isActive={isActive}
          onClear={handleClearRatings}
        />
        <RevealItem
          isReady={!isLoadingRatings}
          xstyle={styles.renderRatingFacetRevealItem}
          placeholder={<Skeleton xstyle={styles.timeFacetSkeleton} />}
        >
          {isRatingError ? (
            <output {...stylex.props(styles.statusOutput, styles.renderRatingFacetDiv)}>
              无法加载期刊评级
              <Button variant="ghost" size="sm" onClick={() => void refetchRatings()}>
                重试
              </Button>
            </output>
          ) : hasRatings ? (
            <MultiSelectList xstyle={styles.renderRatingFacetMultiSelectList}>
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
            <p {...stylex.props(styles.renderRatingFacetP)}>当前数据库暂无期刊评级信息。</p>
          )}
        </RevealItem>
      </MultiSelectContent>
    </MultiSelect>
  );
}

/** Shared empty option list for closed pickers. */
const NO_OPTION_IDS: string[] = [];

/** Virtualizer over the journal option list's scroll element. */
type JournalOptionVirtualizer = Virtualizer<HTMLDivElement, Element>;

/** Height of a one-line journal option before it is measured, in pixels. */
const JOURNAL_OPTION_ESTIMATED_HEIGHT = 36;
/** Rows rendered beyond each edge of the visible range, so keyboard steps land on mounted rows. */
const JOURNAL_OPTION_OVERSCAN = 8;
/** Block padding of the option list, kept inside the virtualized scroll range, in pixels. */
const JOURNAL_LIST_PADDING_BLOCK = 6;

/**
 * Scroll a highlighted journal into view when it may not be mounted yet.
 *
 * Mounted rows are scrolled into view by the combobox itself; highlights that jump past the
 * rendered range, such as wrapping from the first row to the last, need the virtualizer.
 *
 * @param virtualizer - Virtualizer of the open option list, or null while it is unmounted.
 * @param details - Highlight reason and the highlighted row's index in the filtered list.
 */
function scrollHighlightedJournal(
  virtualizer: JournalOptionVirtualizer | null,
  details: { reason: string; index: number },
): void {
  const { reason, index } = details;
  if (!virtualizer || index < 0) {
    return;
  }
  const isEnd = index === virtualizer.options.count - 1;
  if (reason === 'none' || (reason === 'keyboard' && (index === 0 || isEnd))) {
    queueMicrotask(() => virtualizer.scrollToIndex(index, { align: isEnd ? 'start' : 'end' }));
  }
}

/**
 * Measure one mounted journal row by its layout height.
 *
 * Layout height ignores the popup's opening scale transform. A row without layout, such as one
 * inside a hidden list, keeps the estimate so the visible range does not grow to every row.
 *
 * @param element - Mounted row element.
 * @returns Row height in pixels.
 */
function measureJournalRow(element: Element): number {
  const height = element instanceof HTMLElement ? element.offsetHeight : 0;
  return height || JOURNAL_OPTION_ESTIMATED_HEIGHT;
}

type JournalOptionListProps = {
  /** Display title per journal id. */
  journalLabelMap: ReadonlyMap<string, string>;
  /** Receives the virtualizer so the combobox can scroll highlighted rows into view. */
  virtualizerRef: RefObject<JournalOptionVirtualizer | null>;
};

/**
 * Render only the visible rows of the filtered journal options.
 *
 * Rows keep their position in the full filtered list for keyboard navigation and announce it
 * through `aria-setsize` and `aria-posinset`, since most rows are not in the DOM. React Compiler
 * is opted out because the virtualizer is a mutable instance whose reads must not be memoized.
 * Rows are placed with `top`, not a transform: a transform would make every row its own stacking
 * context, so the highlight surface sliding inside a later row would paint over the labels of the
 * earlier rows it crosses instead of beneath them.
 *
 * @param props - Journal titles and the virtualizer handle.
 * @returns Virtualized option list, empty when no journal matches.
 */
function JournalOptionList({ journalLabelMap, virtualizerRef }: JournalOptionListProps) {
  'use no memo';
  const filteredIds = useComboboxFilteredItems<string>();
  const [scrollElement, setScrollElement] = useState<HTMLDivElement | null>(null);
  // oxlint-disable-next-line react/incompatible-library -- 'use no memo' keeps its reads fresh.
  const virtualizer = useVirtualizer({
    count: filteredIds.length,
    getScrollElement: () => scrollElement,
    getItemKey: (index) => filteredIds[index] ?? index,
    estimateSize: () => JOURNAL_OPTION_ESTIMATED_HEIGHT,
    measureElement: measureJournalRow,
    overscan: JOURNAL_OPTION_OVERSCAN,
    paddingStart: JOURNAL_LIST_PADDING_BLOCK,
    paddingEnd: JOURNAL_LIST_PADDING_BLOCK,
    scrollPaddingStart: JOURNAL_LIST_PADDING_BLOCK,
    scrollPaddingEnd: JOURNAL_LIST_PADDING_BLOCK,
  });
  useImperativeHandle(virtualizerRef, () => virtualizer, [virtualizer]);

  return (
    <ComboboxList
      ref={setScrollElement}
      data-slot="journal-options"
      xstyle={[styles.journalOptions, styles.journalOptionsVirtualized]}
    >
      {filteredIds.length > 0 && (
        <div
          role="presentation"
          {...stylex.props(styles.journalRows(`${virtualizer.getTotalSize()}px`))}
        >
          {virtualizer.getVirtualItems().map((row) => {
            const id = filteredIds[row.index];
            const label = journalLabelMap.get(id) ?? id;
            return (
              <ComboboxItem
                key={row.key}
                ref={virtualizer.measureElement}
                index={row.index}
                data-index={row.index}
                value={id}
                title={label}
                aria-setsize={filteredIds.length}
                aria-posinset={row.index + 1}
                xstyle={styles.journalRow(`${row.start}px`)}
              >
                {label}
              </ComboboxItem>
            );
          })}
        </div>
      )}
    </ComboboxList>
  );
}

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
  const virtualizerRef = useRef<JournalOptionVirtualizer | null>(null);
  const optionIds =
    isOpen && journalOptions
      ? journalOptions.map((option) => String(option.journal_id))
      : NO_OPTION_IDS;
  const selectedIds = new Set(journalIds);
  const orderedJournalIds = [
    ...optionIds.filter((id) => selectedIds.has(id)),
    ...optionIds.filter((id) => !selectedIds.has(id)),
  ];
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
      modal
      virtualized
      open={isOpen}
      onOpenChange={setIsOpen}
      items={orderedJournalIds}
      value={journalIds}
      onValueChange={(ids: string[]) => handleJournalsChange(ids)}
      onInputValueChange={() => virtualizerRef.current?.scrollToOffset(0)}
      onItemHighlighted={(_id, details) =>
        scrollHighlightedJournal(virtualizerRef.current, details)
      }
      itemToStringLabel={(id: string) => journalLabelMap.get(id) ?? id}
    >
      <ComboboxTrigger
        aria-label={accessibleName}
        title={accessibleName}
        data-active={isActive ? 'true' : 'false'}
        {...stylex.props(filterPillStyles(isActive))}
      >
        <FilterPillLabel text={visibleText} />
      </ComboboxTrigger>
      <ComboboxContent aria-label="期刊" xstyle={styles.journalFacetComboboxContent}>
        <FilterPanelHeader
          label="期刊"
          description={`在 ${journalOptions?.length ?? 0} 本期刊中选择。`}
          isActive={isActive}
          onClear={handleClearJournals}
        />
        <RevealItem
          isReady={!loadingJournals}
          xstyle={styles.renderRatingFacetRevealItem}
          placeholder={<Skeleton xstyle={styles.timeFacetSkeleton} />}
        >
          <ComboboxInput aria-label="搜索期刊" name="journal_search" placeholder="搜索期刊" />
          <ComboboxEmpty>未找到期刊。</ComboboxEmpty>
          <JournalOptionList journalLabelMap={journalLabelMap} virtualizerRef={virtualizerRef} />
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
  const areaCounts = new Map((areaOptions ?? []).map((option) => [option.value, option.count]));
  const areaValues = (areaOptions ?? []).map((option) => option.value);
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
      modal
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
        {...stylex.props(filterPillStyles(isActive))}
      >
        <FilterPillLabel text={visibleText} />
      </ComboboxTrigger>
      <ComboboxContent aria-label="领域" xstyle={styles.renderRatingFacetMultiSelectContent}>
        <FilterPanelHeader
          label="领域"
          description="可多选，满足任一领域即显示。"
          isActive={isActive}
          onClear={handleClearAreas}
        />
        <RevealItem
          isReady={!loadingAreas}
          xstyle={styles.renderRatingFacetRevealItem}
          placeholder={<Skeleton xstyle={styles.timeFacetSkeleton} />}
        >
          {areaValues.length > 10 && <ComboboxInput aria-label="搜索领域" placeholder="搜索领域" />}
          <ComboboxEmpty>未找到领域。</ComboboxEmpty>
          <ComboboxList data-slot="area-options" xstyle={styles.areaOptions}>
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
  const defaultStartMonth = yearBounds ? monthKey(yearBounds.min, 1) : null;
  const defaultEndMonth = yearBounds ? monthKey(yearBounds.max, 12) : null;
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

const styles = stylex.create({
  layoutFreeGroup: {
    display: 'contents',
  },
  statusOutput: {
    display: 'block',
  },
  dateSegmentSelectSelectTrigger: {
    width: '100%',
    justifyContent: 'space-between',
    paddingInline: '0.625rem',
    fontVariantNumeric: 'tabular-nums',
  },
  dateSegmentSelectSelectContent: {
    maxHeight: '16rem',
  },
  dateSegmentSelectSelectItem: {
    fontVariantNumeric: 'tabular-nums',
  },
  searchFiltersDiv: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
      '@media (min-width: 48rem)': '0px',
    },
    marginBlock: '-0.25rem',
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    overflowX: 'auto',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
      '@media (min-width: 48rem)': '0px',
    },
    paddingBlock: '0.25rem',
    scrollbarWidth: 'none',
    '::-webkit-scrollbar': {
      display: 'none',
    },
  },
  searchFiltersSpan: {
    height: '1.25rem',
    width: '1px',
    flexShrink: 0,
    backgroundColor: colors.border,
  },
  searchFiltersButton: {
    display: 'inline-flex',
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.25rem',
    borderRadius: 'calc(infinity * 1px)',
    paddingInline: '0.625rem',
    fontSize: '13px',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  searchFiltersX: {
    width: '0.875rem',
    height: '0.875rem',
  },
  renderDatabaseFacetRevealItem: {
    flexShrink: 0,
  },
  renderDatabaseFacetSkeleton: {
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    width: '10rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  timeFacetSkeleton: {
    margin: '0.5rem',
    height: '6rem',
  },
  timeFacetP: {
    paddingInline: '0.625rem',
    paddingBlock: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  timeFacetDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  timeFacetRadioGroup: {
    display: 'grid',
    gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
    gap: '0.375rem',
    padding: '0.25rem',
  },
  timeFacetRadioGroupCard: {
    height: {
      default: '2.5rem',
      '@media (min-width: 48rem)': '2.25rem',
    },
  },
  timeFacetDiv: {
    borderRadius: '8px',
    backgroundColor: colors.muted,
    padding: '0.75rem',
  },
  timeFacetP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  timeFacetDiv2: {
    display: 'grid',
    gridTemplateColumns: 'minmax(0,1.3fr) minmax(0,1fr)',
    gap: '0.5rem',
  },
  renderRatingFacetSpan: {
    minWidth: '0px',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  renderRatingFacetChevronDown: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    opacity: '70%',
  },
  renderRatingFacetMultiSelectContent: {
    width: '22rem',
    maxWidth: 'calc(100vw - 2rem)',
  },
  renderRatingFacetRevealItem: {
    display: 'flex',
    minHeight: '0px',
    flexDirection: 'column',
  },
  renderRatingFacetDiv: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingInline: '1rem',
    paddingBlock: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderRatingFacetMultiSelectList: {
    maxHeight: 'min(60vh, 26rem)',
  },
  renderRatingFacetP: {
    paddingInline: '1rem',
    paddingBlock: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  journalFacetComboboxContent: {
    width: '24rem',
    maxWidth: 'calc(100vw - 2rem)',
  },
  journalOptions: {
    maxHeight: 'min(50vh, 18rem)',
  },
  journalOptionsVirtualized: {
    paddingBlock: 0,
  },
  journalRows: (height: string) => ({
    position: 'relative',
    width: '100%',
    height,
  }),
  journalRow: (offset: string) => ({
    position: 'absolute',
    top: offset,
    left: 0,
  }),
  areaOptions: {
    maxHeight: 'min(55vh, 22rem)',
  },
});
