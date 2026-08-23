'use client';

/**
 * Visible, individually removable feedback for applied article filters.
 */

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { parseAsArrayOf, parseAsString, useQueryState } from 'nuqs';
import { X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionPresence,
  MotionSection,
  useMotionTransition,
} from '@/components/ui/motion';
import { getAreaDisplayName } from '@/lib/area-labels';
import { getJournalOptions } from '@/lib/api';
import { formatMonthRangeLabel } from '@/lib/article-filters';
import { useSelectedDatabase } from '@/lib/selected-database';

type FilterChipProps = {
  label: string;
  onRemove: () => void;
  removeLabel: string;
};

type AppliedFilterChip = FilterChipProps & {
  id: string;
};

const FILTER_CHIP_VARIANTS = {
  hidden: { opacity: 0, scale: 0.96 },
  visible: { opacity: 1, scale: 1 },
  exit: { opacity: 0, scale: 0.96 },
};

/**
 * Render one compact filter chip with an accessible removal action.
 *
 * @param props - Display label, removal label, and callback.
 * @returns Removable filter chip.
 */
function FilterChip({ label, onRemove, removeLabel }: FilterChipProps) {
  return (
    <Button
      type="button"
      variant="secondary"
      size="xs"
      className="h-7 max-w-full rounded-full px-2.5"
      aria-label={removeLabel}
      title={removeLabel}
      onClick={onRemove}
    >
      <span className="truncate">{label}</span>
      <X className="h-3 w-3" />
    </Button>
  );
}

/**
 * Render all applied homepage filters and an explicit reset action.
 *
 * @returns Active filter feedback, or null when no filter is applied.
 */
export function ActiveFilterChips() {
  const [q, setQ] = useQueryState('q', parseAsString);
  const [areas, setAreas] = useQueryState('area', parseAsArrayOf(parseAsString).withDefault([]));
  const [journalIds, setJournalIds] = useQueryState(
    'journal_id',
    parseAsArrayOf(parseAsString).withDefault([]),
  );
  const [monthRange, setMonthRange] = useQueryState('month_range', parseAsString);
  const currentDatabase = useSelectedDatabase();
  const { data: journalOptions = [] } = useQuery({
    queryKey: ['meta', 'journals', currentDatabase],
    queryFn: () => getJournalOptions(currentDatabase),
    enabled: journalIds.length > 0,
  });
  const journalLabels = useMemo(
    () =>
      new Map(
        journalOptions.map((option) => [
          String(option.journal_id),
          option.title ?? String(option.journal_id),
        ]),
      ),
    [journalOptions],
  );
  const appliedQuery = q?.trim() ?? '';
  const monthRangeLabel = formatMonthRangeLabel(monthRange);
  const transition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const appliedFilters: AppliedFilterChip[] = [];

  if (appliedQuery) {
    appliedFilters.push({
      id: `query-${appliedQuery}`,
      label: `搜索：${appliedQuery}`,
      removeLabel: `移除搜索 ${appliedQuery}`,
      onRemove: () => void setQ(null),
    });
  }
  for (const area of areas) {
    const label = getAreaDisplayName(area);
    appliedFilters.push({
      id: `area-${area}`,
      label: `领域：${label}`,
      removeLabel: `移除领域 ${label}`,
      onRemove: () => void setAreas((current) => current.filter((item) => item !== area)),
    });
  }
  for (const journalId of journalIds) {
    const label = journalLabels.get(journalId) ?? journalId;
    appliedFilters.push({
      id: `journal-${journalId}`,
      label: `期刊：${label}`,
      removeLabel: `移除期刊 ${label}`,
      onRemove: () => void setJournalIds((current) => current.filter((item) => item !== journalId)),
    });
  }
  if (monthRangeLabel) {
    appliedFilters.push({
      id: `month-${monthRange}`,
      label: `时间：${monthRangeLabel}`,
      removeLabel: `移除时间 ${monthRangeLabel}`,
      onRemove: () => void setMonthRange(null),
    });
  }

  return (
    <MotionPresence>
      {appliedFilters.length > 0 && (
        <MotionSection
          key="active-filter-summary"
          data-testid="active-filter-chips"
          aria-label="已应用筛选"
          className="flex flex-wrap items-center gap-1.5 rounded-md bg-muted/40 px-2.5 py-2 shadow-vercel-ring"
          variants={FADE_UP_VARIANTS}
          initial="hidden"
          animate="visible"
          exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
          transition={transition}
        >
          <span className="mr-0.5 text-xs font-medium text-muted-foreground">
            已应用 {appliedFilters.length}
          </span>
          <MotionPresence>
            {appliedFilters.map((filter) => (
              <MotionDiv
                key={filter.id}
                data-motion-filter-key={filter.id}
                className="max-w-full"
                variants={FILTER_CHIP_VARIANTS}
                initial="hidden"
                animate="visible"
                exit={{ opacity: 0, pointerEvents: 'none', scale: 0.96 }}
                transition={transition}
              >
                <FilterChip
                  label={filter.label}
                  removeLabel={filter.removeLabel}
                  onRemove={filter.onRemove}
                />
              </MotionDiv>
            ))}
          </MotionPresence>
          <Button
            type="button"
            variant="ghost"
            size="xs"
            className="ml-auto h-7"
            onClick={() => {
              void setQ(null);
              void setAreas([]);
              void setJournalIds([]);
              void setMonthRange(null);
            }}
          >
            重置筛选
          </Button>
        </MotionSection>
      )}
    </MotionPresence>
  );
}
