'use client';

/** Collapsible journal groups with independent scrolling and server-owned availability. */

import { useId, useState } from 'react';
import { ChevronDown } from 'lucide-react';

import type { CfpJournalSummary } from '@/lib/api';
import { cn } from '@/lib/utils';

type CfpJournalGroupId = 'current' | 'inactive' | 'unadapted';

/** Ordered sidebar section and its alphabetically sorted journals. */
export type CfpJournalGroup = {
  id: CfpJournalGroupId;
  label: string;
  journals: CfpJournalSummary[];
};

const JOURNAL_COLLATOR = new Intl.Collator('zh-CN', { sensitivity: 'base', numeric: true });

/** Group backend current-notice counts without deriving dates or availability in the browser. */
export function groupCfpJournals(journals: readonly CfpJournalSummary[]): CfpJournalGroup[] {
  const groups: CfpJournalGroup[] = [
    { id: 'current', label: '正在征稿', journals: [] },
    { id: 'inactive', label: '当前未征稿', journals: [] },
    { id: 'unadapted', label: '暂未适配', journals: [] },
  ];
  for (const journal of journals) {
    const index = journal.coverage === 'unadapted' ? 2 : journal.currentCount > 0 ? 0 : 1;
    groups[index].journals.push(journal);
  }
  for (const group of groups) {
    group.journals.sort((first, second) => JOURNAL_COLLATOR.compare(first.title, second.title));
  }
  return groups;
}

/** Keep collapsed sections visible while expanded journal lists share the available height. */
export function CfpJournalList({
  groups,
  selectedCatalogId,
  isSearching,
  onSelect,
}: {
  groups: readonly CfpJournalGroup[];
  selectedCatalogId?: string;
  isSearching: boolean;
  onSelect: (catalogId: string) => void;
}) {
  const regionId = useId();
  const [expandedGroups, setExpandedGroups] = useState<Record<CfpJournalGroupId, boolean>>({
    current: true,
    inactive: isSearching,
    unadapted: isSearching,
  });

  return (
    <div data-slot="cfp-journal-groups" className="flex min-h-0 flex-1 flex-col gap-2">
      {isSearching && groups.every((group) => group.journals.length === 0) && (
        <p className="py-3 text-center text-xs text-muted-foreground">未找到匹配期刊</p>
      )}
      {groups.map((group) => {
        const isExpanded = expandedGroups[group.id] && (!isSearching || group.journals.length > 0);
        const panelId = `${regionId}-${group.id}`;
        return (
          <section
            key={group.id}
            data-cfp-group={group.id}
            className={cn(
              'flex min-h-0 flex-col overflow-hidden rounded-lg border border-sidebar-border',
              group.id === 'current' && 'mb-auto',
              isExpanded ? 'flex-1' : 'shrink-0',
            )}
          >
            <button
              type="button"
              aria-expanded={isExpanded}
              aria-controls={panelId}
              onClick={() =>
                setExpandedGroups((previous) => ({ ...previous, [group.id]: !isExpanded }))
              }
              className="flex min-h-11 w-full shrink-0 items-center gap-2 px-3 py-2 text-left text-xs font-semibold hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-inset"
            >
              <ChevronDown
                className={cn(
                  'size-3.5 shrink-0 transition-transform',
                  !isExpanded && '-rotate-90',
                )}
                aria-hidden="true"
              />
              <span className="min-w-0 flex-1">{group.label}</span>
              <span className="font-normal tabular-nums text-muted-foreground">
                {group.journals.length}
              </span>
            </button>
            <div
              id={panelId}
              role="region"
              aria-label={`${group.label}期刊`}
              hidden={!isExpanded}
              data-slot="cfp-journal-list"
              className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-1 pb-1"
            >
              {group.journals.length === 0 ? (
                <p className="px-3 py-4 text-xs leading-5 text-muted-foreground">
                  暂无符合条件的期刊
                </p>
              ) : (
                group.journals.map((journal) => (
                  <button
                    key={journal.catalogId}
                    type="button"
                    aria-pressed={selectedCatalogId === journal.catalogId}
                    title={journal.title}
                    onClick={() => onSelect(journal.catalogId)}
                    className={cn(
                      'motion-control grid min-h-20 w-full grid-cols-[minmax(0,1fr)_auto] items-start gap-x-3 rounded-md border border-transparent px-3 py-3 text-left transition-colors hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring/50',
                      selectedCatalogId === journal.catalogId &&
                        'border-sidebar-border bg-sidebar-accent',
                    )}
                  >
                    <span className="min-w-0 space-y-1">
                      <span className="line-clamp-2 text-xs font-medium leading-5 [overflow-wrap:anywhere]">
                        {journal.title}
                      </span>
                      <span className="block text-[11px] leading-4 text-muted-foreground">
                        {journal.coverage === 'unadapted'
                          ? '暂未适配'
                          : journal.currentCount > 0
                            ? `${journal.currentCount} 条当前征稿`
                            : journal.noticeCount > 0
                              ? `${journal.noticeCount} 条历史征稿`
                              : '暂无征稿'}
                      </span>
                    </span>
                    {journal.coverage === 'adapted' && (
                      <span className="min-w-6 rounded px-1 py-0.5 text-center text-[11px] leading-4 tabular-nums text-muted-foreground">
                        {journal.currentCount}
                      </span>
                    )}
                  </button>
                ))
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}
