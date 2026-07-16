'use client';

/**
 * Visible, individually removable feedback for applied article filters.
 */

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { parseAsArrayOf, parseAsString, useQueryState } from 'nuqs';
import { X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { getAreaDisplayName } from '@/lib/area-labels';
import { getJournalOptions } from '@/lib/api';
import { formatMonthRangeLabel } from '@/lib/article-filters';
import { useSelectedDatabase } from '@/lib/selected-database';

type FilterChipProps = {
  label: string;
  onRemove: () => void;
  removeLabel: string;
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
  const activeFilterCount =
    (appliedQuery ? 1 : 0) + areas.length + journalIds.length + (monthRangeLabel ? 1 : 0);

  if (activeFilterCount === 0) {
    return null;
  }

  return (
    <section
      data-testid="active-filter-chips"
      aria-label="已应用筛选"
      className="flex flex-wrap items-center gap-2 rounded-lg border bg-muted/30 px-3 py-2"
    >
      <span className="text-xs font-medium text-muted-foreground">已应用</span>
      {appliedQuery && (
        <FilterChip
          label={`搜索：${appliedQuery}`}
          removeLabel={`移除搜索 ${appliedQuery}`}
          onRemove={() => void setQ(null)}
        />
      )}
      {areas.map((area) => {
        const label = getAreaDisplayName(area);
        return (
          <FilterChip
            key={`area-${area}`}
            label={`领域：${label}`}
            removeLabel={`移除领域 ${label}`}
            onRemove={() => void setAreas((current) => current.filter((item) => item !== area))}
          />
        );
      })}
      {journalIds.map((journalId) => {
        const label = journalLabels.get(journalId) ?? journalId;
        return (
          <FilterChip
            key={`journal-${journalId}`}
            label={`期刊：${label}`}
            removeLabel={`移除期刊 ${label}`}
            onRemove={() =>
              void setJournalIds((current) => current.filter((item) => item !== journalId))
            }
          />
        );
      })}
      {monthRangeLabel && (
        <FilterChip
          label={`时间：${monthRangeLabel}`}
          removeLabel={`移除时间 ${monthRangeLabel}`}
          onRemove={() => void setMonthRange(null)}
        />
      )}
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
    </section>
  );
}
