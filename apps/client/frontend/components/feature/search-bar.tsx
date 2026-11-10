/**
 * URL-backed search form with keyboard history, an advanced-syntax assistant, and restrained motion.
 */

import { useSearch } from '@tanstack/react-router';
import { z } from 'zod';
import { Input } from '@/components/ui/input';
import { Search, X, Clock, SlidersHorizontal, Wand2 } from '@/components/ui/animated-icons';
import { Button } from '@/components/ui/button';
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover';
import { SearchBuilderDialog } from '@/components/feature/search-builder-dialog';
import { useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from 'react';
import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
  parseStoredJson,
} from '@/lib/browser-storage';
import { useUpdateWorkspaceSearch, WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { WORKSPACE_SEARCH_ATTRIBUTE } from '@/components/feature/article-reader-context';
import {
  compileSearchConditions,
  describeSearchCondition,
  describeSearchConnector,
  readStoredSearchBuilder,
  writeStoredSearchBuilder,
  type SearchBuilderCondition,
  type StoredSearchBuilder,
} from '@/lib/search-builder';
import {
  buildRewriteSuggestions,
  isAdvancedSearchQuery,
  type SearchRewriteSuggestion,
} from '@/lib/search-syntax';
import { MotionDiv, MotionPresence, SWAP_VARIANTS } from '@/components/ui/motion';
import * as stylex from '@stylexjs/stylex';
import { searchMarker } from '@/lib/markers.stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

const SEARCH_HISTORY_KEY = 'litradar:v1:search_history';
const LEGACY_SEARCH_HISTORY_KEY = 'search_history';
const MAX_HISTORY_ITEMS = 10;
const SEARCH_HISTORY_SCHEMA = z.array(z.string());
const SEARCH_HISTORY_LISTBOX_ID = 'search-history-listbox';

/**
 * Parse and validate serialized search history.
 *
 * @param value - Serialized history value.
 * @returns Search history or null when invalid.
 */
function parseSearchHistory(value: string): string[] | null {
  return parseStoredJson(value, SEARCH_HISTORY_SCHEMA);
}

/**
 * Read validated search history from local storage.
 *
 * @returns Stored search history or an empty list.
 */
function getSearchHistory(): string[] {
  if (typeof window === 'undefined') return [];
  const history = readLocalStorageValue(SEARCH_HISTORY_KEY);
  if (history) {
    const parsedHistory = parseSearchHistory(history);
    if (parsedHistory) {
      return parsedHistory;
    }
    removeLocalStorageValue(SEARCH_HISTORY_KEY);
    return [];
  }
  const legacyHistory = readLocalStorageValue(LEGACY_SEARCH_HISTORY_KEY);
  if (!legacyHistory) {
    return [];
  }
  const parsedLegacyHistory = parseSearchHistory(legacyHistory);
  if (parsedLegacyHistory) {
    writeLocalStorageValue(SEARCH_HISTORY_KEY, JSON.stringify(parsedLegacyHistory));
    removeLocalStorageValue(LEGACY_SEARCH_HISTORY_KEY);
    return parsedLegacyHistory;
  }
  removeLocalStorageValue(LEGACY_SEARCH_HISTORY_KEY);
  return [];
}

/**
 * Persist validated search history.
 *
 * @param history - Search history entries.
 */
function writeSearchHistory(history: string[]): void {
  writeLocalStorageValue(SEARCH_HISTORY_KEY, JSON.stringify(history));
  removeLocalStorageValue(LEGACY_SEARCH_HISTORY_KEY);
}

/**
 * Remove all known search history storage keys.
 */
function removeSearchHistory(): void {
  removeLocalStorageValue(SEARCH_HISTORY_KEY);
  removeLocalStorageValue(LEGACY_SEARCH_HISTORY_KEY);
}

/**
 * Save a search query to local history.
 *
 * @param query - Query text.
 */
function saveSearchHistory(query: string): void {
  if (typeof window === 'undefined' || !query.trim()) return;

  const history = getSearchHistory();
  const trimmedQuery = query.trim();
  const filtered = history.filter((item) => item !== trimmedQuery);
  const newHistory = [trimmedQuery, ...filtered].slice(0, MAX_HISTORY_ITEMS);

  writeSearchHistory(newHistory);
}

/**
 * Clear stored search history.
 */
function clearSearchHistory(): void {
  if (typeof window === 'undefined') return;
  removeSearchHistory();
}

type SearchBarProps = {
  xstyle?: stylex.StyleXStyles;
  placeholder?: string;
  /** Workspace search parameter that holds the submitted query. */
  queryParam?: 'q' | 'weekly_q';
  /** Offer advanced-syntax rewrites and templates next to the search history. */
  hasSyntaxAssist?: boolean;
};

type SearchAssistOption =
  | { kind: 'history'; id: string; index: number; query: string }
  | { kind: 'rewrite'; id: string; suggestion: SearchRewriteSuggestion }
  | { kind: 'builder'; id: string };

const MAX_ASSIST_HISTORY_ITEMS = 5;
const ASSIST_OPTION_ID_PREFIX = 'search-assist-option';
const REWRITE_LISTBOX_ID = 'search-rewrite-listbox';
const BUILDER_LISTBOX_ID = 'search-builder-listbox';

/**
 * Build the ordered option list shown in the search panel.
 *
 * @param draft - Current input text.
 * @param history - Loaded search history.
 * @param hasSyntaxAssist - Whether syntax suggestions are enabled.
 * @returns Options grouped as rewrites, history, then the builder entry.
 */
function buildAssistOptions(
  draft: string,
  history: readonly string[],
  hasSyntaxAssist: boolean,
): SearchAssistOption[] {
  const text = draft.trim();
  if (!hasSyntaxAssist) {
    return history.map((query, index) => ({
      kind: 'history',
      id: `${SEARCH_HISTORY_LISTBOX_ID}-option-${index}`,
      index,
      query,
    }));
  }
  const rewrites: SearchAssistOption[] = buildRewriteSuggestions(text)
    .slice(0, 6)
    .map((suggestion) => ({
      kind: 'rewrite',
      id: `${ASSIST_OPTION_ID_PREFIX}-rewrite-${suggestion.id}`,
      suggestion,
    }));
  const needle = text.toLocaleLowerCase();
  const matchingHistory = text
    ? history.filter((query) => query !== text && query.toLocaleLowerCase().includes(needle))
    : history;
  const historyOptions: SearchAssistOption[] = matchingHistory
    .slice(0, text ? MAX_ASSIST_HISTORY_ITEMS : MAX_HISTORY_ITEMS)
    .map((query, index) => ({
      kind: 'history',
      id: `${SEARCH_HISTORY_LISTBOX_ID}-option-${index}`,
      index,
      query,
    }));
  const builder: SearchAssistOption = { kind: 'builder', id: `${ASSIST_OPTION_ID_PREFIX}-builder` };
  return [...rewrites, ...historyOptions, builder];
}

/**
 * Render a submitted search query with a separate draft, search history, and syntax assistant.
 *
 * @param props - Optional styles, URL query parameter override, and syntax assistant flag.
 * @returns Search form and syntax-help popover.
 */
export function SearchBar({
  xstyle,
  placeholder = '搜索文章…',
  queryParam = 'q',
  hasSyntaxAssist = false,
}: SearchBarProps) {
  const q = useSearch({ from: WORKSPACE_ROUTE_ID, select: (search) => search[queryParam] ?? '' });
  const updateSearch = useUpdateWorkspaceSearch();
  const [inputValue, setInputValue] = useState(q);
  const [previousQuery, setPreviousQuery] = useState(q);
  const [searchHistory, setSearchHistory] = useState<string[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const [activeOptionIndex, setActiveOptionIndex] = useState(-1);
  const [isBuilderOpen, setIsBuilderOpen] = useState(false);
  const [storedBuilder, setStoredBuilder] = useState<StoredSearchBuilder | null>(() =>
    hasSyntaxAssist ? readStoredSearchBuilder() : null,
  );
  const inputRef = useRef<HTMLInputElement>(null);

  if (q !== previousQuery) {
    setPreviousQuery(q);
    setInputValue(q);
    if (hasSyntaxAssist) {
      setStoredBuilder(readStoredSearchBuilder());
    }
  }
  const appliedConditions =
    hasSyntaxAssist && storedBuilder && q && storedBuilder.query === q
      ? storedBuilder.conditions
      : [];

  const options = buildAssistOptions(inputValue, searchHistory, hasSyntaxAssist);
  const historyOptions = options.filter(
    (option): option is Extract<SearchAssistOption, { kind: 'history' }> =>
      option.kind === 'history',
  );
  const rewriteOptions = options.filter(
    (option): option is Extract<SearchAssistOption, { kind: 'rewrite' }> =>
      option.kind === 'rewrite',
  );
  const builderOption = options.find((option) => option.kind === 'builder');
  const activeOption = activeOptionIndex >= 0 ? options[activeOptionIndex] : undefined;
  const isPanelOpen = showHistory && options.length > 0;
  const isAdvancedDraft = hasSyntaxAssist && isAdvancedSearchQuery(inputValue);
  const controlledListboxes = [
    rewriteOptions.length > 0 ? REWRITE_LISTBOX_ID : null,
    historyOptions.length > 0 ? SEARCH_HISTORY_LISTBOX_ID : null,
    builderOption ? BUILDER_LISTBOX_ID : null,
  ].filter(Boolean);

  /**
   * Close the search panel and clear its active descendant.
   */
  const closeHistory = () => {
    setShowHistory(false);
    setActiveOptionIndex(-1);
  };

  /**
   * Move the caret after React commits a programmatic draft change.
   *
   * @param caret - Caret index in the next draft.
   */
  const placeCaret = (caret: number) => {
    requestAnimationFrame(() => {
      const input = inputRef.current;
      if (!input) return;
      input.focus();
      input.setSelectionRange(caret, caret);
    });
  };

  /**
   * Commit a supplied query or the current draft, including an empty value.
   *
   * @param query - Optional explicit history value.
   */
  const handleSearch = (query?: string) => {
    const searchQuery = (query ?? inputValue).trim();
    setInputValue(searchQuery);
    void updateSearch((search) => ({ ...search, [queryParam]: searchQuery || undefined }));
    if (searchQuery) {
      saveSearchHistory(searchQuery);
      setSearchHistory(getSearchHistory());
    }
    closeHistory();
  };

  /**
   * Apply one panel option: search history, rewrite the draft, or open the builder.
   *
   * @param option - Selected option.
   */
  const applyOption = (option: SearchAssistOption) => {
    if (option.kind === 'history') {
      handleSearch(option.query);
      return;
    }
    if (option.kind === 'rewrite') {
      setInputValue(option.suggestion.value);
      setActiveOptionIndex(-1);
      placeCaret(option.suggestion.value.length);
      return;
    }
    closeHistory();
    setIsBuilderOpen(true);
  };

  /**
   * Run a builder query and remember the rows that produced it.
   *
   * @param query - Compiled expression.
   * @param conditions - Rows that produced the expression.
   */
  const handleBuilderSearch = (query: string, conditions: SearchBuilderCondition[]) => {
    const snapshot = { query, conditions };
    writeStoredSearchBuilder(snapshot);
    setStoredBuilder(snapshot);
    setIsBuilderOpen(false);
    handleSearch(query);
  };

  /**
   * Remove one applied builder condition and search again.
   *
   * @param id - Condition id.
   */
  const removeAppliedCondition = (id: string) => {
    const conditions = appliedConditions.filter((condition) => condition.id !== id);
    const result = compileSearchConditions(conditions);
    const snapshot = { query: result.error ? '' : result.query, conditions };
    writeStoredSearchBuilder(snapshot);
    setStoredBuilder(snapshot);
    handleSearch(snapshot.query);
  };

  /**
   * Submit the current search draft.
   *
   * @param event - Search form submission event.
   */
  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    handleSearch();
  };

  /**
   * Navigate, apply, or dismiss panel options from the input.
   *
   * @param event - Search input keyboard event.
   */
  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const isNavigation = event.key === 'ArrowDown' || event.key === 'ArrowUp';
    let availableOptions = options;
    if (isNavigation && searchHistory.length === 0) {
      const loadedHistory = getSearchHistory();
      if (loadedHistory.length > 0) {
        setSearchHistory(loadedHistory);
        availableOptions = buildAssistOptions(inputValue, loadedHistory, hasSyntaxAssist);
      }
    }
    if (event.key === 'ArrowDown' && availableOptions.length > 0) {
      event.preventDefault();
      setShowHistory(true);
      setActiveOptionIndex((current) =>
        current < 0 ? 0 : Math.min(current + 1, availableOptions.length - 1),
      );
      return;
    }
    if (event.key === 'ArrowUp' && availableOptions.length > 0) {
      event.preventDefault();
      setShowHistory(true);
      setActiveOptionIndex((current) =>
        current < 0 ? availableOptions.length - 1 : Math.max(current - 1, 0),
      );
      return;
    }
    if (event.key === 'Enter' && isPanelOpen && activeOption) {
      event.preventDefault();
      applyOption(activeOption);
      return;
    }
    if (event.key === 'Escape' && showHistory) {
      event.preventDefault();
      closeHistory();
      inputRef.current?.focus();
    }
  };

  /**
   * Clear persisted search history and retain focus in the search input.
   */
  const handleClearHistory = () => {
    clearSearchHistory();
    setSearchHistory([]);
    if (!hasSyntaxAssist) {
      closeHistory();
    } else {
      setActiveOptionIndex(-1);
    }
    inputRef.current?.focus();
  };

  /**
   * Render one option row with shared active and pointer behavior.
   *
   * @param option - Option to render.
   * @param content - Row content.
   * @param optionXstyle - Additional row styles.
   * @returns Option button.
   */
  const renderOptionButton = (
    option: SearchAssistOption,
    content: ReactNode,
    optionXstyle?: stylex.StyleXStyles,
  ) => {
    const optionIndex = options.indexOf(option);
    const isActive = activeOptionIndex === optionIndex;
    return (
      <button
        key={option.id}
        id={option.id}
        type="button"
        role="option"
        aria-selected={isActive}
        {...stylex.props(
          styles.renderOptionButtonButton,
          stylex.defaultMarker(),
          isActive && styles.renderOptionButtonButtonActive,
          optionXstyle,
        )}
        onMouseDown={(event) => {
          if (option.kind === 'rewrite') event.preventDefault();
        }}
        onMouseMove={() => setActiveOptionIndex(optionIndex)}
        onClick={() => applyOption(option)}
      >
        {content}
      </button>
    );
  };

  return (
    <div {...stylex.props(styles.searchBarDiv, xstyle)}>
      <form role="search" {...stylex.props(styles.searchBarForm)} onSubmit={handleSubmit}>
        <div {...stylex.props(styles.searchBarDiv2, searchMarker)}>
          <Search {...stylex.props(styles.searchBarSearch)} aria-hidden="true" />
          <Popover
            open={isPanelOpen}
            onOpenChange={(isOpen) => {
              setShowHistory(isOpen);
              if (!isOpen) {
                setActiveOptionIndex(-1);
              }
            }}
          >
            <PopoverAnchor asChild>
              <Input
                ref={inputRef}
                type="search"
                role="combobox"
                aria-label="搜索文章"
                aria-autocomplete="list"
                aria-controls={
                  controlledListboxes.length > 0 ? controlledListboxes.join(' ') : undefined
                }
                aria-expanded={isPanelOpen}
                aria-activedescendant={activeOption?.id}
                name="article_search"
                autoComplete="off"
                spellCheck={false}
                placeholder={placeholder}
                {...{ [WORKSPACE_SEARCH_ATTRIBUTE]: '' }}
                xstyle={[
                  styles.searchBarInput,
                  isAdvancedDraft && styles.searchBarInputAdvancedDraft,
                ]}
                value={inputValue}
                onChange={(event) => {
                  setInputValue(event.target.value);
                  setActiveOptionIndex(-1);
                  if (hasSyntaxAssist) {
                    setShowHistory(true);
                  }
                }}
                onKeyDown={handleKeyDown}
                onClick={() => {
                  const availableHistory = getSearchHistory();
                  setSearchHistory(availableHistory);
                  if ((availableHistory.length > 0 || hasSyntaxAssist) && !showHistory) {
                    setShowHistory(true);
                  }
                }}
              />
            </PopoverAnchor>
            {options.length > 0 && (
              <PopoverContent
                xstyle={styles.searchBarPopoverContent}
                align="start"
                onOpenAutoFocus={(event: Event) => event.preventDefault()}
              >
                <div {...stylex.props(styles.searchBarDiv3)}>
                  {rewriteOptions.length > 0 && (
                    <section {...stylex.props(styles.searchBarSection)}>
                      <p {...stylex.props(styles.searchBarP)}>改写当前输入</p>
                      <div id={REWRITE_LISTBOX_ID} role="listbox" aria-label="改写当前输入">
                        {rewriteOptions.map((option) =>
                          renderOptionButton(
                            option,
                            <>
                              <Wand2 {...stylex.props(styles.searchBarWand2)} aria-hidden="true" />
                              <span {...stylex.props(styles.searchBarSpan)}>
                                {option.suggestion.value}
                              </span>
                              <span {...stylex.props(styles.searchBarSpan2)}>
                                {option.suggestion.label}
                              </span>
                            </>,
                          ),
                        )}
                      </div>
                    </section>
                  )}
                  {historyOptions.length > 0 && (
                    <section
                      {...stylex.props(
                        rewriteOptions.length > 0 && styles.searchBarSectionLength,
                        styles.searchBarSection,
                      )}
                    >
                      <div {...stylex.props(styles.searchBarDiv4)}>
                        <span {...stylex.props(styles.searchBarSpan3)}>
                          <Clock {...stylex.props(styles.searchBarClock)} aria-hidden="true" />
                          最近搜索
                        </span>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          static
                          xstyle={styles.searchBarButton}
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={handleClearHistory}
                        >
                          清空
                        </Button>
                      </div>
                      <div id={SEARCH_HISTORY_LISTBOX_ID} role="listbox" aria-label="最近搜索">
                        <MotionPresence>
                          {historyOptions.map((option) => (
                            <MotionDiv
                              key={option.query}
                              data-motion-history-key={option.query}
                              variants={SWAP_VARIANTS}
                              initial="hidden"
                              animate="visible"
                              exit="exit"
                            >
                              {renderOptionButton(
                                option,
                                <>
                                  <span {...stylex.props(styles.searchBarSpan4)}>
                                    {option.query}
                                  </span>
                                  <Search
                                    aria-hidden="true"
                                    {...stylex.props(
                                      styles.searchBarSearch2,
                                      activeOption === option
                                        ? styles.searchBarSearchOption
                                        : styles.searchBarSearchNotOption,
                                    )}
                                  />
                                </>,
                                styles.optionButtonSpread,
                              )}
                            </MotionDiv>
                          ))}
                        </MotionPresence>
                      </div>
                    </section>
                  )}
                  {builderOption && (
                    <section {...stylex.props(options.length > 1 && styles.searchBarSectionLength)}>
                      <div id={BUILDER_LISTBOX_ID} role="listbox" aria-label="高级检索构建器">
                        {renderOptionButton(
                          builderOption,
                          <>
                            <SlidersHorizontal
                              {...stylex.props(styles.searchBarSlidersHorizontal)}
                              aria-hidden="true"
                            />
                            <span {...stylex.props(styles.searchBarSpan5)}>使用高级检索构建器</span>
                            <span {...stylex.props(styles.searchBarSpan6)}>
                              按字段组合条件，无需手写语法
                            </span>
                          </>,
                        )}
                      </div>
                    </section>
                  )}
                </div>
                {hasSyntaxAssist && (
                  <div {...stylex.props(styles.searchBarDiv5)}>
                    <span {...stylex.props(styles.searchBarSpan7)}>
                      <i
                        aria-hidden="true"
                        {...stylex.props(
                          styles.searchBarI,
                          isAdvancedDraft
                            ? styles.searchBarIAdvancedDraft
                            : styles.searchBarINotAdvancedDraft,
                        )}
                      />
                      {isAdvancedDraft ? '将按高级语法检索' : '普通关键词检索'}
                    </span>
                    <span {...stylex.props(styles.searchBarSpan8)}>
                      ↑↓ 选择 · Enter 确认 · Esc 关闭
                    </span>
                  </div>
                )}
              </PopoverContent>
            )}
          </Popover>
          {!inputValue && (
            <kbd aria-hidden="true" {...stylex.props(styles.searchBarKbd)}>
              /
            </kbd>
          )}
          {inputValue && (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              static
              aria-label="清空搜索输入"
              onClick={() => {
                setInputValue('');
                setActiveOptionIndex(-1);
                inputRef.current?.focus();
              }}
              xstyle={styles.searchBarButton2}
            >
              <X {...stylex.props(styles.searchBarX)} aria-hidden="true" />
            </Button>
          )}
        </div>

        <Button type="submit" xstyle={styles.searchBarButton3}>
          搜索
        </Button>
        {hasSyntaxAssist && (
          <Button
            type="button"
            variant="ghost"
            xstyle={styles.searchBarButton4}
            aria-label="高级检索"
            title="高级检索"
            onClick={() => {
              closeHistory();
              setIsBuilderOpen(true);
            }}
          >
            <SlidersHorizontal {...stylex.props(styles.searchBarX)} aria-hidden="true" />
            <span {...stylex.props(styles.searchBarSpan9)}>高级</span>
          </Button>
        )}
      </form>
      {appliedConditions.length > 0 && (
        <ul aria-label="已应用的检索条件" {...stylex.props(styles.searchBarUl)}>
          {appliedConditions.map((condition, index) => {
            const remaining = appliedConditions.filter((item) => item.id !== condition.id);
            const canRemove = remaining.length === 0 || !compileSearchConditions(remaining).error;
            return (
              <li key={condition.id} {...stylex.props(styles.searchBarSpan7)}>
                {index > 0 && (
                  <span
                    {...stylex.props(
                      styles.searchBarSpan10,
                      condition.connector === 'NOT'
                        ? styles.searchBarSpanNOT
                        : styles.searchBarInputAdvancedDraft,
                    )}
                  >
                    {describeSearchConnector(condition.connector)}
                  </span>
                )}
                <span {...stylex.props(styles.searchBarSpan11)}>
                  <button
                    type="button"
                    {...stylex.props(styles.searchBarButton5)}
                    title="在高级检索中编辑"
                    onClick={() => setIsBuilderOpen(true)}
                  >
                    {describeSearchCondition(condition)}
                  </button>
                  <button
                    type="button"
                    {...stylex.props(styles.searchBarButton6)}
                    aria-label={`移除条件：${describeSearchCondition(condition)}`}
                    title={canRemove ? '移除条件' : '“不含”条件前至少需要一个包含条件'}
                    disabled={!canRemove}
                    onClick={() => removeAppliedCondition(condition.id)}
                  >
                    <X {...stylex.props(styles.searchBarX2)} aria-hidden="true" />
                  </button>
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {hasSyntaxAssist && (
        <SearchBuilderDialog
          currentQuery={q}
          open={isBuilderOpen}
          onOpenChange={setIsBuilderOpen}
          onSearch={handleBuilderSearch}
        />
      )}
    </div>
  );
}

const styles = stylex.create({
  optionButtonSpread: {
    justifyContent: 'space-between',
  },
  renderOptionButtonButton: {
    display: 'flex',
    minHeight: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2.25rem',
    },
    width: '100%',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.75rem',
    borderRadius: '6px',
    paddingInline: '0.625rem',
    paddingBlock: '0.375rem',
    textAlign: 'left',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    transitionProperty: 'background-color, color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
  },
  renderOptionButtonButtonActive: {
    backgroundColor: {
      default: colors.accent,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
  },
  searchBarDiv: {
    width: '100%',
    minWidth: '0px',
  },
  searchBarForm: {
    display: 'flex',
    width: '100%',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.5rem',
  },
  searchBarDiv2: {
    position: 'relative',
    minWidth: '0px',
    flex: '1',
    borderRadius: '12px',
    backgroundColor: {
      default: colors.muted,
      ':focus-within': colors.background,
    },
    boxShadow: {
      default: `inset 0 0 0 1px ${colors.border}`,
      ':focus-within': {
        default: `inset 0 0 0 1px ${colors.ring}, 0 0 0 3px color-mix(in oklab,${colors.ring} 18%,transparent)`,
        ':hover': {
          default: null,
          '@media (hover: hover)': `inset 0 0 0 1px ${colors.inputHover}`,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `inset 0 0 0 1px ${colors.inputHover}`,
        ':focus-within': {
          default: null,
          '@media (hover: hover)': `inset 0 0 0 1px ${colors.inputHover}`,
        },
      },
    },
    transitionProperty: 'background-color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
  },
  searchBarSearch: {
    pointerEvents: 'none',
    position: 'absolute',
    top: 'calc(1 / 2 * 100%)',
    left: '0.875rem',
    width: '1rem',
    height: '1rem',
    translate: '0 calc(calc(1 / 2 * 100%) * -1)',
    color: colors.mutedForeground,
  },
  searchBarInput: {
    height: '2.75rem',
    borderRadius: '12px',
    backgroundColor: {
      default: 'transparent',
      ':hover': {
        default: null,
        '@media (hover: hover)': 'transparent',
      },
    },
    paddingRight: '2.75rem',
    paddingLeft: '2.5rem',
    boxShadow: {
      default: 'none',
      ':focus-visible': {
        default: '0 0 0 0px currentcolor',
        ':hover': {
          default: null,
          '@media (hover: hover)': '0 0 0 0px currentcolor',
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': 'none',
        ':focus-visible': {
          default: null,
          '@media (hover: hover)': '0 0 0 0px currentcolor',
        },
      },
    },
    '::-webkit-search-cancel-button': {
      display: 'none',
      appearance: 'none',
    },
  },
  searchBarInputAdvancedDraft: {
    color: colors.primary,
  },
  searchBarPopoverContent: {
    width: 'var(--radix-popover-trigger-width)',
    minWidth: '18rem',
    padding: '0px',
  },
  searchBarDiv3: {
    maxHeight: 'min(28rem, 70dvh)',
    overflowY: 'auto',
    overscrollBehavior: 'contain',
    padding: '0.375rem',
  },
  searchBarSection: {
    paddingBottom: '0.25rem',
  },
  searchBarP: {
    paddingInline: '0.625rem',
    paddingTop: '0.375rem',
    paddingBottom: '0.25rem',
    fontSize: '11px',
    fontWeight: 700,
    letterSpacing: '0.08em',
    color: colors.mutedForeground,
  },
  searchBarWand2: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    color: colors.mutedForeground,
  },
  searchBarSpan: {
    minWidth: '0px',
    flex: '1',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    color: colors.foreground,
  },
  searchBarSpan2: {
    flexShrink: 0,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  searchBarSectionLength: {
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingTop: '0.25rem',
  },
  searchBarDiv4: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingInline: '0.625rem',
    paddingTop: '0.25rem',
  },
  searchBarSpan3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.375rem',
    fontSize: '11px',
    fontWeight: 700,
    letterSpacing: '0.08em',
    color: colors.mutedForeground,
  },
  searchBarClock: {
    width: '0.75rem',
    height: '0.75rem',
  },
  searchBarButton: {
    minHeight: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    paddingInline: {
      default: '0.5rem',
      ':has( > svg)': '0.625rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  searchBarSpan4: {
    minWidth: '0px',
    flex: '1',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  searchBarSearch2: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    color: colors.mutedForeground,
    transitionProperty: 'opacity',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
  },
  searchBarSearchOption: {
    opacity: '100%',
  },
  searchBarSearchNotOption: {
    opacity: {
      default: '0%',
      '@media (hover: hover)': {
        default: null,
        [stylex.when.ancestor(':hover')]: '100%',
      },
    },
  },
  searchBarSlidersHorizontal: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
    color: colors.primary,
  },
  searchBarSpan5: {
    minWidth: '0px',
    flex: '1',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontWeight: 700,
  },
  searchBarSpan6: {
    display: {
      default: 'none',
      '@media (min-width: 40rem)': 'inline',
    },
    flexShrink: 0,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  searchBarDiv5: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingInline: '1rem',
    paddingBlock: '0.5rem',
    fontSize: '11px',
    color: colors.mutedForeground,
  },
  searchBarSpan7: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.375rem',
  },
  searchBarI: {
    width: '0.375rem',
    height: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  searchBarIAdvancedDraft: {
    backgroundColor: colors.primary,
  },
  searchBarINotAdvancedDraft: {
    backgroundColor: `color-mix(in oklab, ${colors.mutedForeground} 40%, transparent)`,
  },
  searchBarSpan8: {
    display: {
      default: 'none',
      '@media (min-width: 40rem)': 'inline',
    },
  },
  searchBarKbd: {
    pointerEvents: 'none',
    position: 'absolute',
    top: 'calc(1 / 2 * 100%)',
    right: '0.75rem',
    display: {
      default: 'none',
      [stylex.when.ancestor(':focus-within', searchMarker)]: 'none',
      '@media (min-width: 64rem)': 'inline-flex',
    },
    height: '1.5rem',
    minWidth: '1.5rem',
    translate: '0 calc(calc(1 / 2 * 100%) * -1)',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '6px',
    backgroundColor: colors.background,
    paddingInline: '0.375rem',
    fontSize: '11px',
    color: colors.mutedForeground,
    boxShadow: shadows.vercelRing,
  },
  searchBarButton2: {
    position: 'absolute',
    top: '0px',
    right: '0px',
    width: '2.75rem',
    height: '2.75rem',
    borderRadius: '12px',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': 'transparent',
      },
    },
  },
  searchBarX: {
    width: '1rem',
    height: '1rem',
  },
  searchBarButton3: {
    height: '2.75rem',
    minWidth: '2.75rem',
    flexShrink: 0,
    borderRadius: '12px',
    backgroundColor: {
      default: colors.foreground,
      ':active': {
        default: `color-mix(in oklab, ${colors.foreground} 75%, transparent)`,
        ':hover': {
          default: null,
          '@media (hover: hover)': `color-mix(in oklab, ${colors.foreground} 75%, transparent)`,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.foreground} 85%, transparent)`,
      },
    },
    paddingInline: {
      default: '1rem',
      ':has( > svg)': '0.75rem',
    },
    color: colors.background,
  },
  searchBarButton4: {
    height: '2.75rem',
    minWidth: '2.75rem',
    flexShrink: 0,
    gap: '0.375rem',
    borderRadius: '12px',
    paddingInline: {
      default: '0px',
      ':has( > svg)': '0.75rem',
      '@media (min-width: 64rem)': '0.75rem',
    },
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
  },
  searchBarSpan9: {
    display: {
      default: 'none',
      '@media (min-width: 64rem)': 'inline',
    },
    fontSize: '13px',
  },
  searchBarUl: {
    marginTop: '0.625rem',
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.375rem',
  },
  searchBarSpan10: {
    fontSize: '11px',
    fontWeight: 700,
  },
  searchBarSpanNOT: {
    color: colors.destructive,
  },
  searchBarSpan11: {
    display: 'inline-flex',
    height: '1.75rem',
    maxWidth: '16rem',
    alignItems: 'center',
    borderRadius: '6px',
    backgroundColor: colors.muted,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  searchBarButton5: {
    minWidth: '0px',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    borderTopLeftRadius: '6px',
    borderBottomLeftRadius: '6px',
    paddingRight: '0.25rem',
    paddingLeft: '0.625rem',
    outlineStyle: 'none',
    color: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  searchBarButton6: {
    display: 'flex',
    width: '1.75rem',
    height: '1.75rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderTopRightRadius: '6px',
    borderBottomRightRadius: '6px',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentPressed,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
    pointerEvents: {
      default: null,
      ':disabled': 'none',
    },
    opacity: {
      default: null,
      ':disabled': '40%',
    },
  },
  searchBarX2: {
    width: '0.875rem',
    height: '0.875rem',
  },
});
