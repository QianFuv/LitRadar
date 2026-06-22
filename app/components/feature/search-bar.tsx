'use client';

import { useQueryState, parseAsString } from 'nuqs';
import { Input } from '@/components/ui/input';
import { Search, X, Clock, HelpCircle } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useState, useEffect } from 'react';
import { cn } from '@/lib/utils';

const SEARCH_HISTORY_KEY = 'ps:v1:search_history';
const LEGACY_SEARCH_HISTORY_KEY = 'search_history';
const MAX_HISTORY_ITEMS = 10;

/**
 * Read a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 * @returns Stored value or null.
 */
function readLocalStorageValue(key: string): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Write a localStorage value without surfacing quota or privacy-mode errors.
 *
 * @param key - Storage key.
 * @param value - Value to store.
 */
function writeLocalStorageValue(key: string, value: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.setItem(key, value);
  } catch {}
}

/**
 * Remove a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 */
function removeLocalStorageValue(key: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.removeItem(key);
  } catch {}
}

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

export function SearchBar({ className }: { className?: string }) {
  const [q, setQ] = useQueryState('q', parseAsString.withDefault(''));
  const [inputValue, setInputValue] = useState(q);
  const [searchHistory, setSearchHistory] = useState<string[]>([]);
  const [showHistory, setShowHistory] = useState(false);

  useEffect(() => {
    setSearchHistory(getSearchHistory());
  }, []);

  useEffect(() => {
    setInputValue(q);
  }, [q]);

  const handleSearch = (query?: string) => {
    const searchQuery = query || inputValue;
    if (searchQuery.trim()) {
      setQ(searchQuery);
      saveSearchHistory(searchQuery);
      setSearchHistory(getSearchHistory());
      setShowHistory(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleSearch();
    }
  };

  const handleClearHistory = () => {
    clearSearchHistory();
    setSearchHistory([]);
  };

  const handleHistoryItemClick = (query: string) => {
    setInputValue(query);
    handleSearch(query);
  };

  return (
    <div className={cn('flex w-full min-w-0 items-center gap-2', className)}>
      <div className="relative min-w-0 flex-1">
        <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-slate-500" />
        <Popover open={showHistory} onOpenChange={setShowHistory}>
          <PopoverTrigger asChild>
            <Input
              type="search"
              aria-label="搜索文章"
              placeholder="搜索文章…"
              className="pl-9 pr-9"
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              onKeyDown={handleKeyDown}
              onClick={() => {
                if (searchHistory.length > 0 && !showHistory) {
                  setShowHistory(true);
                }
              }}
            />
          </PopoverTrigger>
          {searchHistory.length > 0 && (
            <PopoverContent
              className="w-[var(--radix-popover-trigger-width)] p-0"
              align="start"
              onOpenAutoFocus={(event: Event) => event.preventDefault()}
            >
              <div className="p-2">
                <div className="flex items-center justify-between px-2 py-1 mb-1">
                  <span className="text-xs font-medium text-muted-foreground flex items-center gap-1">
                    <Clock className="h-3 w-3" />
                    最近搜索
                  </span>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 text-xs"
                    onClick={handleClearHistory}
                  >
                    清空
                  </Button>
                </div>
                <div className="space-y-1">
                  {searchHistory.map((query, index) => (
                    <button
                      key={index}
                      className="w-full text-left px-2 py-1.5 text-sm rounded hover:bg-accent transition-colors flex items-center justify-between group"
                      onClick={() => handleHistoryItemClick(query)}
                    >
                      <span className="truncate">{query}</span>
                      <Search className="h-3 w-3 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity" />
                    </button>
                  ))}
                </div>
              </div>
            </PopoverContent>
          )}
        </Popover>
        {inputValue && (
          <button
            type="button"
            aria-label="清空搜索"
            onClick={() => {
              setInputValue('');
              setQ('');
            }}
            className="absolute right-2.5 top-2.5 text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 transition-colors"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>

      <Button className="px-3 sm:px-4" onClick={() => handleSearch()}>
        搜索
      </Button>
      <Popover>
        <PopoverTrigger asChild>
          <Button variant="outline" size="icon" aria-label="搜索语法帮助" title="搜索语法帮助">
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
    </div>
  );
}
