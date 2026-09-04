'use client';

/**
 * URL-backed search form with keyboard history and restrained local motion.
 */

import { useQueryState, parseAsString } from 'nuqs';
import { Input } from '@/components/ui/input';
import { Search, X, Clock, HelpCircle } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Popover, PopoverAnchor, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
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
  queryParam?: string;
};

/**
 * Render a submitted search query with a separate draft and keyboard history picker.
 *
 * @param props - Optional class name and URL query parameter override.
 * @returns Search form and syntax-help popover.
 */
export function SearchBar({ className, queryParam = 'q' }: SearchBarProps) {
  const [q, setQ] = useQueryState(queryParam, parseAsString.withDefault(''));
  const [inputValue, setInputValue] = useState(q);
  const [previousQuery, setPreviousQuery] = useState(q);
  const [searchHistory, setSearchHistory] = useState<string[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const [activeHistoryIndex, setActiveHistoryIndex] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const transition = useMotionTransition(MOTION_DURATION_SECONDS.fast);

  if (q !== previousQuery) {
    setPreviousQuery(q);
    setInputValue(q);
  }

  /**
   * Close the history popup and clear its active descendant.
   */
  const closeHistory = () => {
    setShowHistory(false);
    setActiveHistoryIndex(-1);
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
   * Submit the current search draft.
   *
   * @param event - Search form submission event.
   */
  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    handleSearch();
  };

  /**
   * Navigate, apply, or dismiss search history from the input.
   *
   * @param event - Search input keyboard event.
   */
  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const availableHistory = searchHistory.length > 0 ? searchHistory : getSearchHistory();
    if (event.key === 'ArrowDown' && availableHistory.length > 0) {
      event.preventDefault();
      setSearchHistory(availableHistory);
      setShowHistory(true);
      setActiveHistoryIndex((current) =>
        current < 0 ? 0 : Math.min(current + 1, availableHistory.length - 1),
      );
      return;
    }
    if (event.key === 'ArrowUp' && availableHistory.length > 0) {
      event.preventDefault();
      setSearchHistory(availableHistory);
      setShowHistory(true);
      setActiveHistoryIndex((current) =>
        current < 0 ? availableHistory.length - 1 : Math.max(current - 1, 0),
      );
      return;
    }
    if (event.key === 'Enter' && showHistory && activeHistoryIndex >= 0) {
      event.preventDefault();
      handleSearch(searchHistory[activeHistoryIndex]);
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
    closeHistory();
    inputRef.current?.focus();
  };

  /**
   * Apply one clicked history entry.
   *
   * @param query - Selected history entry.
   */
  const handleHistoryItemClick = (query: string) => {
    handleSearch(query);
  };

  return (
    <form
      role="search"
      className={cn('flex w-full min-w-0 items-center gap-1.5 sm:gap-2', className)}
      onSubmit={handleSubmit}
    >
      <div className="relative min-w-0 flex-1">
        <Search
          className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden="true"
        />
        <Popover
          open={showHistory}
          onOpenChange={(isOpen) => {
            setShowHistory(isOpen);
            if (!isOpen) {
              setActiveHistoryIndex(-1);
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
              aria-controls={searchHistory.length > 0 ? SEARCH_HISTORY_LISTBOX_ID : undefined}
              aria-expanded={showHistory}
              aria-activedescendant={
                activeHistoryIndex >= 0
                  ? `${SEARCH_HISTORY_LISTBOX_ID}-option-${activeHistoryIndex}`
                  : undefined
              }
              name="article_search"
              autoComplete="off"
              spellCheck={false}
              placeholder="搜索文章…"
              className="search-input h-11 pl-10 pr-11 md:h-10 md:pr-10"
              value={inputValue}
              onChange={(event) => {
                setInputValue(event.target.value);
                setActiveHistoryIndex(-1);
              }}
              onKeyDown={handleKeyDown}
              onClick={() => {
                const availableHistory = getSearchHistory();
                setSearchHistory(availableHistory);
                if (availableHistory.length > 0 && !showHistory) {
                  setShowHistory(true);
                }
              }}
            />
          </PopoverAnchor>
          {searchHistory.length > 0 && (
            <PopoverContent
              className="w-[var(--radix-popover-trigger-width)] rounded-xl border-0 p-0 shadow-vercel-card"
              align="start"
              onOpenAutoFocus={(event: Event) => event.preventDefault()}
            >
              <div className="p-2">
                <div className="mb-1 flex items-center justify-between px-2 py-1">
                  <span className="flex items-center gap-1 text-xs font-medium text-muted-foreground">
                    <Clock className="h-3 w-3" aria-hidden="true" />
                    最近搜索
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    static
                    className="min-h-11 text-xs md:min-h-10"
                    onClick={handleClearHistory}
                  >
                    清空
                  </Button>
                </div>
                <div
                  id={SEARCH_HISTORY_LISTBOX_ID}
                  role="listbox"
                  aria-label="最近搜索"
                  className="space-y-1"
                >
                  <MotionPresence>
                    {searchHistory.map((query, index) => (
                      <MotionDiv
                        key={query}
                        data-motion-history-key={query}
                        variants={FADE_VARIANTS}
                        initial="hidden"
                        animate="visible"
                        exit={{ opacity: 0, pointerEvents: 'none' }}
                        transition={transition}
                      >
                        <button
                          id={`${SEARCH_HISTORY_LISTBOX_ID}-option-${index}`}
                          type="button"
                          role="option"
                          aria-selected={activeHistoryIndex === index}
                          className={cn(
                            'motion-control group flex min-h-11 w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left text-sm outline-none transition-[background-color,color] hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 md:min-h-10',
                            activeHistoryIndex === index && 'bg-accent',
                          )}
                          onMouseMove={() => setActiveHistoryIndex(index)}
                          onClick={() => handleHistoryItemClick(query)}
                        >
                          <span className="truncate">{query}</span>
                          <Search
                            aria-hidden="true"
                            className={cn(
                              'motion-control size-4 shrink-0 text-muted-foreground transition-opacity',
                              activeHistoryIndex === index
                                ? 'opacity-100'
                                : 'opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100',
                            )}
                          />
                        </button>
                      </MotionDiv>
                    ))}
                  </MotionPresence>
                </div>
              </div>
            </PopoverContent>
          )}
        </Popover>
        {inputValue && (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            static
            aria-label="清空搜索输入"
            onClick={() => {
              setInputValue('');
              setActiveHistoryIndex(-1);
              inputRef.current?.focus();
            }}
            className="absolute right-0 top-0 size-11 rounded-l-none text-muted-foreground hover:text-foreground md:size-10"
          >
            <X className="size-4" aria-hidden="true" />
          </Button>
        )}
      </div>

      <Button type="submit" className="h-11 min-w-11 shrink-0 px-3 sm:px-4 md:h-10">
        搜索
      </Button>
      <Popover>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            size="icon"
            className="size-11 md:size-10"
            aria-label="搜索语法帮助"
            title="搜索语法帮助"
          >
            <HelpCircle className="h-4 w-4" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-96 max-w-[calc(100vw-2rem)]" align="end" sideOffset={8}>
          <div className="space-y-4 text-xs">
            <div className="text-sm font-semibold text-foreground">FTS5 搜索语法</div>
            <div className="space-y-2">
              <div className="text-foreground/80 font-medium">基础</div>
              <ul className="space-y-1 text-muted-foreground">
                <li>
                  <code>term1 AND term2</code> 同时包含两个词
                </li>
                <li>
                  <code>term1 OR term2</code> 任意一个词
                </li>
                <li>
                  <code>term1 NOT term2</code> 排除 term2
                </li>
                <li>
                  <code>&quot;exact phrase&quot;</code> 精确短语
                </li>
                <li>
                  <code>bio*</code> 前缀匹配
                </li>
              </ul>
            </div>
            <div className="space-y-2">
              <div className="text-foreground/80 font-medium">高级</div>
              <ul className="space-y-1 text-muted-foreground">
                <li>
                  <code>NEAR(&quot;gene expression&quot; therapy, 5)</code> 距离 5 词以内
                </li>
                <li>
                  <code>title:diabetes</code> 指定字段
                </li>
                <li>
                  <code>{'{title abstract}:imaging'}</code> 多字段
                </li>
                <li>
                  <code>authors:&quot;Smith&quot;</code> 作者
                </li>
                <li>
                  <code>journal_title:&quot;Nature&quot;</code> 期刊
                </li>
                <li>
                  <code>^introduction</code> 列开头匹配
                </li>
              </ul>
            </div>
            <div className="text-muted-foreground">运算符 AND/OR/NOT/NEAR 需要大写。</div>
          </div>
        </PopoverContent>
      </Popover>
    </form>
  );
}
