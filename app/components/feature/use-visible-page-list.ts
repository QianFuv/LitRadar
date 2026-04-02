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

  const visiblePages =
    visiblePageState.listKey === listKey ? visiblePageState.count : 1;

  useEffect(() => {
    const scrollContainer = scrollContainerId
      ? document.getElementById(scrollContainerId)
      : null;

    if (scrollContainer) {
      scrollContainer.scrollTo({ top: 0 });
      return;
    }

    window.scrollTo({ top: 0 });
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
    [
      hasNextPage,
      isFetchingNextPage,
      loadedPages,
      onFetchNextPage,
      visiblePages,
    ],
  );

  const handleLoadMoreChange = useCallback(
    (inView: boolean) => {
      if (!inView) {
        return;
      }
      if (visiblePages < loadedPages) {
        setVisiblePageState((current) => {
          const currentCount =
            current.listKey === listKey ? current.count : 1;
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
    [
      hasNextPage,
      isFetchingNextPage,
      listKey,
      loadedPages,
      onFetchNextPage,
      visiblePages,
    ],
  );

  const { ref: prefetchRef } = useInView({
    threshold: 0,
    onChange: handlePrefetchChange,
  });

  const { ref: loadMoreRef } = useInView({
    threshold: 0,
    onChange: handleLoadMoreChange,
  });

  return {
    visiblePages,
    prefetchRef,
    loadMoreRef,
  };
}
