/** Collapsible journal groups with independent scrolling and server-owned availability. */

import { useId, useState } from 'react';
import { ChevronDown } from '@/components/ui/animated-icons';

import { READER_ITEM_ATTRIBUTE } from '@/components/feature/article-reader-context';
import type { CfpJournalSummary } from '@/lib/api';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

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
    <div data-slot="cfp-journal-groups" {...stylex.props(styles.cfpJournalGroups)}>
      {isSearching && groups.every((group) => group.journals.length === 0) && (
        <p {...stylex.props(styles.cfpJournalListP)}>未找到匹配期刊</p>
      )}
      {groups.map((group) => {
        const isExpanded = expandedGroups[group.id] && (!isSearching || group.journals.length > 0);
        const panelId = `${regionId}-${group.id}`;
        return (
          <section
            key={group.id}
            data-cfp-group={group.id}
            {...stylex.props(
              styles.cfpJournalListSection,
              group.id === 'current' && styles.cfpJournalListSectionCurrent,
              isExpanded
                ? styles.cfpJournalListSectionExpanded
                : styles.cfpJournalListSectionNotExpanded,
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
              {...stylex.props(styles.cfpJournalListButton)}
            >
              <ChevronDown
                xstyle={[
                  styles.cfpJournalListChevronDown,
                  !isExpanded && styles.cfpJournalListChevronDownExpanded,
                ]}
                aria-hidden="true"
              />
              <span {...stylex.props(styles.cfpJournalListSpan)}>{group.label}</span>
              <span {...stylex.props(styles.cfpJournalListSpan2)}>{group.journals.length}</span>
            </button>
            <div
              id={panelId}
              role="region"
              aria-label={`${group.label}期刊`}
              hidden={!isExpanded}
              data-slot="cfp-journal-list"
              {...stylex.props(styles.cfpJournalList)}
            >
              {mountedGroups[group.id] &&
                (group.journals.length === 0 ? (
                  <p {...stylex.props(styles.cfpJournalListP2)}>暂无符合条件的期刊</p>
                ) : (
                  <ul>
                    {group.journals.map((journal) => {
                      const isSelected = selectedCatalogId === journal.catalogId;
                      const isCurrent = journal.coverage === 'adapted' && journal.currentCount > 0;
                      return (
                        <li {...stylex.props(styles.cfpJournalListUlItem)} key={journal.catalogId}>
                          <button
                            type="button"
                            {...{ [READER_ITEM_ATTRIBUTE]: '' }}
                            data-selected={isSelected ? 'true' : 'false'}
                            aria-pressed={isSelected}
                            title={journal.title}
                            onClick={() => onSelect(journal.catalogId)}
                            onPointerEnter={() => onIntent?.(journal.catalogId)}
                            onFocus={() => onIntent?.(journal.catalogId)}
                            {...stylex.props(
                              styles.cfpJournalListButton2,
                              isSelected && styles.cfpJournalListButtonSelected,
                            )}
                          >
                            <span
                              aria-hidden="true"
                              {...stylex.props(
                                styles.cfpJournalListSpan3,
                                isCurrent
                                  ? styles.cfpJournalListSpanCurrent
                                  : styles.cfpJournalListSpanNotCurrent,
                              )}
                            >
                              {journal.coverage === 'unadapted' ? '–' : journal.currentCount}
                            </span>
                            <span {...stylex.props(styles.cfpJournalListSpan)}>
                              <span
                                {...stylex.props(
                                  styles.cfpJournalListSpan4,
                                  isSelected || isCurrent
                                    ? styles.cfpJournalListSpanCurrent2
                                    : styles.cfpJournalListSpanNotCurrent2,
                                )}
                              >
                                {journal.title}
                              </span>
                              <span {...stylex.props(styles.cfpJournalListSpan5)}>
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

const styles = stylex.create({
  cfpJournalGroups: {
    display: 'flex',
    minHeight: '0px',
    flex: '1',
    flexDirection: 'column',
    gap: '0.25rem',
  },
  cfpJournalListP: {
    paddingBlock: '0.75rem',
    textAlign: 'center',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  cfpJournalListSection: {
    display: 'flex',
    minHeight: '0px',
    flexDirection: 'column',
    overflow: 'hidden',
  },
  cfpJournalListSectionCurrent: {
    marginBottom: 'auto',
  },
  cfpJournalListSectionExpanded: {
    flex: '1',
  },
  cfpJournalListSectionNotExpanded: {
    flexShrink: 0,
  },
  cfpJournalListButton: {
    display: 'flex',
    minHeight: '2.5rem',
    width: '100%',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.5rem',
    borderRadius: '8px',
    paddingInline: '0.625rem',
    paddingBlock: '0.5rem',
    textAlign: 'left',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
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
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `inset 0 0 0 2px ${colors.ring}`,
    },
    outlineStyle: {
      default: null,
      ':focus-visible': 'none',
    },
  },
  cfpJournalListChevronDown: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    transitionProperty: 'transform, translate, scale, rotate',
    transitionTimingFunction: 'cubic-bezier(0.215, 0.61, 0.355, 1)',
    transitionDuration: '350ms',
  },
  cfpJournalListChevronDownExpanded: {
    rotate: 'calc(90deg * -1)',
  },
  cfpJournalListSpan: {
    minWidth: '0px',
    flex: '1',
  },
  cfpJournalListSpan2: {
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.muted,
    paddingInline: '0.375rem',
    fontWeight: 400,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  cfpJournalList: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
    overscrollBehavior: 'contain',
    paddingRight: '0.75rem',
    paddingBottom: '0.25rem',
    paddingLeft: '0.25rem',
  },
  cfpJournalListP2: {
    paddingInline: '0.5rem',
    paddingBlock: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    color: colors.mutedForeground,
  },
  cfpJournalListUlItem: {
    borderTopStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderTopWidth: {
      default: null,
      ':not(:last-child)': '0px',
    },
    borderBottomStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderBottomWidth: {
      default: null,
      ':not(:last-child)': '1px',
    },
    borderColor: {
      default: null,
      ':not(:last-child)': `color-mix(in oklab, ${colors.border} 70%, transparent)`,
    },
  },
  cfpJournalListButton2: {
    position: 'relative',
    marginBlock: '0.25rem',
    display: 'flex',
    width: '100%',
    alignItems: 'flex-start',
    gap: '0.75rem',
    borderRadius: '8px',
    paddingInline: '0.625rem',
    paddingBlock: '0.75rem',
    textAlign: 'left',
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `inset 0 0 0 2px ${colors.ring}`,
    },
    outlineStyle: {
      default: null,
      ':focus-visible': 'none',
    },
  },
  cfpJournalListButtonSelected: {
    backgroundColor: {
      default: colors.accent,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
  },
  cfpJournalListSpan3: {
    display: 'flex',
    width: '2.25rem',
    height: '2.25rem',
    flexShrink: 0,
    flexDirection: 'column',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
    fontSize: '13px',
    lineHeight: 1,
    fontWeight: 700,
    fontVariantNumeric: 'tabular-nums',
  },
  cfpJournalListSpanCurrent: {
    backgroundColor: colors.success,
    color: colors.successForeground,
  },
  cfpJournalListSpanNotCurrent: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  cfpJournalListSpan4: {
    overflow: 'hidden',
    display: '-webkit-box',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 2,
    fontSize: '13.5px',
    lineHeight: '1.25rem',
    overflowWrap: 'anywhere',
  },
  cfpJournalListSpanCurrent2: {
    fontWeight: 700,
  },
  cfpJournalListSpanNotCurrent2: {
    color: `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
  },
  cfpJournalListSpan5: {
    marginTop: '0.25rem',
    display: 'block',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '11px',
    lineHeight: '1rem',
    color: colors.mutedForeground,
  },
});
