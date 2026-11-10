/**
 * Visual advanced-search builder: add, edit, and remove conditions without writing syntax.
 */

import { ChevronRight, Plus, Trash2 } from '@/components/ui/animated-icons';
import { useState, type FormEvent } from 'react';

import {
  Root as Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@animate-ui/react/primitives/radix-ui/collapsible';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { DISCLOSURE_VARIANTS, MotionListItem, MotionPresence } from '@/components/ui/motion';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  compileSearchConditions,
  createSearchCondition,
  readStoredSearchBuilder,
  SEARCH_BUILDER_CONNECTORS,
  SEARCH_BUILDER_FIELDS,
  SEARCH_BUILDER_MATCHES,
  SEARCH_BUILDER_MAX_CONDITIONS,
  type SearchBuilderCondition,
} from '@/lib/search-builder';
import { isAdvancedSearchQuery } from '@/lib/search-syntax';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type SearchBuilderDialogProps = {
  /** Query currently applied in the URL. */
  currentQuery: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Run a compiled query and remember the rows that produced it. */
  onSearch: (query: string, conditions: SearchBuilderCondition[]) => void;
};

/**
 * Choose the rows shown when the builder opens.
 *
 * @param currentQuery - Applied query.
 * @returns Initial rows and whether a handwritten expression will be replaced.
 */
function getInitialConditions(currentQuery: string): {
  conditions: SearchBuilderCondition[];
  replacesHandwritten: boolean;
} {
  const query = currentQuery.trim();
  const stored = readStoredSearchBuilder();
  if (stored && query && stored.query === query) {
    return { conditions: stored.conditions, replacesHandwritten: false };
  }
  if (query && !isAdvancedSearchQuery(query)) {
    return {
      conditions: [createSearchCondition({ value: query }), createSearchCondition()],
      replacesHandwritten: false,
    };
  }
  return {
    conditions: [createSearchCondition(), createSearchCondition()],
    replacesHandwritten: Boolean(query),
  };
}

/**
 * Render the advanced-search builder dialog.
 *
 * @param props - Applied query, open state, and search callback.
 * @returns Builder dialog.
 */
