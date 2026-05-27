'use client';

/**
 * Reusable client-side React hooks for Paper Scanner.
 */

import { useCallback, useEffect, useMemo, useState, type RefObject } from 'react';
import { readSelectedDatabase, storeSelectedDatabase } from './client-api';

export interface SearchHistoryEntry {
  query: string;
  timestamp: number;
}

const SEARCH_HISTORY_KEY = 'paper_scanner_search_history';

/**
 * Read search history from local storage.
 *
 * @returns Array of search history entries.
 */
function readSearchHistory(): SearchHistoryEntry[] {
  if (typeof window === 'undefined') {
    return [];
  }
  const rawValue = window.localStorage.getItem(SEARCH_HISTORY_KEY);
  if (!rawValue) {
    return [];
  }
  try {
    const parsed = JSON.parse(rawValue) as SearchHistoryEntry[];
    return Array.isArray(parsed) ? parsed.filter((entry) => typeof entry.query === 'string') : [];
  } catch {
    return [];
  }
}

/**
 * Persist search history to local storage.
 *
 * @param entries - Search history entries.
 */
function storeSearchHistory(entries: SearchHistoryEntry[]): void {
  if (typeof window !== 'undefined') {
    window.localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify(entries.slice(0, 8)));
  }
}

/**
 * Helper to append a query to the history list.
 *
 * @param entries - Existing search history entries.
 * @param query - New query term.
 * @returns Updated search history entries.
 */
function addSearchHistoryEntry(entries: SearchHistoryEntry[], query: string): SearchHistoryEntry[] {
  const trimmedQuery = query.trim();
  if (!trimmedQuery) {
    return entries;
  }
  return [
    { query: trimmedQuery, timestamp: Date.now() },
    ...entries.filter((entry) => entry.query.toLowerCase() !== trimmedQuery.toLowerCase()),
  ].slice(0, 8);
}

/**
 * Format a relative search-history timestamp.
 *
 * @param timestamp - Entry timestamp.
 * @returns Short relative label.
 */
export function formatHistoryTime(timestamp: number): string {
  const elapsedDays = Math.floor((Date.now() - timestamp) / 86_400_000);
  if (elapsedDays <= 0) {
    return '今天';
  }
  if (elapsedDays === 1) {
    return '昨天';
  }
  return `${elapsedDays} 天前`;
}

/**
 * Hook to trigger a callback when clicking outside a ref or pressing Escape.
 *
 * @param ref - Element ref to check clicks against.
 * @param callback - Event handler to trigger when outside click occurs.
 * @param active - Optional trigger active state.
 */
export function useClickOutside(
  ref: RefObject<HTMLElement | null>,
  callback: () => void,
  active = true,
): void {
  useEffect(() => {
    if (!active) {
      return;
    }

    const handlePointerDown = (event: MouseEvent) => {
      if (event.target instanceof Node && ref.current && !ref.current.contains(event.target)) {
        callback();
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        callback();
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [active, callback, ref]);
}

/**
 * Hook to read, append, and clear search history entries.
 *
 * @returns History state and actions.
 */
export function useSearchHistory() {
  const [entries, setEntries] = useState<SearchHistoryEntry[]>([]);

  useEffect(() => {
    const handle = setTimeout(() => {
      setEntries(readSearchHistory());
    }, 0);
    return () => clearTimeout(handle);
  }, []);

  const addEntry = useCallback((query: string) => {
    setEntries((current) => {
      const nextEntries = addSearchHistoryEntry(current, query);
      storeSearchHistory(nextEntries);
      return nextEntries;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setEntries([]);
    if (typeof window !== 'undefined') {
      window.localStorage.removeItem(SEARCH_HISTORY_KEY);
    }
  }, []);

  return {
    entries,
    addEntry,
    clearHistory,
  };
}

/**
 * Hook to manage active index database coordinate across views.
 *
 * @param databases - List of available database files.
 * @param requestedDb - Override value typically requested via query params.
 * @returns Tuple of effective database name and setter.
 */
export function useActiveDatabase(
  databases: string[],
  requestedDb?: string,
): [string, (dbName: string) => void] {
  const [selectedDb, setSelectedDb] = useState<string>(() => readSelectedDatabase());

  const effectiveDb = useMemo(() => {
    if (requestedDb && databases.includes(requestedDb)) {
      return requestedDb;
    }
    if (databases.includes(selectedDb)) {
      return selectedDb;
    }
    return databases[0] || selectedDb;
  }, [databases, selectedDb, requestedDb]);

  const changeDb = useCallback((dbName: string) => {
    setSelectedDb(dbName);
    storeSelectedDatabase(dbName);
  }, []);

  useEffect(() => {
    if (effectiveDb && effectiveDb !== selectedDb) {
      const handle = setTimeout(() => {
        setSelectedDb(effectiveDb);
        storeSelectedDatabase(effectiveDb);
      }, 0);
      return () => clearTimeout(handle);
    }
  }, [effectiveDb, selectedDb]);

  return [effectiveDb, changeDb];
}
