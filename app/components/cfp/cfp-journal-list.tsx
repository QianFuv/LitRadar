'use client';

/** Collapsible journal groups with independent scrolling and server-owned availability. */

import { useId, useState } from 'react';
import { ChevronDown } from 'lucide-react';

import { READER_ITEM_ATTRIBUTE } from '@/components/feature/article-reader-context';
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
  onIntent,
  onSelect,
}: {
  groups: readonly CfpJournalGroup[];
  selectedCatalogId?: string;
  isSearching: boolean;
  /** Called when the pointer or focus suggests a journal is about to be opened. */
  onIntent?: (catalogId: string) => void;
  onSelect: (catalogId: string) => void;
}) {
  const regionId = useId();
  const [expandedGroups, setExpandedGroups] = useState<Record<CfpJournalGroupId, boolean>>({
    current: true,
    inactive: isSearching,
    unadapted: isSearching,
  });
  const [mountedGroups, setMountedGroups] = useState(expandedGroups);

  return (
    <div data-slot="cfp-journal-groups" className="flex min-h-0 flex-1 flex-col gap-1">
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
              'flex min-h-0 flex-col overflow-hidden',
              group.id === 'current' && 'mb-auto',
              isExpanded ? 'flex-1' : 'shrink-0',
            )}
          >
            <button
              type="button"
              aria-expanded={isExpanded}
              aria-controls={panelId}
              onClick={() => {
                setExpandedGroups((previous) => ({ ...previous, [group.id]: !isExpanded }));
                if (!mountedGroups[group.id]) {
                  setMountedGroups((previous) => ({ ...previous, [group.id]: true }));
                }
              }}
              className="motion-control flex min-h-10 w-full shrink-0 items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xs font-bold text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
            >
              <ChevronDown
                className={cn(
                  'size-3.5 shrink-0 transition-transform',
                  !isExpanded && '-rotate-90',
                )}
                aria-hidden="true"
              />
              <span className="min-w-0 flex-1">{group.label}</span>
              <span className="rounded-full bg-muted px-1.5 font-normal tabular-nums text-muted-foreground">
                {group.journals.length}
              </span>
            </button>
            <div
              id={panelId}
              role="region"
              aria-label={`${group.label}期刊`}
              hidden={!isExpanded}
              data-slot="cfp-journal-list"
              className="min-h-0 flex-1 overflow-y-auto overscroll-contain pr-3 pb-1 pl-1"
            >
              {mountedGroups[group.id] &&
                (group.journals.length === 0 ? (
                  <p className="px-2 py-3 text-xs leading-5 text-muted-foreground">
                    暂无符合条件的期刊
                  </p>
                ) : (
                  <ul className="divide-y divide-border/70">
                    {group.journals.map((journal) => {
                      const isSelected = selectedCatalogId === journal.catalogId;
                      const isCurrent = journal.coverage === 'adapted' && journal.currentCount > 0;
                      return (
                        <li key={journal.catalogId}>
                          <button
                            type="button"
                            {...{ [READER_ITEM_ATTRIBUTE]: '' }}
                            data-selected={isSelected ? 'true' : 'false'}
                            aria-pressed={isSelected}
                            title={journal.title}
                            onClick={() => onSelect(journal.catalogId)}
                            onPointerEnter={() => onIntent?.(journal.catalogId)}
                            onFocus={() => onIntent?.(journal.catalogId)}
                            className={cn(
                              'motion-control relative my-1 flex w-full items-start gap-3 rounded-lg px-2.5 py-3 text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset',
                              isSelected && 'bg-accent hover:bg-accent',
                            )}
                          >
                            <span
                              aria-hidden="true"
                              className={cn(
                                'flex size-9 shrink-0 flex-col items-center justify-center rounded-lg text-[13px] leading-none font-bold tabular-nums',
                                isCurrent
                                  ? 'bg-success text-success-foreground'
                                  : 'bg-muted text-muted-foreground',
                              )}
                            >
                              {journal.coverage === 'unadapted' ? '–' : journal.currentCount}
                            </span>
                            <span className="min-w-0 flex-1">
                              <span
                                className={cn(
                                  'line-clamp-2 text-[13.5px] leading-5 [overflow-wrap:anywhere]',
                                  isSelected || isCurrent ? 'font-bold' : 'text-foreground/85',
                                )}
                              >
                                {journal.title}
                              </span>
                              <span className="mt-1 block truncate text-[11px] leading-4 text-muted-foreground">
                                {journal.coverage === 'unadapted'
                                  ? '暂未适配'
                                  : journal.currentCount > 0
                                    ? `${journal.currentCount} 条当前征稿`
                                    : journal.noticeCount > 0
                                      ? `${journal.noticeCount} 条历史征稿`
                                      : '暂无征稿'}
                                {journal.allIssns[0] ? ` · ${journal.allIssns[0]}` : ''}
                              </span>
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
