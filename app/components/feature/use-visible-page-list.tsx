'use client';

import { useCallback, useEffect, useState } from 'react';
import { useInView } from 'react-intersection-observer';

type UseVisiblePageListOptions = {
  listKey: string;
  loadedPages: number;
  hasNextPage?: boolean;
  isFetchingNextPage?: boolean;
  onFetchNextPage?: () => void;
  scrollContainerId?: string;
};

/**
 * Reveal loaded pages incrementally and coordinate prefetch/load-more sentinels.
 *
 * @param options - List identity, pagination state, fetch callback, and scroll container.
 * @returns Visible page count and intersection-observer refs.
 */
export function useVisiblePageList({
  listKey,
  loadedPages,
  hasNextPage = false,
  isFetchingNextPage = false,
  onFetchNextPage,
  scrollContainerId,
}: UseVisiblePageListOptions) {
  const [visiblePageState, setVisiblePageState] = useState({
    listKey: '',
    count: 1,
  });

  const visiblePages = visiblePageState.listKey === listKey ? visiblePageState.count : 1;

  useEffect(() => {
    const scrollContainer = scrollContainerId ? document.getElementById(scrollContainerId) : null;

    if (scrollContainer) {
      scrollContainer.scrollTo({ behavior: 'auto', top: 0 });
      return;
    }

    window.scrollTo({ behavior: 'auto', top: 0 });
  }, [listKey, scrollContainerId]);

  const handlePrefetchChange = useCallback(
    (inView: boolean) => {
      if (!inView || !hasNextPage || isFetchingNextPage || !onFetchNextPage) {
        return;
      }
      if (loadedPages > visiblePages) {
        return;
      }
      onFetchNextPage();
    },
    [hasNextPage, isFetchingNextPage, loadedPages, onFetchNextPage, visiblePages],
  );

  const handleLoadMoreChange = useCallback(
    (inView: boolean) => {
      if (!inView) {
        return;
      }
      if (visiblePages < loadedPages) {
        setVisiblePageState((current) => {
          const currentCount = current.listKey === listKey ? current.count : 1;
          return {
            listKey,
            count: Math.min(currentCount + 1, loadedPages),
          };
        });
        return;
      }
      if (hasNextPage && !isFetchingNextPage && onFetchNextPage) {
        onFetchNextPage();
      }
    },
    [hasNextPage, isFetchingNextPage, listKey, loadedPages, onFetchNextPage, visiblePages],
  );

  const { ref: prefetchRef } = useInView({
    threshold: 0,
    onChange: handlePrefetchChange,
  });

  const { ref: loadMoreRef, inView: isLoadMoreInView } = useInView({
    threshold: 0,
    onChange: handleLoadMoreChange,
  });

  useEffect(() => {
    if (!isLoadMoreInView || visiblePages >= loadedPages) {
      return;
    }

    const animationFrame = window.requestAnimationFrame(() => {
      setVisiblePageState((current) => {
        const currentCount = current.listKey === listKey ? current.count : 1;
        const nextCount = Math.min(currentCount + 1, loadedPages);
        if (current.listKey === listKey && current.count === nextCount) {
          return current;
        }
        return { listKey, count: nextCount };
      });
    });

    return () => window.cancelAnimationFrame(animationFrame);
  }, [isLoadMoreInView, listKey, loadedPages, visiblePages]);

  return {
    visiblePages,
    prefetchRef,
    loadMoreRef,
  };
}
