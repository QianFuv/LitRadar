'use client';

/**
 * URL-backed search form with keyboard history, an advanced-syntax assistant, and restrained motion.
 */

import { useQueryState, parseAsString } from 'nuqs';
import { Input } from '@/components/ui/input';
import { Search, X, Clock, SlidersHorizontal, Wand2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover';
import { SearchBuilderDialog } from '@/components/feature/search-builder-dialog';
import { useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from 'react';
import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
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
import { cn } from '@/lib/utils';
import {
  FADE_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';

const SEARCH_HISTORY_KEY = 'litradar:v1:search_history';
const LEGACY_SEARCH_HISTORY_KEY = 'search_history';
const MAX_HISTORY_ITEMS = 10;
const SEARCH_HISTORY_LISTBOX_ID = 'search-history-listbox';

/**
 * Parse and validate serialized search history.
 *
 * @param value - Serialized history value.
 * @returns Search history or null when invalid.
 */
function parseSearchHistory(value: string): string[] | null {
  try {
    const parsedHistory: unknown = JSON.parse(value);
    if (Array.isArray(parsedHistory) && parsedHistory.every((item) => typeof item === 'string')) {
      return parsedHistory;
    }
  } catch {}
  return null;
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
  className?: string;
  placeholder?: string;
  queryParam?: string;
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
 * @param props - Optional class name, URL query parameter override, and syntax assistant flag.
 * @returns Search form and syntax-help popover.
 */
export function SearchBar({
  className,
  placeholder = '搜索文章…',
  queryParam = 'q',
  hasSyntaxAssist = false,
}: SearchBarProps) {
  const [q, setQ] = useQueryState(queryParam, parseAsString.withDefault(''));
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
  const transition = useMotionTransition(MOTION_DURATION_SECONDS.fast);

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
    void setQ(searchQuery || null);
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
   * @param className - Additional row classes.
   * @returns Option button.
   */
  const renderOptionButton = (
    option: SearchAssistOption,
    content: ReactNode,
    className?: string,
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
        className={cn(
          'motion-control group flex min-h-11 w-full min-w-0 items-center gap-3 rounded-md px-2.5 py-1.5 text-left text-sm outline-none transition-[background-color,color] hover:bg-accent md:min-h-9',
          isActive && 'bg-accent',
          className,
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
    <div className={cn('w-full min-w-0', className)}>
      <form
        role="search"
        className="flex w-full min-w-0 items-center gap-2"
        onSubmit={handleSubmit}
      >
        <div className="motion-control group/search relative min-w-0 flex-1 rounded-xl bg-muted shadow-[inset_0_0_0_1px_var(--border)] transition-[background-color,box-shadow] focus-within:bg-background focus-within:shadow-[inset_0_0_0_1px_var(--ring),0_0_0_3px_color-mix(in_oklab,var(--ring)_18%,transparent)] hover:shadow-[inset_0_0_0_1px_var(--input-hover)]">
          <Search
            className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
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
                className={cn(
                  'search-input h-11 rounded-xl bg-transparent pr-11 pl-10 !shadow-none hover:bg-transparent hover:!shadow-none focus-visible:ring-0',
                  isAdvancedDraft && 'text-primary',
                )}
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
                className="w-[var(--radix-popover-trigger-width)] min-w-72 p-0"
                align="start"
                onOpenAutoFocus={(event: Event) => event.preventDefault()}
              >
                <div className="max-h-[min(28rem,70dvh)] overflow-y-auto overscroll-contain p-1.5">
                  {rewriteOptions.length > 0 && (
                    <section className="pb-1">
                      <p className="px-2.5 pt-1.5 pb-1 text-[11px] font-bold tracking-[0.08em] text-muted-foreground">
                        改写当前输入
                      </p>
                      <div id={REWRITE_LISTBOX_ID} role="listbox" aria-label="改写当前输入">
                        {rewriteOptions.map((option) =>
                          renderOptionButton(
                            option,
                            <>
                              <Wand2
                                className="size-3.5 shrink-0 text-muted-foreground"
                                aria-hidden="true"
                              />
                              <span className="min-w-0 flex-1 truncate text-foreground">
                                {option.suggestion.value}
                              </span>
                              <span className="shrink-0 text-xs text-muted-foreground">
                                {option.suggestion.label}
                              </span>
                            </>,
                          ),
                        )}
                      </div>
                    </section>
                  )}
                  {historyOptions.length > 0 && (
                    <section className={cn(rewriteOptions.length > 0 && 'border-t pt-1', 'pb-1')}>
                      <div className="flex items-center justify-between px-2.5 pt-1">
                        <span className="flex items-center gap-1.5 text-[11px] font-bold tracking-[0.08em] text-muted-foreground">
                          <Clock className="size-3" aria-hidden="true" />
                          最近搜索
                        </span>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          static
                          className="min-h-11 px-2 text-xs text-muted-foreground md:min-h-8"
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
                              variants={FADE_VARIANTS}
                              initial="hidden"
                              animate="visible"
                              exit={{ opacity: 0, pointerEvents: 'none' }}
                              transition={transition}
                            >
                              {renderOptionButton(
                                option,
                                <>
                                  <span className="min-w-0 flex-1 truncate">{option.query}</span>
                                  <Search
                                    aria-hidden="true"
                                    className={cn(
                                      'motion-control size-4 shrink-0 text-muted-foreground transition-opacity',
                                      activeOption === option
                                        ? 'opacity-100'
                                        : 'opacity-0 group-hover:opacity-100',
                                    )}
                                  />
                                </>,
                                'justify-between',
                              )}
                            </MotionDiv>
                          ))}
                        </MotionPresence>
                      </div>
                    </section>
                  )}
                  {builderOption && (
                    <section className={cn(options.length > 1 && 'border-t pt-1')}>
                      <div id={BUILDER_LISTBOX_ID} role="listbox" aria-label="高级检索构建器">
                        {renderOptionButton(
                          builderOption,
                          <>
                            <SlidersHorizontal
                              className="size-3.5 shrink-0 text-primary"
                              aria-hidden="true"
                            />
                            <span className="min-w-0 flex-1 truncate font-bold">
                              使用高级检索构建器
                            </span>
                            <span className="hidden shrink-0 text-xs text-muted-foreground sm:inline">
                              按字段组合条件，无需手写语法
                            </span>
                          </>,
                        )}
                      </div>
                    </section>
                  )}
                </div>
                {hasSyntaxAssist && (
                  <div className="flex items-center justify-between gap-3 border-t px-4 py-2 text-[11px] text-muted-foreground">
                    <span className="flex items-center gap-1.5">
                      <i
                        aria-hidden="true"
                        className={cn(
                          'size-1.5 rounded-full',
                          isAdvancedDraft ? 'bg-primary' : 'bg-muted-foreground/40',
                        )}
                      />
                      {isAdvancedDraft ? '将按高级语法检索' : '普通关键词检索'}
                    </span>
                    <span className="hidden sm:inline">↑↓ 选择 · Enter 确认 · Esc 关闭</span>
                  </div>
                )}
              </PopoverContent>
            )}
          </Popover>
          {!inputValue && (
            <kbd
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 right-3 hidden h-6 min-w-6 -translate-y-1/2 items-center justify-center rounded-md bg-background px-1.5 text-[11px] text-muted-foreground shadow-vercel-ring group-focus-within/search:hidden lg:inline-flex"
            >
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
              className="absolute top-0 right-0 size-11 rounded-xl text-muted-foreground hover:bg-transparent hover:text-foreground"
            >
              <X className="size-4" aria-hidden="true" />
            </Button>
          )}
        </div>

        <Button
          type="submit"
          className="h-11 min-w-11 shrink-0 rounded-xl bg-foreground px-4 text-background hover:bg-foreground/85 active:bg-foreground/75"
        >
          搜索
        </Button>
        {hasSyntaxAssist && (
          <Button
            type="button"
            variant="ghost"
            className="h-11 min-w-11 shrink-0 gap-1.5 rounded-xl px-0 text-muted-foreground hover:text-foreground lg:px-3"
            aria-label="高级检索"
            title="高级检索"
            onClick={() => {
              closeHistory();
              setIsBuilderOpen(true);
            }}
          >
            <SlidersHorizontal className="size-4" aria-hidden="true" />
            <span className="hidden text-[13px] lg:inline">高级</span>
          </Button>
        )}
      </form>
      {appliedConditions.length > 0 && (
        <ul aria-label="已应用的检索条件" className="mt-2.5 flex flex-wrap items-center gap-1.5">
          {appliedConditions.map((condition, index) => {
            const remaining = appliedConditions.filter((item) => item.id !== condition.id);
            const canRemove = remaining.length === 0 || !compileSearchConditions(remaining).error;
            return (
              <li key={condition.id} className="flex items-center gap-1.5">
                {index > 0 && (
                  <span
                    className={cn(
                      'text-[11px] font-bold',
                      condition.connector === 'NOT' ? 'text-destructive' : 'text-primary',
                    )}
                  >
                    {describeSearchConnector(condition.connector)}
                  </span>
                )}
                <span className="inline-flex h-7 max-w-64 items-center rounded-md bg-muted text-xs">
                  <button
                    type="button"
                    className="min-w-0 truncate rounded-l-md pr-1 pl-2.5 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                    title="在高级检索中编辑"
                    onClick={() => setIsBuilderOpen(true)}
                  >
                    {describeSearchCondition(condition)}
                  </button>
                  <button
                    type="button"
                    className="flex size-7 shrink-0 items-center justify-center rounded-r-md text-muted-foreground outline-none hover:bg-accent-pressed hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40"
                    aria-label={`移除条件：${describeSearchCondition(condition)}`}
                    title={canRemove ? '移除条件' : '“不含”条件前至少需要一个包含条件'}
                    disabled={!canRemove}
                    onClick={() => removeAppliedCondition(condition.id)}
                  >
                    <X className="size-3.5" aria-hidden="true" />
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