export function SearchBuilderDialog({
  currentQuery,
  open,
  onOpenChange,
  onSearch,
}: SearchBuilderDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent xstyle={styles.searchBuilderDialogDialogContent}>
        <DialogHeader xstyle={styles.searchBuilderDialogDialogHeader}>
          <DialogTitle>高级检索</DialogTitle>
          <DialogDescription>
            按条件组合检索，只需填写检索词，系统会自动生成正确的检索式。
          </DialogDescription>
        </DialogHeader>
        {open && (
          <SearchBuilderBody
            currentQuery={currentQuery}
            onCancel={() => onOpenChange(false)}
            onSearch={onSearch}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/**
 * Render builder rows, the generated expression, and actions.
 *
 * @param props - Applied query, cancel action, and search callback.
 * @returns Builder form.
 */
function SearchBuilderBody({
  currentQuery,
  onCancel,
  onSearch,
}: {
  currentQuery: string;
  onCancel: () => void;
  onSearch: (query: string, conditions: SearchBuilderCondition[]) => void;
}) {
  const [initialState] = useState(() => getInitialConditions(currentQuery));
  const [conditions, setConditions] = useState(initialState.conditions);
  const result = compileSearchConditions(conditions);
  const activeConditions = conditions.filter((condition) => condition.value.trim());

  /**
   * Update one row.
   *
   * @param id - Row id.
   * @param patch - Changed fields.
   */
  const updateCondition = (id: string, patch: Partial<SearchBuilderCondition>) => {
    setConditions((current) =>
      current.map((condition) => (condition.id === id ? { ...condition, ...patch } : condition)),
    );
  };

  /**
   * Remove one row, keeping at least one editable row.
   *
   * @param id - Row id.
   */
  const removeCondition = (id: string) => {
    setConditions((current) => {
      const next = current.filter((condition) => condition.id !== id);
      return next.length > 0 ? next : [createSearchCondition()];
    });
  };

  /**
   * Run the compiled query.
   *
   * @param event - Form submission.
   */
  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (result.error) return;
    onSearch(
      result.query,
      conditions.filter((condition) => condition.value.trim()),
    );
  };

  return (
    <form onSubmit={handleSubmit} {...stylex.props(styles.searchBuilderBodyForm)}>
      <div {...stylex.props(styles.searchBuilderBodyDiv)}>
        {initialState.replacesHandwritten && (
          <p {...stylex.props(styles.searchBuilderBodyP)}>
            当前检索式是手写语法，构建器会生成新的检索式替换它。
          </p>
        )}
        <ol aria-label="检索条件">
          <MotionPresence>
            {conditions.map((condition, index) => {
              const position = index + 1;
              return (
                <MotionListItem
                  key={condition.id}
                  data-search-condition={condition.id}
                  {...stylex.props(styles.conditionRow, styles.searchBuilderBodyMotionListItem)}
                  variants={DISCLOSURE_VARIANTS}
                  initial="hidden"
                  animate="visible"
                  exit="exit"
                >
                  <div {...stylex.props(styles.searchBuilderBodyDiv2)}>
                    {index === 0 && condition.connector !== 'NOT' ? (
                      <span {...stylex.props(styles.searchBuilderBodySpan)}>检索</span>
                    ) : (
                      <ToggleGroup
                        type="single"
                        aria-label={`第 ${position} 个条件的连接方式`}
                        xstyle={styles.searchBuilderBodyToggleGroup}
                        value={condition.connector}
                        onValueChange={(connector) => {
                          if (connector) {
                            updateCondition(condition.id, {
                              connector: connector as typeof condition.connector,
                            });
                          }
                        }}
                      >
                        {SEARCH_BUILDER_CONNECTORS.map((connector) => (
                          <ToggleGroupItem
                            key={connector.value}
                            value={connector.value}
                            xstyle={[
                              styles.searchBuilderBodyToggleGroupItem,
                              connector.value === 'NOT' &&
                                styles.searchBuilderBodyToggleGroupItemNOT,
                            ]}
                          >
                            {connector.label}
                          </ToggleGroupItem>
                        ))}
                      </ToggleGroup>
                    )}
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      xstyle={styles.searchBuilderBodyButton}
                      aria-label={`删除第 ${position} 个条件`}
                      onClick={() => removeCondition(condition.id)}
                    >
                      <Trash2
                        {...stylex.props(styles.searchBuilderBodyTrash2)}
                        aria-hidden="true"
                      />
                    </Button>
                    <div {...stylex.props(styles.searchBuilderBodyDiv3)}>
                      <Select
                        value={condition.field}
                        onValueChange={(value) =>
                          updateCondition(condition.id, {
                            field: value as SearchBuilderCondition['field'],
                          })
                        }
                      >
                        <SelectTrigger
                          size="sm"
                          aria-label={`第 ${position} 个条件的字段`}
                          xstyle={styles.searchBuilderBodySelectTrigger}
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {SEARCH_BUILDER_FIELDS.map((field) => (
                            <SelectItem key={field.value} value={field.value}>
                              {field.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <Select
                        value={condition.match}
                        onValueChange={(value) =>
                          updateCondition(condition.id, {
                            match: value as SearchBuilderCondition['match'],
                          })
                        }
                      >
                        <SelectTrigger
                          size="sm"
                          aria-label={`第 ${position} 个条件的匹配方式`}
                          xstyle={styles.searchBuilderBodySelectTrigger}
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {SEARCH_BUILDER_MATCHES.map((match) => (
                            <SelectItem key={match.value} value={match.value}>
                              {match.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <Input
                      aria-label={`第 ${position} 个条件的检索词`}
                      autoComplete="off"
                      spellCheck={false}
                      placeholder={index === 0 ? '例如：graph neural network' : '输入检索词'}
                      value={condition.value}
                      autoFocus={index === 0 && !condition.value}
                      onChange={(event) =>
                        updateCondition(condition.id, { value: event.target.value })
                      }
                      xstyle={styles.searchBuilderBodyInput}
                    />
                  </div>
                </MotionListItem>
              );
            })}
          </MotionPresence>
        </ol>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          xstyle={styles.searchBuilderBodyButton2}
          disabled={conditions.length >= SEARCH_BUILDER_MAX_CONDITIONS}
          onClick={() => setConditions((current) => [...current, createSearchCondition()])}
        >
          <Plus {...stylex.props(styles.searchBuilderBodyTrash2)} aria-hidden="true" />
          添加条件
        </Button>

        {result.error && activeConditions.length > 0 && (
          <p role="alert" {...stylex.props(styles.searchBuilderBodyP2)}>
            {result.error}
          </p>
        )}

        <Collapsible {...stylex.props(styles.searchBuilderBodyCollapsible)}>
          <CollapsibleTrigger
            {...stylex.props(styles.searchBuilderBodyCollapsibleTrigger, stylex.defaultMarker())}
          >
            <ChevronRight
              {...stylex.props(styles.searchBuilderBodyChevronRight)}
              aria-hidden="true"
            />
            <span {...stylex.props(styles.searchBuilderBodySpan2)}>点击展开手写检索语法</span>
            <span {...stylex.props(styles.searchBuilderBodySpan3)}>收起手写检索语法</span>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <p {...stylex.props(styles.searchBuilderBodyP3)}>
              在搜索框中直接输入以下语法同样有效：
            </p>
            <ul {...stylex.props(styles.searchBuilderBodyUl)}>
              <li>
                <code>a AND b</code> 同时包含
              </li>
              <li>
                <code>a OR b</code> 任一包含
              </li>
              <li>
                <code>a NOT b</code> 排除 b
              </li>
              <li>
                <code>&quot;exact phrase&quot;</code> 精确短语
              </li>
              <li>
                <code>bio*</code> 前缀匹配
              </li>
              <li>
                <code>NEAR(gene therapy, 5)</code> 相距 5 词内
              </li>
              <li>
                <code>title:diabetes</code> 标题
              </li>
              <li>
                <code>authors:&quot;Smith&quot;</code> 作者
              </li>
              <li>
                <code>journal_title:&quot;Nature&quot;</code> 期刊
              </li>
              <li>
                <code>abstract_text:imaging</code> 摘要
              </li>
            </ul>
            <p {...stylex.props(styles.searchBuilderBodyP4)}>
              运算符 AND / OR / NOT / NEAR 需要大写。
            </p>
          </CollapsibleContent>
        </Collapsible>
      </div>
      <div {...stylex.props(styles.searchBuilderBodyDiv4)}>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          xstyle={styles.searchBuilderBodyButton3}
          onClick={() => setConditions([createSearchCondition(), createSearchCondition()])}
        >
          清空条件
        </Button>
        <Button type="button" variant="outline" onClick={onCancel}>
          取消
        </Button>
        <Button
          type="submit"
          disabled={Boolean(result.error)}
          xstyle={styles.searchBuilderBodyButton4}
        >
          检索
        </Button>
      </div>
    </form>
  );
}

const styles = stylex.create({
  conditionRow: {
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
  },
  searchBuilderDialogDialogContent: {
    display: 'flex',
    maxHeight: '90dvh',
    flexDirection: 'column',
    gap: '0px',
    padding: '0px',
    maxWidth: {
      default: 'none',
      '@media (min-width: 48rem)': '48rem',
    },
  },
  searchBuilderDialogDialogHeader: {
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    paddingInline: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '1.5rem',
    },
    paddingTop: '1.25rem',
    paddingBottom: '1rem',
    textAlign: 'left',
  },
  searchBuilderBodyForm: {
    display: 'flex',
    minHeight: '0px',
    flex: '1',
    flexDirection: 'column',
  },
  searchBuilderBodyDiv: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
    paddingInline: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '1.5rem',
    },
    paddingBlock: '1rem',
  },
  searchBuilderBodyP: {
    marginBottom: '0.75rem',
    borderRadius: '8px',
    backgroundColor: colors.muted,
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    color: colors.mutedForeground,
  },
  searchBuilderBodyMotionListItem: {
    overflow: 'hidden',
  },
  searchBuilderBodyDiv2: {
    display: 'grid',
    gridTemplateColumns: {
      default: 'minmax(0,1fr) auto',
      '@media (min-width: 40rem)': '8rem 7.5rem 8.75rem minmax(0,1fr) auto',
    },
    alignItems: 'center',
    gap: '0.5rem',
    paddingBlock: '0.75rem',
  },
  searchBuilderBodySpan: {
    display: 'flex',
    height: '2.25rem',
    alignItems: 'center',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
    color: colors.mutedForeground,
  },
  searchBuilderBodyToggleGroup: {
    display: 'inline-flex',
    height: '2.25rem',
    gap: '0.125rem',
    borderStyle: 'solid',
    borderWidth: '0px',
    backgroundColor: colors.muted,
  },
  searchBuilderBodyToggleGroupItem: {
    height: {
      default: '2rem',
      '@media (min-width: 48rem)': '2rem',
    },
    paddingInline: {
      default: '0.625rem',
      '@media (min-width: 48rem)': '0.625rem',
    },
    fontSize: '13px',
    whiteSpace: 'nowrap',
    backgroundColor: {
      default: null,
      ':is([data-state="on"])': colors.background,
    },
    fontWeight: {
      default: null,
      ':is([data-state="on"])': 700,
    },
    color: {
      default: colors.mutedForeground,
      ':is([data-state="on"])': colors.foreground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': {
        default: `0 0 0 2px ${colors.ring}`,
        ':is([data-state="on"])': `0 0 0 2px ${colors.ring}, 0 0 0 1px ${colors.border}`,
      },
      ':is([data-state="on"])': `0 0 0 1px ${colors.border}`,
    },
    lineHeight: 'inherit',
  },
  searchBuilderBodyToggleGroupItemNOT: {
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': {
          default: colors.foreground,
          ':is([data-state="on"])': colors.destructive,
        },
      },
      ':is([data-state="on"])': colors.destructive,
    },
  },
  searchBuilderBodyButton: {
    width: {
      default: '2.75rem',
      '@media (min-width: 40rem)': '2.25rem',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 40rem)': '2.25rem',
    },
    justifySelf: 'flex-end',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.destructive,
      },
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
      },
    },
    order: {
      default: null,
      '@media (min-width: 40rem)': 9999,
    },
  },
  searchBuilderBodyTrash2: {
    width: '1rem',
    height: '1rem',
  },
  searchBuilderBodyDiv3: {
    gridColumnStart: 'span 2',
    gridColumnEnd: 'span 2',
    display: {
      default: 'grid',
      '@media (min-width: 40rem)': 'contents',
    },
    gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
    gap: '0.5rem',
  },
  searchBuilderBodySelectTrigger: {
    width: '100%',
  },
  searchBuilderBodyInput: {
    gridColumnStart: {
      default: 'span 2',
      '@media (min-width: 40rem)': 'span 1',
    },
    gridColumnEnd: {
      default: 'span 2',
      '@media (min-width: 40rem)': 'span 1',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 40rem)': '2.25rem',
    },
  },
  searchBuilderBodyButton2: {
    marginTop: '0.25rem',
    marginLeft: '-0.5rem',
    color: {
      default: colors.primary,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.primary,
      },
    },
  },
  searchBuilderBodyP2: {
    marginTop: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.destructive,
  },
  searchBuilderBodyCollapsible: {
    marginTop: '1rem',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingTop: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  searchBuilderBodyCollapsibleTrigger: {
    display: 'flex',
    width: 'fit-content',
    cursor: 'pointer',
    alignItems: 'center',
    gap: '0.25rem',
    fontWeight: 700,
    color: {
      default: `color-mix(in oklab, ${colors.foreground} 80%, transparent)`,
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
  searchBuilderBodyChevronRight: {
    width: '0.875rem',
    height: '0.875rem',
    transitionProperty: 'transform, translate, scale, rotate',
    transitionTimingFunction: 'cubic-bezier(0.215, 0.61, 0.355, 1)',
    transitionDuration: '350ms',
    rotate: {
      default: null,
      [stylex.when.ancestor('[data-state="open"]')]: '90deg',
    },
  },
  searchBuilderBodySpan2: {
    display: {
      default: null,
      [stylex.when.ancestor('[data-state="open"]')]: 'none',
    },
  },
  searchBuilderBodySpan3: {
    display: {
      default: 'none',
      [stylex.when.ancestor('[data-state="open"]')]: 'inline',
    },
  },
  searchBuilderBodyP3: {
    paddingTop: '0.5rem',
  },
  searchBuilderBodyUl: {
    marginTop: '0.5rem',
    display: 'grid',
    gap: '0.25rem',
    gridTemplateColumns: {
      default: null,
      '@media (min-width: 40rem)': 'repeat(2, minmax(0, 1fr))',
    },
  },
  searchBuilderBodyP4: {
    marginTop: '0.5rem',
  },
  searchBuilderBodyDiv4: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingInline: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '1.5rem',
    },
    paddingBlock: '0.75rem',
  },
  searchBuilderBodyButton3: {
    marginRight: 'auto',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  searchBuilderBodyButton4: {
    backgroundColor: {
      default: colors.foreground,
      ':active': {
        default: colors.primaryPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.primaryPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
      },
    },
    color: colors.background,
  },
});
